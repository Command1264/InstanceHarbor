//! Settings profiles (#36): named sets of instance and world settings that the
//! user applies to several servers at once.
//!
//! The first version only applies once: applying copies the values into each
//! server, and the servers stay independent afterwards. Profiles live in the
//! workspace file `settings-profiles.json`, together with the profile a server
//! group (#37) remembers. Remembering never applies anything by itself.
//!
//! Applying reuses the existing writers, so their rules hold unchanged:
//! - Java, JVM, timeouts and automatic restart go through `save-instance-settings`;
//!   a running server keeps them as deferred settings until it stops.
//! - `server.properties` fields (and their YAML bindings) go through
//!   `save-world-settings`, one file at a time; a running server queues them.
//! - Game rules go through `world-live-queue-rules`; the next start applies them.
//!
//! Each apply is journaled in `history/settings-profile-applies/` and listed in
//! the activity page, one row per server.
use super::*;
use std::collections::BTreeMap;

const PROFILES_FILE: &str = "settings-profiles.json";
const PROFILES_LOCK: &str = "settings-profiles.lock";
const JOURNAL_FOLDER: &str = "settings-profile-applies";
const MAX_FILE_BYTES: usize = 256 * 1024;
const MAX_EDIT_BYTES: usize = 64 * 1024;
const MAX_PROFILES: usize = 64;
const MAX_NAME_CHARS: usize = 40;
const MAX_TARGETS: usize = 512;
/// Game rules a profile may carry, with the largest accepted integer
/// (`None` for true/false rules). The keys match `world_live::RULE_KEYS`.
const RULES: &[(&str, Option<u32>)] = &[
    ("gamerules.advance_time", None),
    ("gamerules.spawn_mobs", None),
    ("gamerules.command_block_output", None),
    ("gamerules.max_entity_cramming", Some(9999)),
    ("gamerules.allow_entering_nether_using_portals", None),
];

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Values {
    /// `""` selects Java automatically; otherwise the path to `java.exe`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    java: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    jvm: Option<Jvm>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timing: Option<LifecycleTiming>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auto_restart: Option<AutoRestart>,
    /// World field id (`serverProperties.<key>`) → value.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    server_properties: BTreeMap<String, String>,
    /// Game rule key (`gamerules.<key>`) → value.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    gamerules: BTreeMap<String, String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Profile {
    id: String,
    name: String,
    values: Values,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Profiles {
    #[serde(default)]
    profiles: Vec<Profile>,
    /// Server group id (#37) → profile id it remembers.
    #[serde(default)]
    group_profiles: BTreeMap<String, String>,
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
    values: Option<Values>,
    #[serde(default)]
    group: Option<String>,
}

fn invalid() -> String {
    "InvalidSettingsProfiles".into()
}

fn read_limited(path: &Path, limit: usize) -> Result<Option<Vec<u8>>> {
    let file = match fs::symlink_metadata(path) {
        Ok(meta) if !meta.is_file() => return Err("SettingsProfilesUnexpectedFileType".into()),
        Ok(_) => {
            let resolved =
                retry_shared_file(|| fs::canonicalize(path)).map_err(|error| file_error("resolve", path, error))?;
            if resolved != path {
                return Err("PathRedirected".into());
            }
            retry_shared_file(|| File::open(path)).map_err(|error| file_error("read", path, error))?
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(file_error("read", path, error)),
    };
    let mut bytes = Vec::new();
    file.take((limit + 1) as u64).read_to_end(&mut bytes).map_err(|error| file_error("read", path, error))?;
    if bytes.len() > limit {
        return Err(invalid());
    }
    Ok(Some(bytes))
}

fn load(root: &Path) -> Result<(Profiles, String, bool)> {
    let path = root.join(PROFILES_FILE);
    let Some(bytes) = read_limited(&path, MAX_FILE_BYTES)? else {
        return Ok((Profiles::default(), content_revision::revision(b"")?, false));
    };
    let profiles: Profiles =
        serde_json::from_slice(&bytes).map_err(|error| format!("{}: {PROFILES_FILE}: {error}", invalid()))?;
    validate(&profiles).map_err(|error| format!("{error}: {PROFILES_FILE}"))?;
    Ok((profiles, content_revision::revision(&bytes)?, true))
}

fn display_name(raw: &str, error: &str) -> Result<String> {
    let name = raw.trim();
    let count = name.chars().count();
    if count == 0
        || count > MAX_NAME_CHARS
        || name.chars().any(|character| {
            character.is_control()
                || matches!(character, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
    {
        return Err(error.into());
    }
    Ok(name.to_owned())
}

fn prefixed_id(value: &str, prefix: char) -> bool {
    value.len() > 1 && value.len() <= 32 && value.starts_with(prefix) && value.bytes().skip(1).all(|byte| byte.is_ascii_hexdigit())
}

fn rule_value(key: &str, value: &str) -> bool {
    match RULES.iter().find(|(rule, _)| *rule == key) {
        Some((_, None)) => matches!(value, "true" | "false"),
        Some((_, Some(max))) => value.parse::<u32>().is_ok_and(|number| number <= *max),
        None => false,
    }
}

fn validate_values(values: &Values) -> Result<()> {
    if let Some(java) = &values.java {
        if !java.is_empty()
            && (java.len() > 1024 || java.chars().any(char::is_control) || !Path::new(java).is_absolute())
        {
            return Err("InvalidSettingsProfileJava".into());
        }
    }
    if let Some(jvm) = &values.jvm {
        jvm.arguments()?;
    }
    if let Some(timing) = &values.timing {
        timing.validate()?;
    }
    if values.auto_restart.as_ref().is_some_and(|policy| !policy.valid()) {
        return Err("InvalidAutoRestartPolicy".into());
    }
    for (key, value) in &values.server_properties {
        if !policy_settings::profile_world_field(key) || value.len() > 512 || value.chars().any(char::is_control) {
            return Err(format!("InvalidSettingsProfileField: {key}"));
        }
    }
    for (key, value) in &values.gamerules {
        if !rule_value(key, value) {
            return Err(format!("InvalidSettingsProfileField: {key}"));
        }
    }
    Ok(())
}

fn validate(value: &Profiles) -> Result<()> {
    if value.profiles.len() > MAX_PROFILES || value.group_profiles.len() > 4096 {
        return Err(invalid());
    }
    for (index, profile) in value.profiles.iter().enumerate() {
        if !prefixed_id(&profile.id, 'p') || display_name(&profile.name, "InvalidSettingsProfileName")? != profile.name {
            return Err(invalid());
        }
        if value.profiles[..index].iter().any(|other| {
            other.id == profile.id || other.name.to_lowercase() == profile.name.to_lowercase()
        }) {
            return Err(invalid());
        }
        validate_values(&profile.values)?;
    }
    for (group, profile) in &value.group_profiles {
        if !prefixed_id(group, 'g') || !value.profiles.iter().any(|candidate| &candidate.id == profile) {
            return Err(invalid());
        }
    }
    Ok(())
}

fn response(profiles: &Profiles, revision: String) -> Value {
    json!({"profiles":profiles.profiles,"groupProfiles":profiles.group_profiles,"revision":revision})
}

fn edit(root: &Path, raw: &str) -> Result<Value> {
    if raw.len() > MAX_EDIT_BYTES {
        return Err(invalid());
    }
    let edit: Edit = serde_json::from_str(raw).map_err(|error| format!("{}: {error}", invalid()))?;
    let _lock = command_lock(root, PROFILES_LOCK, false)?;
    let (mut profiles, revision, exists) = load(root)?;
    let mut saved_id = Value::Null;
    match edit.op.as_str() {
        "save" => {
            if edit.group.is_some() {
                return Err(invalid());
            }
            let name = display_name(edit.name.as_deref().ok_or_else(invalid)?, "InvalidSettingsProfileName")?;
            let values = edit.values.ok_or_else(invalid)?;
            validate_values(&values)?;
            if profiles.profiles.iter().any(|profile| {
                Some(&profile.id) != edit.id.as_ref() && profile.name.to_lowercase() == name.to_lowercase()
            }) {
                return Err("SettingsProfileNameTaken".into());
            }
            match &edit.id {
                Some(id) => {
                    let profile = profiles
                        .profiles
                        .iter_mut()
                        .find(|profile| &profile.id == id)
                        .ok_or("SettingsProfileNotFound")?;
                    profile.name = name;
                    profile.values = values;
                    saved_id = json!(id);
                }
                None => {
                    if profiles.profiles.len() >= MAX_PROFILES {
                        return Err("TooManySettingsProfiles".into());
                    }
                    let mut stamp = clock() as u64;
                    while profiles.profiles.iter().any(|profile| profile.id == format!("p{stamp:x}")) {
                        stamp += 1;
                    }
                    let id = format!("p{stamp:x}");
                    profiles.profiles.push(Profile { id: id.clone(), name, values });
                    saved_id = json!(id);
                }
            }
        }
        "delete" => {
            if edit.name.is_some() || edit.values.is_some() || edit.group.is_some() {
                return Err(invalid());
            }
            let id = edit.id.ok_or_else(invalid)?;
            let before = profiles.profiles.len();
            profiles.profiles.retain(|profile| profile.id != id);
            if profiles.profiles.len() == before {
                return Err("SettingsProfileNotFound".into());
            }
            // Groups that remembered it simply remember nothing.
            profiles.group_profiles.retain(|_, profile| *profile != id);
        }
        "link" => {
            if edit.name.is_some() || edit.values.is_some() {
                return Err(invalid());
            }
            let group = edit.group.filter(|group| prefixed_id(group, 'g')).ok_or_else(invalid)?;
            match edit.id {
                Some(id) => {
                    if !profiles.profiles.iter().any(|profile| profile.id == id) {
                        return Err("SettingsProfileNotFound".into());
                    }
                    profiles.group_profiles.insert(group, id);
                }
                None => {
                    profiles.group_profiles.remove(&group);
                }
            }
        }
        _ => return Err(invalid()),
    }
    validate(&profiles)?;
    let bytes = serde_json::to_vec_pretty(&profiles).map_err(fail)?;
    publish_json(&root.join(PROFILES_FILE), &bytes, exists, || {
        let (_, latest, present) = load(root)?;
        if latest != revision || present != exists {
            return Err("SettingsProfilesConflict".into());
        }
        Ok(())
    })?;
    let (saved, revision, _) = load(root)?;
    let mut result = response(&saved, revision);
    result["saved"] = saved_id;
    Ok(result)
}

fn profile(root: &Path, id: &str) -> Result<Profile> {
    load(root)?
        .0
        .profiles
        .into_iter()
        .find(|profile| profile.id == id)
        .ok_or_else(|| "SettingsProfileNotFound".into())
}

fn targets(raw: &str) -> Result<Vec<String>> {
    let names: Vec<String> = serde_json::from_str(raw).map_err(|_| "InvalidSettingsProfileTargets")?;
    let mut seen = std::collections::HashSet::new();
    if names.is_empty() || names.len() > MAX_TARGETS || names.iter().any(|name| !instance_name(name) || !seen.insert(name)) {
        return Err("InvalidSettingsProfileTargets".into());
    }
    Ok(names)
}

/// The instance part of a profile as a `save-instance-settings` input, and the
/// rows that differ from `current` (an `instance-settings` reply).
fn instance_plan(values: &Values, current: &Value) -> (Option<Value>, Vec<Value>, Vec<Value>) {
    let mut input = json!({"java":current["java"],"port":current["port"],"revision":current["revision"]});
    let mut changes = Vec::new();
    let mut skipped = Vec::new();
    if let Some(java) = &values.java {
        let target = if java.is_empty() { String::new() } else { path_format::display(Path::new(java)) };
        if java.is_empty() && current["canSelectJavaAutomatically"] != true {
            skipped.push(json!({"key":"java","reason":"javaAutomaticUnavailable"}));
        } else if current["java"].as_str() != Some(target.as_str()) {
            changes.push(json!({"key":"java","before":current["java"],"after":target}));
            input["java"] = json!(java);
        }
    }
    for (key, target) in [
        ("jvm", values.jvm.as_ref().map(|value| json!(value))),
        ("timing", values.timing.as_ref().map(|value| json!(value))),
        ("autoRestart", values.auto_restart.as_ref().map(|value| json!(value))),
    ] {
        if let Some(target) = target {
            if current[key] != target {
                changes.push(json!({"key":key,"before":current[key],"after":target}));
                input[key] = target;
            }
        }
    }
    ((!changes.is_empty()).then_some(input), changes, skipped)
}

/// Game rules: the version's supported keys come from `world-live-values`.
fn rule_plan(root: &Path, name: &str, values: &Values) -> (BTreeMap<String, String>, Vec<Value>, Vec<Value>) {
    let mut send = BTreeMap::new();
    let mut changes = Vec::new();
    let mut skipped = Vec::new();
    if values.gamerules.is_empty() {
        return (send, changes, skipped);
    }
    let current = match world_live::dispatch(root, "world-live-values", &[name.to_owned()]) {
        Some(Ok(current)) => current,
        _ => {
            skipped.extend(values.gamerules.keys().map(|key| json!({"key":key,"reason":"rulesUnconfirmed"})));
            return (send, changes, skipped);
        }
    };
    for (key, value) in &values.gamerules {
        let Some(row) = current.get(key) else {
            skipped.push(json!({"key":key,"reason":"noFieldForVersion"}));
            continue;
        };
        // A rule already queued with the same value needs nothing new.
        let before = row["target"].as_str().or(row["value"].as_str());
        if before != Some(value.as_str()) {
            changes.push(json!({"key":key,"before":before,"after":value,"deferred":true}));
            send.insert(key.clone(), value.clone());
        }
    }
    (send, changes, skipped)
}

fn preview_item(root: &Path, name: &str, values: &Values) -> Value {
    let plan = || -> Result<Value> {
        let current = presentation::dispatch(root, "instance-settings", &[name.to_owned()]).ok_or("InvalidPresentationArguments")??;
        let state = status(root, &presentation::active_instance(root, name)?)?["state"].clone();
        let (_, mut changes, mut skipped) = instance_plan(values, &current);
        if !values.server_properties.is_empty() {
            let (world, world_skipped) = policy_settings::profile_world_preview(root, name, &values.server_properties)?;
            changes.extend(world);
            skipped.extend(world_skipped);
        }
        let (_, rules, rules_skipped) = rule_plan(root, name, values);
        changes.extend(rules);
        skipped.extend(rules_skipped);
        Ok(json!({"name":name,"state":state,"changes":changes,"skipped":skipped}))
    };
    plan().unwrap_or_else(|error| json!({"name":name,"error":error,"changes":[],"skipped":[]}))
}

fn preview(root: &Path, id: &str, raw: &str) -> Result<Value> {
    let profile = profile(root, id)?;
    let items: Vec<Value> = targets(raw)?.iter().map(|name| preview_item(root, name, &profile.values)).collect();
    // The token binds a confirmation to exactly this preview.
    let token = content_revision::revision(&serde_json::to_vec(&json!({"profile":profile,"items":items})).map_err(fail)?)?;
    Ok(json!({"profile":{"id":profile.id,"name":profile.name},"items":items,"token":token}))
}

fn journal_directory(root: &Path, create: bool) -> Result<Option<PathBuf>> {
    let history = child_directory(root, "history")?;
    let directory = history.join(JOURNAL_FOLDER);
    match fs::symlink_metadata(&directory) {
        Ok(meta) if meta.is_dir() => {
            no_redirect(&directory)?;
            Ok(Some(directory))
        }
        Ok(_) => Err("SettingsProfileJournalUnavailable".into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound && create => {
            fs::create_dir(&directory).map_err(|error| file_error("create", &directory, error))?;
            Ok(Some(directory))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(file_error("inspect", &directory, error)),
    }
}

fn write_journal(directory: &Path, request: &str, journal: &Value, replace: bool) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(journal).map_err(fail)?;
    publish_json(&directory.join(format!("{request}.json")), &bytes, replace, || Ok(()))
}

fn apply_item(root: &Path, request: &str, index: usize, item: &Value, values: &Values) -> Value {
    let name = item["name"].as_str().unwrap_or_default().to_owned();
    let mut result = json!({"name":name,"skipped":item["skipped"],"applied":[],"deferred":[]});
    if let Some(error) = item.get("error") {
        result["state"] = json!("Failed");
        result["error"] = error.clone();
        return result;
    }
    let keys = |filter: &dyn Fn(&Value) -> bool| -> Vec<Value> {
        item["changes"].as_array().into_iter().flatten().filter(|change| filter(change)).map(|change| change["key"].clone()).collect()
    };
    if item["changes"].as_array().is_none_or(Vec::is_empty) {
        result["state"] = json!("Skipped");
        return result;
    }
    let mut deferred = false;
    let outcome = (|| -> Result<()> {
        {
            // The same guards `save-instance-settings` and `save-world-settings` pass.
            let _writer = command_lock(root, &format!("instance-{name}.lock"), false)?;
            creation::assert_idle(root, &name)?;
            agent_rebuild::assert_idle(root, &name)?;
            core_maintenance::assert_start_allowed(&instance_path(root, &name)?)?;
            let current = presentation::dispatch(root, "instance-settings", std::slice::from_ref(&name)).ok_or("InvalidPresentationArguments")??;
            if let (Some(input), _, _) = instance_plan(values, &current) {
                let saved = presentation::dispatch(root, "save-instance-settings", &[name.clone(), input.to_string()])
                    .ok_or("InvalidPresentationArguments")??;
                deferred |= saved["queued"] == true;
            }
            if !values.server_properties.is_empty() {
                let saved = policy_settings::profile_world_apply(root, &name, &values.server_properties)?;
                deferred |= saved["queued"] == true;
            }
        }
        let (rules, _, _) = rule_plan(root, &name, values);
        if !rules.is_empty() {
            let rule_request = format!("{request}-r{index}");
            world_live::dispatch(root, "world-live-queue-rules", &[name.clone(), rule_request, json!(rules).to_string()])
                .ok_or("InvalidWorldRequest")??;
        }
        Ok(())
    })();
    let rule = |change: &Value| change["deferred"] == true;
    match outcome {
        Ok(()) if deferred => {
            result["state"] = json!("Completed");
            result["deferred"] = json!(keys(&|_| true));
        }
        Ok(()) => {
            result["state"] = json!("Completed");
            result["applied"] = json!(keys(&|change| !rule(change)));
            result["deferred"] = json!(keys(&rule));
        }
        Err(error) => {
            result["state"] = json!("Failed");
            result["error"] = json!(error);
        }
    }
    result
}

fn apply(root: &Path, request: &str, id: &str, raw: &str, confirmation: &str) -> Result<Value> {
    if !token(request) || request.len() > 48 {
        return Err("InvalidSettingsProfileRequest".into());
    }
    let directory = journal_directory(root, true)?.ok_or("SettingsProfileJournalUnavailable")?;
    let path = directory.join(format!("{request}.json"));
    if path.exists() {
        // The same request again returns what it did the first time.
        let previous = read_limited(&path, MAX_FILE_BYTES)?.ok_or("SettingsProfileJournalUnavailable")?;
        return serde_json::from_slice(&previous).map_err(|_| "SettingsProfileJournalUnavailable".into());
    }
    let current = preview(root, id, raw)?;
    if current["token"].as_str() != Some(confirmation) {
        return Err("SettingsProfilePreviewChanged".into());
    }
    let values = profile(root, id)?.values;
    let created_at = (clock() / 1_000_000) as u64;
    let mut journal = json!({"request":request,"createdAt":created_at,"profile":current["profile"],"state":"Running","items":[]});
    write_journal(&directory, request, &journal, false)?;
    let items: Vec<Value> = current["items"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(index, item)| apply_item(root, request, index, item, &values))
        .collect();
    journal["items"] = json!(items);
    journal["state"] = json!(if items.iter().any(|item| item["state"] == "Failed") { "CompletedWithIssues" } else { "Completed" });
    write_journal(&directory, request, &journal, true)?;
    Ok(journal)
}

/// One activity row per server of each apply.
pub(super) fn activity(root: &Path) -> Result<Vec<Value>> {
    let Some(directory) = journal_directory(root, false)? else {
        return Ok(Vec::new());
    };
    let mut rows = Vec::new();
    for entry in fs::read_dir(&directory).map_err(fail)? {
        let path = entry.map_err(fail)?.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let Some(bytes) = read_limited(&path, MAX_FILE_BYTES)? else { continue };
        let journal: Value = serde_json::from_slice(&bytes).map_err(|_| "SettingsProfileJournalUnavailable")?;
        let request = journal["request"].as_str().filter(|request| token(request)).ok_or("SettingsProfileJournalUnavailable")?;
        let running = journal["state"] == "Running";
        for (index, item) in journal["items"].as_array().into_iter().flatten().enumerate() {
            let Some(name) = item["name"].as_str().filter(|name| instance_name(name)) else { continue };
            if !admitted(root, name)? {
                continue;
            }
            rows.push(json!({
                "id":format!("{request}:{index}"),"action":"settings-profile-apply","instance":name,
                "createdAt":journal["createdAt"],"state":if running { json!("Unconfirmed") } else { item["state"].clone() },
                "profile":journal["profile"]["name"],"applied":item["applied"],"deferred":item["deferred"],
                "skipped":item["skipped"],"error":item.get("error"),
            }));
        }
    }
    Ok(rows)
}

pub(super) fn dispatch(root: &Path, verb: &str, args: &[String]) -> Option<Result<Value>> {
    Some(match (verb, args) {
        ("settings-profiles", []) => load(root).map(|(profiles, revision, _)| response(&profiles, revision)),
        ("edit-settings-profiles", [raw]) => edit(root, raw),
        ("settings-profile-preview", [id, names]) => preview(root, id, names),
        ("settings-profile-apply", [request, id, names, token]) => apply(root, request, id, names, token),
        ("settings-profiles" | "edit-settings-profiles" | "settings-profile-preview" | "settings-profile-apply", _) => {
            Err(invalid())
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let base = std::env::temp_dir().join(format!("ih-settings-profiles-{}-{}", std::process::id(), clock()));
            fs::create_dir_all(base.join("history")).unwrap();
            Self(fs::canonicalize(base).unwrap())
        }

        fn edit(&self, edit: Value) -> Result<Value> {
            dispatch(&self.0, "edit-settings-profiles", &[edit.to_string()]).unwrap()
        }

        fn read(&self) -> Value {
            dispatch(&self.0, "settings-profiles", &[]).unwrap().unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn sample_values() -> Value {
        json!({
            "java":"",
            "jvm":{"minMib":1024,"maxMib":2048,"gc":"g1"},
            "timing":{"stopSeconds":60,"readySeconds":120,"restartDelayMs":500},
            "autoRestart":{"onCrash":true,"onUnexpectedExit":false,"delaySeconds":10,"maxAttempts":3},
            "serverProperties":{"serverProperties.motd":"Hello","serverProperties.max-players":"30"},
            "gamerules":{"gamerules.advance_time":"false","gamerules.max_entity_cramming":"40"},
        })
    }

    #[test]
    fn profiles_save_reload_rename_link_and_delete() {
        let f = Fixture::new();
        assert_eq!(f.read()["profiles"], json!([]));
        let saved = f.edit(json!({"op":"save","name":" Lobby ","values":sample_values()})).unwrap();
        let id = saved["saved"].as_str().unwrap().to_owned();
        let read = f.read();
        assert_eq!(read["profiles"][0]["name"], "Lobby");
        assert_eq!(read["profiles"][0]["values"], sample_values());
        f.edit(json!({"op":"save","id":id,"name":"Lobby 2","values":{"java":"C:/Java/bin/java.exe"}})).unwrap();
        assert_eq!(f.read()["profiles"][0]["values"], json!({"java":"C:/Java/bin/java.exe"}));
        f.edit(json!({"op":"link","group":"g1a","id":id})).unwrap();
        assert_eq!(f.read()["groupProfiles"], json!({"g1a":id}));
        // Deleting the profile also forgets the group's choice.
        f.edit(json!({"op":"delete","id":id})).unwrap();
        assert_eq!(f.read()["profiles"], json!([]));
        assert_eq!(f.read()["groupProfiles"], json!({}));
    }

    #[test]
    fn invalid_profiles_are_refused_and_keep_the_file() {
        let f = Fixture::new();
        f.edit(json!({"op":"save","name":"Lobby","values":{}})).unwrap();
        let before = f.read();
        for (edit, reason) in [
            (json!({"op":"save","name":"LOBBY","values":{}}), "SettingsProfileNameTaken"),
            (json!({"op":"save","name":" ","values":{}}), "InvalidSettingsProfileName"),
            (json!({"op":"save","name":"x","values":{"java":"relative\\java.exe"}}), "InvalidSettingsProfileJava"),
            (json!({"op":"save","name":"x","values":{"timing":{"stopSeconds":1}}}), "InvalidLifecycleTiming"),
            (json!({"op":"save","name":"x","values":{"autoRestart":{"onCrash":true,"onUnexpectedExit":false,"delaySeconds":1,"maxAttempts":3}}}), "InvalidAutoRestartPolicy"),
            (json!({"op":"save","name":"x","values":{"serverProperties":{"serverProperties.resource-pack":"x"}}}), "InvalidSettingsProfileField: serverProperties.resource-pack"),
            (json!({"op":"save","name":"x","values":{"serverProperties":{"spigot.mob-spawn-range":"4"}}}), "InvalidSettingsProfileField: spigot.mob-spawn-range"),
            (json!({"op":"save","name":"x","values":{"gamerules":{"gamerules.advance_time":"maybe"}}}), "InvalidSettingsProfileField: gamerules.advance_time"),
            (json!({"op":"save","name":"x","values":{"gamerules":{"gamerules.max_entity_cramming":"10000"}}}), "InvalidSettingsProfileField: gamerules.max_entity_cramming"),
            (json!({"op":"link","group":"../g","id":"p1"}), "InvalidSettingsProfiles"),
            (json!({"op":"link","group":"g1","id":"p1"}), "SettingsProfileNotFound"),
            (json!({"op":"delete","id":"p1"}), "SettingsProfileNotFound"),
        ] {
            assert_eq!(f.edit(edit.clone()).unwrap_err(), reason, "{edit}");
            assert_eq!(f.read(), before, "{edit}");
        }
        assert!(f.edit(json!({"op":"save","name":"x","values":{"other":1}})).unwrap_err().starts_with("InvalidSettingsProfiles"));
    }

    #[test]
    fn instance_plan_lists_only_differences_and_skips_automatic_java_when_unavailable() {
        let values: Values = serde_json::from_value(sample_values()).unwrap();
        let current = json!({"java":"C:/Java/bin/java.exe","canSelectJavaAutomatically":false,"port":25565,"revision":"r1",
            "jvm":{"minMib":1024,"maxMib":2048,"gc":"g1"},"timing":null,"autoRestart":null});
        let (input, changes, skipped) = instance_plan(&values, &current);
        let keys: Vec<_> = changes.iter().map(|change| change["key"].as_str().unwrap()).collect();
        assert_eq!(keys, ["timing", "autoRestart"]);
        assert_eq!(skipped, [json!({"key":"java","reason":"javaAutomaticUnavailable"})]);
        let input = input.unwrap();
        // The port and revision always come from the current settings; Java stays as it is.
        assert_eq!((input["port"].clone(), input["revision"].clone(), input["java"].clone()), (json!(25565), json!("r1"), json!("C:/Java/bin/java.exe")));
        assert!(input.get("jvm").is_none());

        let mut automatic = current.clone();
        automatic["canSelectJavaAutomatically"] = json!(true);
        let (input, changes, skipped) = instance_plan(&values, &automatic);
        assert_eq!(changes[0], json!({"key":"java","before":"C:/Java/bin/java.exe","after":""}));
        assert!(skipped.is_empty());
        assert_eq!(input.unwrap()["java"], "");

        let same = json!({"java":"","canSelectJavaAutomatically":true,"port":1,"revision":"r","jvm":values.jvm.as_ref().map(|v| json!(v)),
            "timing":json!(values.timing.as_ref().unwrap()),"autoRestart":json!(values.auto_restart.as_ref().unwrap())});
        let (input, changes, _) = instance_plan(&values, &same);
        assert!(input.is_none() && changes.is_empty());
    }

    #[test]
    fn targets_must_be_distinct_valid_names() {
        assert_eq!(targets(r#"["a","b"]"#).unwrap(), ["a", "b"]);
        for raw in [r#"[]"#, r#"["a","a"]"#, r#"["../a"]"#, "x"] {
            assert_eq!(targets(raw).unwrap_err(), "InvalidSettingsProfileTargets", "{raw}");
        }
    }
}
