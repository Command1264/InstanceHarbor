//! The default parent folder that "Create instance" fills in.
//!
//! The setting lives in its own file. Older versions never read it, so a
//! rollback keeps working and simply falls back to the workspace's `instances`.
use super::*;

const SETTINGS_FILE: &str = "creation-defaults.json";
const MAX_SETTINGS_BYTES: usize = 8192;

#[derive(Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Settings {
    /// Absent: the workspace's `instances` folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Edit {
    /// `null` or blank clears the setting.
    parent: Option<String>,
    revision: String,
}

fn load(root: &Path) -> Result<(Settings, String, bool)> {
    let path = root.join(SETTINGS_FILE);
    let file = match fs::symlink_metadata(&path) {
        Ok(_) => {
            let resolved = retry_shared_file(|| fs::canonicalize(&path))
                .map_err(|error| file_error("resolve", &path, error))?;
            if resolved != path {
                return Err("PathRedirected".into());
            }
            retry_shared_file(|| File::open(&path)).map_err(|error| file_error("read", &path, error))?
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok((Settings::default(), content_revision::revision(b"")?, false));
        }
        Err(error) => return Err(file_error("read", &path, error)),
    };
    let mut bytes = Vec::new();
    file.take((MAX_SETTINGS_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| file_error("read", &path, error))?;
    if bytes.len() > MAX_SETTINGS_BYTES {
        return Err("InvalidCreationDefaults".into());
    }
    let settings = serde_json::from_slice(&bytes).map_err(|_| "InvalidCreationDefaults: creation-defaults.json")?;
    Ok((settings, content_revision::revision(&bytes)?, true))
}

fn response(settings: &Settings, revision: String) -> Value {
    json!({"parent":settings.parent,"revision":revision})
}

/// The same folders management folders accept: the workspace's `instances` or
/// a folder below it, or a local folder outside the workspace. The workspace's
/// other folders, folders that contain it, drive roots and UNC paths are refused.
fn checked_parent(root: &Path, raw: &str) -> Result<String> {
    if raw.len() > 4096 || raw.chars().any(char::is_control) {
        return Err("InvalidScanPath".into());
    }
    Ok(path_format::display(&scan_policy::managed_path(root, raw)?))
}

fn store(root: &Path, raw: &str) -> Result<Value> {
    if raw.len() > MAX_SETTINGS_BYTES {
        return Err("InvalidCreationDefaults".into());
    }
    let edit: Edit = serde_json::from_str(raw).map_err(|_| "InvalidCreationDefaults")?;
    let _lock = command_lock(root, "creation-defaults.lock", false)?;
    let (current, revision, exists) = load(root)?;
    if edit.revision != revision {
        return Err("CreationDefaultsConflict".into());
    }
    let parent = match edit.parent.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(raw) => Some(checked_parent(root, raw)?),
    };
    let next = Settings { parent };
    if next == current {
        return Ok(response(&current, revision));
    }
    let bytes = serde_json::to_vec_pretty(&next).map_err(fail)?;
    publish_json(&root.join(SETTINGS_FILE), &bytes, exists, || {
        let (_, latest, present) = load(root)?;
        if latest != revision || present != exists {
            return Err("CreationDefaultsConflict".into());
        }
        Ok(())
    })?;
    let (saved, revision, _) = load(root)?;
    Ok(response(&saved, revision))
}

