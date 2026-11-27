//! Server groups (#37): each instance belongs to at most one group, like a folder.
//!
//! Groups live in the workspace file `server-groups.json`. They never touch an
//! instance's folder or registration. Older versions never read the file, so a
//! rollback keeps working and simply shows the list without groups.
//!
//! Edits name what they change (create, rename, delete, assign, collapse) and run
//! under one lock, so two edits never lose each other. An instance that leaves
//! management keeps its entry; restoring management shows it in its group again.
use super::*;
use std::collections::BTreeMap;

const GROUPS_FILE: &str = "server-groups.json";
const GROUPS_LOCK: &str = "server-groups.lock";
const MAX_FILE_BYTES: usize = 256 * 1024;
const MAX_EDIT_BYTES: usize = 64 * 1024;
const MAX_GROUPS: usize = 64;
const MAX_NAME_CHARS: usize = 40;
const MAX_MEMBERS: usize = 4096;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Group {
    id: String,
    name: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    collapsed: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Groups {
    /// Display order of the groups.
    #[serde(default)]
    groups: Vec<Group>,
    /// Instance name → group id. Instances without an entry are ungrouped.
    #[serde(default)]
    members: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    ungrouped_collapsed: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Edit {
    op: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    names: Option<Vec<String>>,
    #[serde(default)]
    collapsed: Option<bool>,
}

fn invalid() -> String {
    "InvalidServerGroups".into()
}

fn load(root: &Path) -> Result<(Groups, String, bool)> {
    let path = root.join(GROUPS_FILE);
    let file = match fs::symlink_metadata(&path) {
        Ok(meta) if !meta.is_file() => return Err("ServerGroupsUnexpectedFileType".into()),
        Ok(_) => {
            let resolved = retry_shared_file(|| fs::canonicalize(&path))
                .map_err(|error| file_error("resolve", &path, error))?;
            if resolved != path {
                return Err("PathRedirected".into());
            }
            retry_shared_file(|| File::open(&path)).map_err(|error| file_error("read", &path, error))?
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok((Groups::default(), content_revision::revision(b"")?, false));
        }
        Err(error) => return Err(file_error("read", &path, error)),
    };
    let mut bytes = Vec::new();
    file.take((MAX_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| file_error("read", &path, error))?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(format!("{}: {GROUPS_FILE}", invalid()));
    }
    let groups: Groups =
        serde_json::from_slice(&bytes).map_err(|error| format!("{}: {GROUPS_FILE}: {error}", invalid()))?;
    validate(&groups).map_err(|error| format!("{error}: {GROUPS_FILE}"))?;
    Ok((groups, content_revision::revision(&bytes)?, true))
}

/// A trimmed display name: 1 to 40 characters without control or direction characters.
fn group_name(raw: &str) -> Result<String> {
    let name = raw.trim();
    let count = name.chars().count();
    if count == 0 || count > MAX_NAME_CHARS {
        return Err("InvalidServerGroupName".into());
    }
    if name.chars().any(|character| {
        character.is_control()
            || matches!(character, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
    }) {
        return Err("InvalidServerGroupName".into());
    }
    Ok(name.to_owned())
}

fn group_id(value: &str) -> bool {
    value.len() <= 32
        && value.starts_with('g')
        && value.len() > 1
        && value.bytes().skip(1).all(|byte| byte.is_ascii_hexdigit())
}

fn same_name(left: &str, right: &str) -> bool {
    left.to_lowercase() == right.to_lowercase()
}

fn validate(value: &Groups) -> Result<()> {
    if value.groups.len() > MAX_GROUPS || value.members.len() > MAX_MEMBERS {
        return Err(invalid());
    }
    for (index, group) in value.groups.iter().enumerate() {
        if !group_id(&group.id) || group_name(&group.name)? != group.name {
            return Err(invalid());
        }
        if value.groups[..index]
            .iter()
            .any(|other| other.id == group.id || same_name(&other.name, &group.name))
        {
            return Err(invalid());
        }
    }
    for (name, id) in &value.members {
        if !instance_name(name) || !value.groups.iter().any(|group| &group.id == id) {
            return Err(invalid());
        }
    }
    Ok(())
}

fn response(groups: &Groups, revision: String) -> Value {
    json!({
        "groups": groups.groups,
        "members": groups.members,
        "ungroupedCollapsed": groups.ungrouped_collapsed,
        "revision": revision,
    })
}

fn new_id(existing: &[Group]) -> String {
    let mut stamp = clock();
    loop {
        let id = format!("g{:x}", stamp as u64);
        if !existing.iter().any(|group| group.id == id) {
            return id;
        }
        stamp += 1;
    }
}

fn find<'a>(groups: &'a mut Groups, id: &Option<String>) -> Result<&'a mut Group> {
    let id = id.as_deref().filter(|id| group_id(id)).ok_or_else(invalid)?;
    groups
        .groups
        .iter_mut()
        .find(|group| group.id == id)
        .ok_or_else(|| "ServerGroupNotFound".into())
}

fn unique_name(groups: &Groups, name: &str, except: Option<&str>) -> Result<()> {
    if groups
        .groups
        .iter()
        .any(|group| Some(group.id.as_str()) != except && same_name(&group.name, name))
    {
        return Err("ServerGroupNameTaken".into());
    }
    Ok(())
}

/// Applies one edit. `registered` says whether an instance name is managed.
fn apply(groups: &mut Groups, edit: Edit, registered: impl Fn(&str) -> Result<bool>) -> Result<Value> {
    match edit.op.as_str() {
        "create" => {
            if edit.id.is_some() || edit.names.is_some() || edit.collapsed.is_some() {
                return Err(invalid());
            }
            let name = group_name(edit.name.as_deref().ok_or_else(invalid)?)?;
            if groups.groups.len() >= MAX_GROUPS {
                return Err("TooManyServerGroups".into());
            }
            unique_name(groups, &name, None)?;
            let id = new_id(&groups.groups);
            groups.groups.push(Group { id: id.clone(), name, collapsed: false });
            Ok(json!(id))
        }
        "rename" => {
            if edit.names.is_some() || edit.collapsed.is_some() {
                return Err(invalid());
            }
            let name = group_name(edit.name.as_deref().ok_or_else(invalid)?)?;
            let id = find(groups, &edit.id)?.id.clone();
            unique_name(groups, &name, Some(&id))?;
            find(groups, &edit.id)?.name = name;
            Ok(Value::Null)
        }
        "delete" => {
            if edit.name.is_some() || edit.names.is_some() || edit.collapsed.is_some() {
                return Err(invalid());
            }
            let id = find(groups, &edit.id)?.id.clone();
            groups.groups.retain(|group| group.id != id);
            // Its servers become ungrouped; no server is removed.
            groups.members.retain(|_, group| *group != id);
            Ok(Value::Null)
        }
        "assign" => {
            if edit.name.is_some() || edit.collapsed.is_some() {
                return Err(invalid());
            }
            let names = edit.names.ok_or_else(invalid)?;
            if names.is_empty() || names.len() > MAX_MEMBERS {
                return Err(invalid());
            }
            // `id: null` moves the instances to "Ungrouped".
            let target = match &edit.id {
                None => None,
                Some(_) => Some(find(groups, &edit.id)?.id.clone()),
            };
            for name in &names {
                if !instance_name(name) || !registered(name)? {
                    return Err("InstanceExcludedOrInvalid".into());
                }
            }
            for name in names {
                match &target {
                    Some(id) => {
                        groups.members.insert(name, id.clone());
                    }
                    None => {
                        groups.members.remove(&name);
                    }
                }
            }
            if groups.members.len() > MAX_MEMBERS {
                return Err(invalid());
            }
            Ok(Value::Null)
        }
        "collapse" => {
            if edit.name.is_some() || edit.names.is_some() {
                return Err(invalid());
            }
            let collapsed = edit.collapsed.ok_or_else(invalid)?;
            match &edit.id {
                None => groups.ungrouped_collapsed = collapsed,
                Some(_) => find(groups, &edit.id)?.collapsed = collapsed,
            }
            Ok(Value::Null)
        }
        _ => Err(invalid()),
    }
}

fn store(root: &Path, raw: &str) -> Result<Value> {
    if raw.len() > MAX_EDIT_BYTES {
        return Err(invalid());
    }
    let edit: Edit = serde_json::from_str(raw).map_err(|_| invalid())?;
    let _lock = command_lock(root, GROUPS_LOCK, false)?;
    let (current, revision, exists) = load(root)?;
    let mut next = current.clone();
    let created = apply(&mut next, edit, |name| Ok(record_path(root, name).is_file()))?;
    validate(&next)?;
    if next == current {
        let mut result = response(&current, revision);
        result["created"] = created;
        return Ok(result);
    }
    let bytes = serde_json::to_vec_pretty(&next).map_err(fail)?;
    publish_json(&root.join(GROUPS_FILE), &bytes, exists, || {
        let (_, latest, present) = load(root)?;
        if latest != revision || present != exists {
            return Err("ServerGroupsConflict".into());
        }
        Ok(())
    })?;
    let (saved, revision, _) = load(root)?;
    let mut result = response(&saved, revision);
    result["created"] = created;
    Ok(result)
}

pub(super) fn dispatch(root: &Path, verb: &str, args: &[String]) -> Option<Result<Value>> {
    Some(match (verb, args) {
        ("server-groups", []) => load(root).map(|(groups, revision, _)| response(&groups, revision)),
        ("edit-server-groups", [raw]) => store(root, raw),
        ("server-groups" | "edit-server-groups", _) => Err(invalid()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let base = std::env::temp_dir().join(format!("ih-server-groups-{}-{}", std::process::id(), clock()));
            fs::create_dir_all(base.join("registry")).unwrap();
            let base = fs::canonicalize(base).unwrap();
            for name in ["alpha", "beta", "gamma"] {
                fs::write(base.join("registry").join(format!("{name}.json")), "{}").unwrap();
            }
            Self(base)
        }

        fn read(&self) -> Value {
            dispatch(&self.0, "server-groups", &[]).unwrap().unwrap()
        }

        fn edit(&self, edit: Value) -> Result<Value> {
            dispatch(&self.0, "edit-server-groups", &[edit.to_string()]).unwrap()
        }

        fn create(&self, name: &str) -> String {
            self.edit(json!({"op":"create","name":name})).unwrap()["created"].as_str().unwrap().to_owned()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn no_file_means_no_groups_and_reading_writes_nothing() {
        let f = Fixture::new();
        let read = f.read();
        assert_eq!(read["groups"], json!([]));
        assert_eq!(read["members"], json!({}));
        assert_eq!(read["ungroupedCollapsed"], false);
        assert!(!f.0.join(GROUPS_FILE).exists());
    }

    #[test]
    fn create_rename_assign_and_reload_keep_the_groups() {
        let f = Fixture::new();
        let lobby = f.create("  Lobby ");
        let games = f.create("Games");
        f.edit(json!({"op":"assign","id":lobby,"names":["alpha","beta"]})).unwrap();
        f.edit(json!({"op":"assign","id":games,"names":["beta"]})).unwrap();
        f.edit(json!({"op":"rename","id":games,"name":"Mini games"})).unwrap();
        f.edit(json!({"op":"collapse","id":lobby,"collapsed":true})).unwrap();
        f.edit(json!({"op":"collapse","id":null,"collapsed":true})).unwrap();
        // A new read, as after reopening the app, shows the same state.
        let read = f.read();
        assert_eq!(read["groups"], json!([{"id":lobby,"name":"Lobby","collapsed":true},{"id":games,"name":"Mini games"}]));
        // One group per server: assigning again moves it.
        assert_eq!(read["members"], json!({"alpha":lobby,"beta":games}));
        assert_eq!(read["ungroupedCollapsed"], true);
        f.edit(json!({"op":"assign","id":null,"names":["alpha"]})).unwrap();
        assert_eq!(f.read()["members"], json!({"beta":games}));
    }

    #[test]
    fn deleting_a_group_leaves_its_servers_ungrouped() {
        let f = Fixture::new();
        let lobby = f.create("Lobby");
        let games = f.create("Games");
        f.edit(json!({"op":"assign","id":lobby,"names":["alpha","beta"]})).unwrap();
        f.edit(json!({"op":"assign","id":games,"names":["gamma"]})).unwrap();
        f.edit(json!({"op":"delete","id":lobby})).unwrap();
        let read = f.read();
        assert_eq!(read["groups"], json!([{"id":games,"name":"Games"}]));
        assert_eq!(read["members"], json!({"gamma":games}));
        // The registrations are untouched.
        for name in ["alpha", "beta", "gamma"] {
            assert!(f.0.join("registry").join(format!("{name}.json")).is_file());
        }
    }

    #[test]
    fn invalid_edits_are_refused_and_keep_the_file() {
        let f = Fixture::new();
        let lobby = f.create("Lobby");
        let before = f.read();
        for (edit, reason) in [
            (json!({"op":"create","name":"lobby"}), "ServerGroupNameTaken"),
            (json!({"op":"create","name":"  "}), "InvalidServerGroupName"),
            (json!({"op":"create","name":"x".repeat(41)}), "InvalidServerGroupName"),
            (json!({"op":"create","name":"a\u{202e}b"}), "InvalidServerGroupName"),
            (json!({"op":"rename","id":"g1","name":"Other"}), "ServerGroupNotFound"),
            (json!({"op":"rename","id":"../x","name":"Other"}), "InvalidServerGroups"),
            (json!({"op":"assign","id":lobby,"names":["missing"]}), "InstanceExcludedOrInvalid"),
            (json!({"op":"assign","id":lobby,"names":["../alpha"]}), "InstanceExcludedOrInvalid"),
            (json!({"op":"assign","id":lobby,"names":[]}), "InvalidServerGroups"),
            (json!({"op":"assign","id":lobby,"names":["alpha"],"extra":1}), "InvalidServerGroups"),
            (json!({"op":"unknown"}), "InvalidServerGroups"),
        ] {
            assert_eq!(f.edit(edit.clone()).unwrap_err(), reason, "{edit}");
            assert_eq!(f.read(), before, "{edit}");
        }
    }

    #[test]
    fn renaming_to_the_same_name_with_other_case_is_allowed() {
        let f = Fixture::new();
        let lobby = f.create("Lobby");
        f.edit(json!({"op":"rename","id":lobby,"name":"LOBBY"})).unwrap();
        assert_eq!(f.read()["groups"][0]["name"], "LOBBY");
    }

    #[test]
    fn a_damaged_file_is_reported_and_not_replaced() {
        let f = Fixture::new();
        fs::write(f.0.join(GROUPS_FILE), r#"{"groups":[],"members":{"alpha":"g1"}}"#).unwrap();
        assert!(dispatch(&f.0, "server-groups", &[]).unwrap().unwrap_err().starts_with("InvalidServerGroups"));
        assert!(f.edit(json!({"op":"create","name":"Lobby"})).unwrap_err().starts_with("InvalidServerGroups"));
        assert_eq!(fs::read_to_string(f.0.join(GROUPS_FILE)).unwrap(), r#"{"groups":[],"members":{"alpha":"g1"}}"#);
    }
}
