use super::*;

const MAX_SETTINGS_BYTES: usize = 8192;
const SETTINGS_FILE: &str = "global-java.json";

#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Mode {
    #[default]
    Automatic,
    Manual,
}

#[derive(Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Settings {
    mode: Mode,
    manual_path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacySettings {
    java: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StoredSettings {
    Current(Settings),
    Legacy(LegacySettings),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ModeEdit {
    mode: Mode,
    manual_path: String,
    revision: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyEdit {
    java: String,
    revision: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Edit {
    Mode(ModeEdit),
    Legacy(LegacyEdit),
}

impl Settings {
    fn legacy(java: String) -> Self {
        let mode = if java.is_empty() { Mode::Automatic } else { Mode::Manual };
        Self { mode, manual_path: java }
    }

    fn effective_java(&self) -> &str {
        match self.mode {
            Mode::Automatic => "",
            Mode::Manual => &self.manual_path,
        }
    }

    fn response(&self, revision: String) -> Value {
        json!({"mode":self.mode,"manualPath":self.manual_path,"java":self.effective_java(),"revision":revision})
    }
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
        return Err("InvalidGlobalJavaSettings".into());
    }
    let state = match serde_json::from_slice(&bytes).map_err(|_| "InvalidGlobalJavaSettings: global-java.json")? {
        StoredSettings::Current(settings) => settings,
        StoredSettings::Legacy(settings) => Settings::legacy(settings.java),
    };
    Ok((state, content_revision::revision(&bytes)?, true))
}

fn explicit_path(raw: &str) -> Result<PathBuf> {
    let path = Path::new(raw);
    if !path.is_absolute() || raw.len() > 4096 || raw.contains(['\0', '\n', '\r']) {
        return Err("InvalidJavaPath".into());
    }
    let requested = scan_policy::rule_path(Path::new(""), &path_format::display(path))
        .map_err(|_| "InvalidJavaPath")?;
    let resolved = java_file(requested.to_str().ok_or("InvalidJavaPath")?)?;
    if !scan_policy::contains(&requested, &resolved) || !scan_policy::contains(&resolved, &requested) {
        return Err("PathRedirected".into());
    }
    Ok(resolved)
}

fn installed_major(executable: &Path) -> Result<u16> {
    if ["JAVA_TOOL_OPTIONS", "JDK_JAVA_OPTIONS", "_JAVA_OPTIONS"]
        .into_iter()
        .any(|key| std::env::var_os(key).is_some_and(|value| !value.is_empty()))
    {
        return Err("JavaEnvironmentOverride".into());
    }
    agent_profiles::java_major(&presentation::probe_version(executable)?)
        .ok_or_else(|| "JavaVersionUnconfirmed".into())
}

pub(super) fn supported_major(profile: &CoreProfile) -> Option<u16> {
    Some(match (profile.family.as_str(), profile.minecraft.as_str()) {
        ("VELOCITY", "4.2.1-SNAPSHOT") => 25,
        ("PAPER" | "FOLIA" | "SPIGOT", version) if version.starts_with("26.") => 25,
        ("PAPER" | "FOLIA", "1.20.6" | "1.21.8") => 21,
        ("SPIGOT", "1.8.8" | "1.12.2" | "1.14" | "1.16.5")
        | ("PAPER", "1.14" | "1.14.4") => 8,
        ("PAPER" | "FOLIA", version) => return paper_java_major(version),
        _ => return None,
    })
}

// PaperMC's version families, while preserving the already validated legacy pins above.
fn paper_java_major(version: &str) -> Option<u16> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse::<u16>().ok()?;
    let minor = parts.next()?.parse::<u16>().ok()?;
    let patch = parts.next().map(str::parse::<u16>).transpose().ok()?.unwrap_or(0);
    if major != 1 || parts.next().is_some() {
        return None;
    }
    match (minor, patch) {
        (7, 10..) | (8..=11, _) => Some(8),
        (12..=15, _) | (16, 0..=4) => Some(11),
        (16, 5) => Some(16),
        (17..=19, _) => Some(17),
        (20, _) | (21, 0..=11) => Some(21),
        _ => None,
    }
}

pub(super) fn resolve(root: &Path, profile: &CoreProfile) -> Result<PathBuf> {
    let major = supported_major(profile).ok_or("AutoJavaUnavailable")?;
    let (settings, _, _) = load(root)?;
    if settings.mode == Mode::Manual {
        let executable = explicit_path(&settings.manual_path).map_err(|_| "GlobalJavaUnavailable")?;
        if installed_major(&executable).map_err(|_| "GlobalJavaUnavailable")? != major {
            return Err("GlobalJavaIncompatible".into());
        }
        return Ok(executable);
    }

    let available = presentation::discover_java(root)?;
    let path = available.as_array().into_iter().flatten()
        .find_map(|row| {
            let version = row["version"].as_str().and_then(agent_profiles::java_major)?;
            if row["valid"] == true && row["stable"] == true && version == major {
                return row["path"].as_str();
            }
            None
        })
        .ok_or("AutoJavaUnavailable")?;
    java_file(path)
}

fn store(root: &Path, raw: &str) -> Result<Value> {
    if raw.len() > MAX_SETTINGS_BYTES {
        return Err("InvalidGlobalJavaSettings".into());
    }
    let edit: Edit = serde_json::from_str(raw).map_err(|_| "InvalidGlobalJavaSettings")?;
    let _lock = command_lock(root, "global-java.lock", false)?;
    let (current, revision, exists) = load(root)?;
    let (mode, raw_path, expected) = match edit {
        Edit::Mode(edit) => (edit.mode, edit.manual_path, edit.revision),
        Edit::Legacy(edit) if edit.java.trim().is_empty() => {
            (Mode::Automatic, current.manual_path.clone(), edit.revision)
        }
        Edit::Legacy(edit) => (Mode::Manual, edit.java, edit.revision),
    };
    if expected != revision {
        return Err("GlobalJavaConflict".into());
    }
    let raw_path = if mode == Mode::Automatic && raw_path.trim().is_empty() {
        current.manual_path.as_str()
    } else {
        raw_path.trim()
    };
    if raw_path.len() > 4096 || raw_path.chars().any(char::is_control) {
        return Err("InvalidJavaPath".into());
    }
    let (manual_path, major) = if mode == Mode::Automatic {
        // Inactive paths are suggestions only. A removed JDK must not
        // prevent saving Automatic or cause the remembered path to vanish.
        (raw_path.to_owned(), None)
    } else {
        let path = explicit_path(raw_path)?;
        let major = installed_major(&path)?;
        (path_format::display(&path), Some(major))
    };
    let next = Settings { mode, manual_path };
    let mut response = if next == current {
        current.response(revision)
    } else {
        let bytes = serde_json::to_vec_pretty(&next).map_err(fail)?;
        publish_json(&root.join(SETTINGS_FILE), &bytes, exists, || {
            let (_, latest, present) = load(root)?;
            if latest != revision || present != exists {
                return Err("GlobalJavaConflict".into());
            }
            Ok(())
        })?;
        next.response(content_revision::revision(&bytes)?)
    };
    response["major"] = json!(major);
    Ok(response)
}

const DISCOVERY_FILE: &str = "java-discovery.json";
const DISCOVERY_SCOPE_FILE: &str = "java-discovery-scope.json";
const MAX_DISCOVERY_ROWS: usize = 16;

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct DiscoverySources {
    pub(super) windows: bool,
    pub(super) mise: bool,
    pub(super) java_home: bool,
    pub(super) path: bool,
}

impl Default for DiscoverySources {
    fn default() -> Self {
        Self { windows: true, mise: true, java_home: true, path: true }
    }
}

#[derive(Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Discovery {
    sources: DiscoverySources,
    directories: Vec<String>,
    // Retained on disk so the preceding candidate can still read this file.
    #[serde(default)]
    excluded_directories: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyDiscovery {
    directory: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DiscoveryEdit {
    sources: DiscoverySources,
    directories: Vec<String>,
    revision: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DiscoveryDraft {
    sources: DiscoverySources,
    directories: Vec<String>,
}

pub(super) struct JavaScanScope {
    pub(super) sources: DiscoverySources,
    pub(super) directories: Vec<PathBuf>,
}

fn discovery_path(raw: &str) -> Result<PathBuf> {
    let path = scan_policy::rule_path(Path::new(""), raw).map_err(|_| "InvalidJavaScanDirectory")?;
    if !path.is_absolute() || path.parent().is_none() || raw.len() > 4096 {
        return Err("InvalidJavaScanDirectory".into());
    }
    Ok(path)
}

fn checked_directory(raw: &str) -> Result<PathBuf> {
    let path = discovery_path(raw)?;
    let resolved = fs::canonicalize(&path).map_err(|_| "InvalidJavaScanDirectory")?;
    if !resolved.is_dir() || !scan_policy::contains(&path, &resolved)
        || !scan_policy::contains(&resolved, &path)
    {
        return Err("InvalidJavaScanDirectory".into());
    }
    Ok(resolved)
}

fn load_discovery(root: &Path) -> Result<(Discovery, bool)> {
    if let Some(settings) = core_maintenance::read_optional::<Discovery>(&root.join(DISCOVERY_SCOPE_FILE))? {
        if settings.directories.len() > MAX_DISCOVERY_ROWS {
            return Err("InvalidJavaScanDirectory".into());
        }
        return Ok((settings, true));
    }
    let directories = core_maintenance::read_optional::<LegacyDiscovery>(&root.join(DISCOVERY_FILE))?
        .filter(|settings| !settings.directory.is_empty())
        .map(|settings| vec![settings.directory])
        .unwrap_or_default();
    Ok((Discovery { directories, ..Discovery::default() }, false))
}

pub(super) fn scan_scope(root: &Path) -> Result<JavaScanScope> {
    let (settings, _) = load_discovery(root)?;
    Ok(available_scope(settings.sources, &settings.directories))
}

fn available_scope(sources: DiscoverySources, directories: &[String]) -> JavaScanScope {
    JavaScanScope {
        sources,
        directories: checked_directories(directories).filter_map(Result::ok).collect(),
    }
}

fn preview_discovery(raw: &str) -> Result<Value> {
    if raw.len() > MAX_SETTINGS_BYTES {
        return Err("InvalidJavaScanDirectory".into());
    }
    let draft: DiscoveryDraft = serde_json::from_str(raw).map_err(|_| "InvalidJavaScanDirectory")?;
    if draft.directories.len() > MAX_DISCOVERY_ROWS {
        return Err("InvalidJavaScanDirectory".into());
    }
    presentation::scan_java(&available_scope(draft.sources, &draft.directories))
}

fn discovery_revision(settings: &Discovery) -> Result<String> {
    content_revision::revision(&serde_json::to_vec(settings).map_err(fail)?)
}

fn discovery_value(settings: &Discovery) -> Result<Value> {
    let unavailable: Vec<_> = settings.directories.iter()
        .filter(|raw| checked_directory(raw).is_err())
        .collect();
    Ok(json!({
        "sources": settings.sources,
        "directories": settings.directories,
        "unavailableDirectories": unavailable,
        "revision": discovery_revision(settings)?,
    }))
}

fn discovery_settings(root: &Path) -> Result<Value> {
    let (settings, _) = load_discovery(root)?;
    discovery_value(&settings)
}

fn parse_discovery_edit(raw: &str) -> Result<DiscoveryEdit> {
    if raw.len() > MAX_SETTINGS_BYTES {
        return Err("InvalidJavaScanDirectory".into());
    }
    let edit: DiscoveryEdit = serde_json::from_str(raw).map_err(|_| "InvalidJavaScanDirectory")?;
    if edit.directories.len() > MAX_DISCOVERY_ROWS {
        return Err("InvalidJavaScanDirectory".into());
    }
    Ok(edit)
}

fn checked_directories(directories: &[String]) -> impl Iterator<Item = Result<PathBuf>> + '_ {
    directories.iter().map(|raw| checked_directory(raw.trim()))
}

fn validate_discovery(raw: &str) -> Result<Value> {
    let edit = parse_discovery_edit(raw)?;
    let errors: Vec<_> = checked_directories(&edit.directories).map(Result::err).collect();
    Ok(json!({ "directoryErrors": errors }))
}

fn normalized_directories(directories: &[String]) -> Result<Vec<String>> {
    let mut unique = std::collections::HashSet::new();
    checked_directories(directories).map(|path| path.map(|path| path_format::display(&path)))
        .filter(|display| display.as_ref().map_or(true, |path| unique.insert(path.to_lowercase())))
        .collect()
}

fn store_discovery(root: &Path, raw: &str) -> Result<Value> {
    let edit = parse_discovery_edit(raw)?;
    let _guard = command_lock(root, "java-discovery.lock", false)?;
    let (current, exists) = load_discovery(root)?;
    if discovery_revision(&current)? != edit.revision {
        return Err("JavaScanDirectoryConflict".into());
    }
    let settings = Discovery {
        sources: edit.sources,
        directories: normalized_directories(&edit.directories)?,
        excluded_directories: Vec::new(),
    };
    if current != settings {
        // The former file remains readable by an older executable after rollback.
        core_maintenance::write_durable(&root.join(DISCOVERY_SCOPE_FILE), &settings, exists)?;
    }
    discovery_value(&settings)
}

pub(super) fn dispatch(root: &Path, verb: &str, args: &[String]) -> Option<Result<Value>> {
    Some(match (verb, args) {
        ("java-discovery-settings", []) => discovery_settings(root),
        ("preview-java-discovery", [raw]) => preview_discovery(raw),
        ("validate-java-discovery-settings", [raw]) => validate_discovery(raw),
        ("save-java-discovery-settings", [raw]) => store_discovery(root, raw),
        ("java-discovery-settings" | "preview-java-discovery" | "validate-java-discovery-settings" | "save-java-discovery-settings", _) => Err("InvalidJavaScanDirectory".into()),
        ("global-java-settings", []) => load(root).map(|(settings, revision, _)| {
            settings.response(revision)
        }),
        ("save-global-java-settings", [raw]) => store(root, raw),
        ("global-java-settings" | "save-global-java-settings", _) => {
            Err("InvalidGlobalJavaSettings".into())
        }
        _ => return None,
    })
}

pub(super) fn is_unavailable(error: &str) -> bool {
    matches!(error, "AutoJavaUnavailable" | "GlobalJavaUnavailable" | "GlobalJavaIncompatible")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_keep_legacy_and_current_java_separate() {
        let profile = |family: &str, minecraft: &str| CoreProfile {
            family: family.into(), minecraft: minecraft.into(), build: String::new(), sha256: String::new(),
        };
        assert_eq!(supported_major(&profile("SPIGOT", "1.8.8")), Some(8));
        assert_eq!(supported_major(&profile("PAPER", "26.3")), Some(25));
        assert_eq!(supported_major(&profile("VELOCITY", "4.2.1-SNAPSHOT")), Some(25));
        assert_eq!(supported_major(&profile("OTHER", "26.3")), None);
    }

    #[test]
    fn discovery_scope_migrates_without_changing_legacy_data_and_rejects_stale_edits() {
        let root = std::env::temp_dir().join(format!("ih-java-scan-{}-{}", std::process::id(), clock()));
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let initial = discovery_settings(&root).unwrap();
        assert_eq!(initial["directories"], json!([]));
        assert_eq!(initial["sources"], json!({"windows":true,"mise":true,"javaHome":true,"path":true}));
        assert!(scan_scope(&root).unwrap().directories.is_empty());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        let first = root.join("jdk");
        let second = root.join("other-jdk");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        let legacy = json!({"directory":path_format::display(&first)}).to_string();
        fs::write(root.join(DISCOVERY_FILE), &legacy).unwrap();
        let previous = discovery_settings(&root).unwrap();
        assert_eq!(previous["directories"], json!([path_format::display(&first)]));
        assert!(!root.join(DISCOVERY_SCOPE_FILE).exists());
        let disabled = json!({"windows":false,"mise":false,"javaHome":false,"path":false});
        let mut edit = json!({"sources":disabled,"directories":["relative/java",path_format::display(&second)],
            "revision":previous["revision"]});
        assert_eq!(validate_discovery(&edit.to_string()).unwrap(), json!({
            "directoryErrors":["InvalidJavaScanDirectory",null]
        }));
        assert_eq!(store_discovery(&root, &edit.to_string()).unwrap_err(), "InvalidJavaScanDirectory");
        assert!(!root.join(DISCOVERY_SCOPE_FILE).exists());
        edit["directories"] = json!([path_format::display(&first),path_format::display(&second),path_format::display(&first)]);
        let saved = store_discovery(&root, &edit.to_string()).unwrap();
        assert_eq!(discovery_settings(&root).unwrap(), saved);
        let scope = scan_scope(&root).unwrap();
        assert_eq!(scope.directories, vec![first.clone(),second.clone()]);
        assert_eq!(serde_json::to_value(scope.sources).unwrap(), disabled);
        assert_eq!(store_discovery(&root, &edit.to_string()).unwrap_err(), "JavaScanDirectoryConflict");
        let clear = json!({"sources":initial["sources"],"directories":[], "revision":saved["revision"]}).to_string();
        assert_eq!(store_discovery(&root, &clear).unwrap()["directories"], json!([]));
        assert!(scan_scope(&root).unwrap().directories.is_empty());
        assert_eq!(fs::read_to_string(root.join(DISCOVERY_FILE)).unwrap(), legacy);
        assert!(!root.join(SETTINGS_FILE).exists());
        fs::remove_dir(first).unwrap();
        fs::remove_dir(second).unwrap();
        fs::remove_file(root.join(DISCOVERY_SCOPE_FILE)).unwrap();
        fs::remove_file(root.join(DISCOVERY_FILE)).unwrap();
        fs::remove_file(root.join("java-discovery.lock")).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn draft_probe_is_read_only_and_old_exclusions_no_longer_apply() {
        let root = std::env::temp_dir().join(format!("ih-java-draft-{}-{}", std::process::id(), clock()));
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let jdk = root.join("jdk");
        let java = jdk.join("bin/java.exe");
        fs::create_dir_all(java.parent().unwrap()).unwrap();
        // An inert executable proves discovery visits this path without requiring a local JDK.
        fs::write(&java, []).unwrap();
        let sources = json!({"windows":false,"mise":false,"javaHome":false,"path":false});
        let draft = json!({"sources":sources,"directories":["", "relative/java", path_format::display(&jdk)]}).to_string();
        let preview = dispatch(&root, "preview-java-discovery", &[draft.clone()]).unwrap().unwrap();
        assert_eq!(preview.as_array().unwrap().len(), 1);
        assert_eq!(preview[0]["path"], path_format::display(&java));
        assert_eq!(preview[0]["valid"], false);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        assert!(scan_scope(&root).unwrap().directories.is_empty());

        let previous = json!({
            "sources":sources, "directories":[path_format::display(&jdk)],
            "excludedDirectories":[path_format::display(&jdk)]
        });
        let path = root.join(DISCOVERY_SCOPE_FILE);
        fs::write(&path, previous.to_string()).unwrap();
        let saved_bytes = fs::read(&path).unwrap();
        assert_eq!(presentation::discover_java(&root).unwrap(), preview);
        assert_eq!(dispatch(&root, "preview-java-discovery", &[draft]).unwrap().unwrap(), preview);
        assert_eq!(fs::read(&path).unwrap(), saved_bytes);

        let saved = discovery_settings(&root).unwrap();
        assert!(saved.get("excludedDirectories").is_none());
        let edit = json!({"sources":saved["sources"],"directories":saved["directories"],"revision":saved["revision"]});
        store_discovery(&root, &edit.to_string()).unwrap();
        let after: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(after["excludedDirectories"], json!([]));

        fs::remove_file(root.join("java-discovery.lock")).unwrap();
        fs::remove_file(&path).unwrap();
        fs::remove_file(&java).unwrap();
        fs::remove_dir(java.parent().unwrap()).unwrap();
        fs::remove_dir(jdk).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn absent_settings_are_read_only_and_stale_or_relative_edits_are_rejected() {
        let root = std::env::temp_dir().join(format!("ih-global-java-{}-{}", std::process::id(), clock()));
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let (settings, revision, exists) = load(&root).unwrap();
        assert!(settings.manual_path.is_empty() && settings.mode == Mode::Automatic && !exists);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        let stale = json!({"java":"","revision":"old"});
        assert_eq!(store(&root, &stale.to_string()).unwrap_err(), "GlobalJavaConflict");
        let relative = json!({"java":"tools/java.exe","revision":revision});
        assert_eq!(store(&root, &relative.to_string()).unwrap_err(), "InvalidJavaPath");
        assert!(!root.join(SETTINGS_FILE).exists());
        fs::remove_file(root.join("global-java.lock")).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn legacy_manual_path_survives_automatic_mode_and_a_later_process_read() {
        let root = std::env::temp_dir().join(format!("ih-global-java-retain-{}-{}", std::process::id(), clock()));
        fs::create_dir(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let missing = "C:/unavailable-java-fixture/bin/java.exe";
        save(&root.join(SETTINGS_FILE), &json!({"java":missing})).unwrap();
        let (original, revision, _) = load(&root).unwrap();
        assert!(original.mode == Mode::Manual);
        assert_eq!(original.effective_java(), missing);

        // Legacy clients sent an empty java value when switching to Automatic.
        let switched = store(&root, &json!({"java":"","revision":revision}).to_string()).unwrap();
        assert_eq!(switched["mode"], "automatic");
        assert_eq!(switched["java"], "");
        assert_eq!(switched["manualPath"], missing);
        let (saved, revision, _) = load(&root).unwrap();
        assert!(saved.mode == Mode::Automatic);
        assert_eq!(saved.manual_path, missing);

        let auto = json!({"mode":"automatic","manualPath":"","revision":revision});
        assert_eq!(store(&root, &auto.to_string()).unwrap()["manualPath"], missing);
        let invalid = json!({"mode":"manual","manualPath":missing,"revision":revision});
        assert!(store(&root, &invalid.to_string()).is_err());
        let (still_saved, _, _) = load(&root).unwrap();
        assert!(still_saved == saved);
        fs::remove_file(root.join(SETTINGS_FILE)).unwrap();
        fs::remove_file(root.join("global-java.lock")).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