pub(super) fn dispatch(root: &Path, verb: &str, args: &[String]) -> Option<Result<Value>> {
    Some(match (verb, args) {
        ("creation-defaults", []) => load(root).map(|(settings, revision, _)| response(&settings, revision)),
        ("save-creation-defaults", [raw]) => store(root, raw),
        ("creation-defaults" | "save-creation-defaults", _) => Err("InvalidCreationDefaults".into()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf, PathBuf);

    impl Fixture {
        fn new() -> Self {
            let base = std::env::temp_dir().join(format!("ih-creation-defaults-{}-{}", std::process::id(), clock()));
            fs::create_dir(&base).unwrap();
            let base = fs::canonicalize(base).unwrap();
            let root = base.join("workspace");
            for folder in ["", "instances", "instances/group", "registry", "staging"] {
                fs::create_dir(root.join(folder)).unwrap();
            }
            fs::create_dir(base.join("servers")).unwrap();
            Self(root, base)
        }

        fn read(&self) -> Value {
            dispatch(&self.0, "creation-defaults", &[]).unwrap().unwrap()
        }

        fn save(&self, parent: Value, revision: &Value) -> Result<Value> {
            let raw = json!({"parent":parent,"revision":revision}).to_string();
            dispatch(&self.0, "save-creation-defaults", &[raw]).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.1);
        }
    }

    #[test]
    fn unset_defaults_to_the_workspace_instances_folder_without_writing() {
        let f = Fixture::new();
        let initial = f.read();
        assert_eq!(initial["parent"], Value::Null);
        assert!(!f.0.join(SETTINGS_FILE).exists());
        // Saving the same empty value keeps the file absent.
        let same = f.save(Value::Null, &initial["revision"]).unwrap();
        assert_eq!(same, initial);
        assert!(!f.0.join(SETTINGS_FILE).exists());
    }

    #[test]
    fn accepts_folders_management_folders_accept_and_clears_back_to_default() {
        let f = Fixture::new();
        let outside = f.1.join("servers");
        let saved = f.save(json!(path_format::display(&outside)), &f.read()["revision"]).unwrap();
        assert_eq!(saved["parent"], path_format::display(&outside));
        assert_eq!(f.read(), saved);

        let inside = f.save(json!(path_format::display(&f.0.join("instances/group"))), &saved["revision"]).unwrap();
        assert_eq!(inside["parent"], path_format::display(&f.0.join("instances").join("group")));
        let relative = f.save(json!("instances"), &inside["revision"]).unwrap();
        assert_eq!(relative["parent"], path_format::display(&f.0.join("instances")));

        let cleared = f.save(json!("  "), &relative["revision"]).unwrap();
        assert_eq!(cleared["parent"], Value::Null);
        assert_eq!(fs::read_to_string(f.0.join(SETTINGS_FILE)).unwrap(), "{}");
    }

    #[test]
    fn refused_folders_keep_the_saved_setting() {
        let f = Fixture::new();
        let outside = path_format::display(&f.1.join("servers"));
        let saved = f.save(json!(outside), &f.read()["revision"]).unwrap();
        let drive = format!("{}\\", &outside[..2]);
        for (raw, reason) in [
            (path_format::display(&f.0.join("registry")), "BatchImportDirectoryOutsideScope"),
            (path_format::display(&f.0.join("staging")), "BatchImportDirectoryOutsideScope"),
            (path_format::display(&f.0), "BatchImportDirectoryOutsideScope"),
            (path_format::display(&f.1), "BatchImportDirectoryOutsideScope"),
            (drive, "BatchImportDirectoryOutsideScope"),
            (r"\\server\share\servers".into(), "InvalidScanPath"),
            ("relative".into(), "InvalidScanPath"),
            (path_format::display(&f.1.join("missing")), "ScanDirectoryUnavailable"),
        ] {
            assert_eq!(f.save(json!(raw), &saved["revision"]).unwrap_err(), reason, "{raw}");
            assert_eq!(f.read(), saved, "{raw}");
        }
    }

    #[test]
    fn stale_edits_and_unknown_fields_are_refused() {
        let f = Fixture::new();
        let initial = f.read();
        let saved = f.save(json!(path_format::display(&f.1.join("servers"))), &initial["revision"]).unwrap();
        assert_eq!(f.save(Value::Null, &initial["revision"]).unwrap_err(), "CreationDefaultsConflict");
        assert_eq!(f.read(), saved);
        let extra = json!({"parent":null,"revision":saved["revision"],"other":1}).to_string();
        assert_eq!(dispatch(&f.0, "save-creation-defaults", &[extra]).unwrap().unwrap_err(), "InvalidCreationDefaults");
    }
}
