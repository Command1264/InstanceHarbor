//! Test administrators configured for one workspace, the pinned LuckPerms
//! cache, and the permission data seeded for them into new instances.
//!
//! The identities live only in the workspace's `test-administrators.json`.
//! Creation reads the cache and never downloads; only the explicit fetch
//! action downloads, and only the pinned files from the policy's sources.
use super::*;

const SETTINGS_FILE: &str = "test-administrators.json";
const MAX_SETTINGS_BYTES: usize = 8192;
pub(super) const MAX_ADMINISTRATORS: usize = 10;
const POLICY: &str = include_str!("../../config/test-admin-permissions.json");
const OFFICIAL_SOURCES: [&str; 3] = [
    "https://download.luckperms.net/",
    "https://ci.lucko.me/job/LuckPerms/",
    "https://cdn.modrinth.com/data/Vebnzrzj/",
];
const SEEDED_CONFIG: &str = "# Written by InstanceHarbor when this instance was created.\n\
# LuckPerms uses its defaults for every setting not listed here.\n\
storage-method: json\n";

#[derive(Clone, Default, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Settings {
    administrators: Vec<Administrator>,
    /// Empty grants `* = true` directly; otherwise members join this group.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    group: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Administrator {
    name: String,
    uuid: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Edit {
    administrators: Vec<Administrator>,
    #[serde(default)]
    group: String,
    revision: String,
}

fn valid_name(name: &str) -> bool {
    (3..=16).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn normalized_uuid(raw: &str) -> Option<String> {
    let lower = raw.trim().to_ascii_lowercase();
    let hex: String = match lower.len() {
        32 => lower.clone(),
        36 if [8, 13, 18, 23].iter().all(|&i| lower.as_bytes()[i] == b'-') => lower.replace('-', ""),
        _ => return None,
    };
    if hex.len() != 32 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..]))
}

fn valid_group(group: &str) -> bool {
    // Granting `*` to `default` would give every player all permissions.
    group.is_empty()
        || ((1..=36).contains(&group.len())
            && group != "default"
            && group.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-'))
}

impl Settings {
    fn normalized(administrators: Vec<Administrator>, group: String) -> Result<Self> {
        if administrators.len() > MAX_ADMINISTRATORS {
            return Err("TooManyTestAdministrators".into());
        }
        let mut seen = Vec::with_capacity(administrators.len());
        let mut checked = Vec::with_capacity(administrators.len());
        for item in administrators {
            let name = item.name.trim().to_owned();
            if !valid_name(&name) {
                return Err("InvalidTestAdministratorName".into());
            }
            let uuid = normalized_uuid(&item.uuid).ok_or("InvalidTestAdministratorUuid")?;
            if seen.contains(&uuid) {
                return Err("DuplicateTestAdministrator".into());
            }
            seen.push(uuid.clone());
            checked.push(Administrator { name, uuid });
        }
        let group = group.trim().to_owned();
        if !valid_group(&group) {
            return Err("InvalidTestAdministratorGroup".into());
        }
        Ok(Self { administrators: checked, group })
    }

    pub(super) fn enabled(&self) -> bool {
        !self.administrators.is_empty()
    }

    pub(super) fn count(&self) -> usize {
        self.administrators.len()
    }

    fn user_file(&self, administrator: &Administrator) -> Result<Vec<u8>> {
        let mut permissions = vec![json!({"permission":"group.default","value":true})];
        permissions.push(if self.group.is_empty() {
            json!({"permission":"*","value":true})
        } else {
            json!({"permission":format!("group.{}", self.group),"value":true})
        });
        serde_json::to_vec_pretty(&json!({
            "uuid":administrator.uuid, "name":administrator.name,
            "primaryGroup":"default", "permissions":permissions,
        }))
        .map_err(fail)
    }
}

pub(super) fn load(root: &Path) -> Result<(Settings, String, bool)> {
    let path = root.join(SETTINGS_FILE);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok((Settings::default(), content_revision::revision(b"")?, false));
        }
        Err(error) => return Err(file_error("read", &path, error)),
        Ok(metadata) if !metadata.is_file() => return Err("InvalidTestAdministratorSettings".into()),
        Ok(_) => no_redirect(&path)?,
    }
    let file = retry_shared_file(|| File::open(&path)).map_err(|error| file_error("read", &path, error))?;
    let mut bytes = Vec::new();
    file.take((MAX_SETTINGS_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| file_error("read", &path, error))?;
    if bytes.len() > MAX_SETTINGS_BYTES {
        return Err("InvalidTestAdministratorSettings".into());
    }
    let stored: Settings = serde_json::from_slice(&bytes)
        .map_err(|_| "InvalidTestAdministratorSettings: test-administrators.json")?;
    let settings = Settings::normalized(stored.administrators, stored.group)?;
    Ok((settings, content_revision::revision(&bytes)?, true))
}

fn response(root: &Path, settings: &Settings, revision: String) -> Result<Value> {
    Ok(json!({
        "administrators":settings.administrators, "group":settings.group, "revision":revision,
        "artifacts":artifacts(root)?,
    }))
}

fn store(root: &Path, raw: &str) -> Result<Value> {
    if raw.len() > MAX_SETTINGS_BYTES {
        return Err("InvalidTestAdministratorSettings".into());
    }
    let edit: Edit = serde_json::from_str(raw).map_err(|_| "InvalidTestAdministratorSettings")?;
    let next = Settings::normalized(edit.administrators, edit.group)?;
    let _lock = command_lock(root, "test-administrators.lock", false)?;
    let (current, revision, exists) = load(root)?;
    if edit.revision != revision {
        return Err("TestAdministratorSettingsConflict".into());
    }
    if next == current {
        return response(root, &current, revision);
    }
    let bytes = serde_json::to_vec_pretty(&next).map_err(fail)?;
    publish_json(&root.join(SETTINGS_FILE), &bytes, exists, || {
        let (_, latest, present) = load(root)?;
        if latest != revision || present != exists {
            return Err("TestAdministratorSettingsConflict".into());
        }
        Ok(())
    })?;
    response(root, &next, content_revision::revision(&bytes)?)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Platform {
    Bukkit,
    BukkitJava8,
    Velocity,
}

const PLATFORMS: [Platform; 3] = [Platform::Bukkit, Platform::BukkitJava8, Platform::Velocity];

impl Platform {
    fn key(self) -> &'static str {
        match self {
            Self::Bukkit => "bukkit",
            Self::BukkitJava8 => "bukkit-java8",
            Self::Velocity => "velocity",
        }
    }

    /// LuckPerms keeps its data in the plugin's own folder name.
    fn data_folder(self) -> &'static str {
        match self {
            Self::Bukkit | Self::BukkitJava8 => "LuckPerms",
            Self::Velocity => "luckperms",
        }
    }

    /// LuckPerms 5.5 is compiled for Java 11, so cores that still run on
    /// Java 8 use the pinned 5.4 line.
    pub(super) fn for_core(profile: &CoreProfile) -> Result<Self> {
        match profile.family.as_str() {
            "VELOCITY" => Ok(Self::Velocity),
            "PAPER" | "FOLIA" | "SPIGOT" | "PURPUR" | "CRAFTBUKKIT" => {
                let java = global_java::supported_major(profile).or(profile.java);
                Ok(if java.is_some_and(|major| major < 11) { Self::BukkitJava8 } else { Self::Bukkit })
            }
            _ => Err("UnsupportedTestPermissionPlatform".into()),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct Pin {
    file_name: String,
    sha256: String,
    url: String,
    version: String,
}

fn pin(platform: Platform) -> Result<Pin> {
    let policy: Value = serde_json::from_str(POLICY).map_err(fail)?;
    let entry = &policy["artifacts"][platform.key()];
    let text = |key: &str| entry[key].as_str().map(str::to_owned).ok_or("InvalidTestPermissionPolicy");
    let file_name = text("fileName")?;
    let sha256 = text("sha256")?.to_ascii_lowercase();
    let url = text("url")?;
    let version = text("version")?;
    let safe_name = file_name.starts_with("LuckPerms-")
        && file_name.ends_with(".jar")
        && file_name.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b));
    let official = OFFICIAL_SOURCES.iter().any(|prefix| url.starts_with(prefix))
        && url.bytes().all(|b| b.is_ascii_graphic() && !b"\"'<>\\".contains(&b));
    if policy["schemaVersion"] != 1
        || !safe_name
        || !official
        || sha256.len() != 64
        || !sha256.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("InvalidTestPermissionPolicy".into());
    }
    Ok(Pin { file_name, sha256, url, version })
}

fn cache_directory(root: &Path, create: bool) -> Result<Option<PathBuf>> {
    let mut path = root.to_path_buf();
    for name in ["plugin-cache", "luckperms"] {
        let next = path.join(name);
        match fs::symlink_metadata(&next) {
            Err(error) if error.kind() == io::ErrorKind::NotFound && create => {
                fs::create_dir(&next).map_err(|error| file_error("create", &next, error))?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(file_error("inspect", &next, error)),
            Ok(_) => {}
        }
        path = child_directory(&path, name)?;
    }
    Ok(Some(path))
}

#[derive(Debug, PartialEq, Eq)]
enum CacheState {
    Ready(PathBuf),
    Missing,
    Mismatch,
}

fn cache_state(root: &Path, pin: &Pin) -> Result<CacheState> {
    let Some(directory) = cache_directory(root, false)? else { return Ok(CacheState::Missing) };
    let path = directory.join(&pin.file_name);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(CacheState::Missing),
        Err(error) => Err(file_error("inspect", &path, error)),
        Ok(metadata) if !metadata.is_file() => Ok(CacheState::Mismatch),
        Ok(_) if content_revision::file_revision(&path)? == pin.sha256 => Ok(CacheState::Ready(path)),
        Ok(_) => Ok(CacheState::Mismatch),
    }
}

fn artifacts(root: &Path) -> Result<Vec<Value>> {
    PLATFORMS
        .iter()
        .map(|&platform| {
            let pin = pin(platform)?;
            let state = match cache_state(root, &pin)? {
                CacheState::Ready(_) => "Ready",
                CacheState::Missing => "Missing",
                CacheState::Mismatch => "Mismatch",
            };
            Ok(json!({"platform":platform.key(),"fileName":pin.file_name,"version":pin.version,"sha256":pin.sha256,"state":state}))
        })
        .collect()
}

/// Downloads pinned files that are missing or do not match. Each download is
/// verified before it replaces anything in the workspace cache.
fn fetch(root: &Path) -> Result<Value> {
    let _lock = command_lock(root, "test-administrators-cache.lock", false)?;
    let directory = cache_directory(root, true)?.ok_or("TestPermissionCacheUnavailable")?;
    for platform in PLATFORMS {
        let pin = pin(platform)?;
        let state = cache_state(root, &pin)?;
        if matches!(state, CacheState::Ready(_)) {
            continue;
        }
        let partial = directory.join(format!("{}.{}-{}.partial", pin.file_name, std::process::id(), clock()));
        let outcome = core_source::download_pinned(&pin.url, &partial, &pin.sha256)
            .and_then(|()| core_maintenance::move_durable(&partial, &directory.join(&pin.file_name), state == CacheState::Mismatch));
        if let Err(error) = outcome {
            if partial.exists() {
                if let Err(cleanup) = fs::remove_file(&partial) {
                    eprintln!("LuckPerms download cleanup: {cleanup}");
                }
            }
            return Err(error);
        }
    }
    Ok(json!({"artifacts":artifacts(root)?}))
}

/// A verified plan to give one new instance its test administrators.
pub(super) struct Grant {
    platform: Platform,
    source: PathBuf,
    pin: Pin,
    settings: Settings,
    revision: String,
}

impl Grant {
    pub(super) fn summary(&self) -> Value {
        json!({
            "count":self.settings.administrators.len(), "group":self.settings.group,
            "fileName":self.pin.file_name, "version":self.pin.version, "sha256":self.pin.sha256,
            "settingsRevision":self.revision,
        })
    }
}

/// New instances under `instances/Agent` start without test permissions.
pub(super) fn agent_path(root: &Path, target: &Path) -> bool {
    target.starts_with(root.join("instances").join("Agent"))
}

/// Returns the number of configured administrators and, when requested and
/// configured, the grant. A missing or mismatched cache stops creation.
pub(super) fn plan(root: &Path, profile: &CoreProfile, requested: bool) -> Result<(usize, Option<Grant>)> {
    let (settings, revision, _) = load(root)?;
    let configured = settings.administrators.len();
    if !requested || !settings.enabled() {
        return Ok((configured, None));
    }
    let platform = Platform::for_core(profile)?;
    let pin = pin(platform)?;
    let source = match cache_state(root, &pin)? {
        CacheState::Ready(path) => path,
        CacheState::Missing => return Err("TestPermissionArtifactMissing".into()),
        CacheState::Mismatch => return Err("TestPermissionArtifactMismatch".into()),
    };
    Ok((configured, Some(Grant { platform, source, pin, settings, revision })))
}

/// Copies the verified plugin into a new instance and seeds its data.
pub(super) fn install(directory: &Path, grant: &Grant) -> Result<()> {
    let plugins = directory.join("plugins");
    match fs::create_dir(&plugins) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(file_error("create", &plugins, error)),
    }
    let plugins = child_directory(directory, "plugins")?;
    core_maintenance::copy_verified(&grant.source, &plugins.join(&grant.pin.file_name), &grant.pin.sha256)?;
    seed(directory, grant.platform, &grant.settings)
}

/// Writes LuckPerms JSON storage for the configured administrators. Existing
/// LuckPerms data is never replaced: those instances need a manual grant.
pub(super) fn seed(directory: &Path, platform: Platform, settings: &Settings) -> Result<()> {
    let plugins = child_directory(directory, "plugins")?;
    let data = plugins.join(platform.data_folder());
    match fs::symlink_metadata(&data) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(file_error("inspect", &data, error)),
        Ok(_) => return Err("TestPermissionDataExists".into()),
    }
    let create = |path: &Path| fs::create_dir(path).map_err(|error| file_error("create", path, error));
    let write = |path: &Path, bytes: &[u8]| -> Result<()> {
        let mut file = File::create_new(path).map_err(|error| file_error("create", path, error))?;
        file.write_all(bytes).and_then(|()| file.sync_all()).map_err(|error| file_error("write", path, error))
    };
    let storage = data.join("json-storage");
    let users = storage.join("users");
    for folder in [&data, &storage, &users] {
        create(folder)?;
    }
    write(&data.join("config.yml"), SEEDED_CONFIG.as_bytes())?;
    // LuckPerms resolves `lp user <name>` through this cache before a first login.
    let mut lookup = String::from("# LuckPerms UUID lookup cache
");
    for administrator in &settings.administrators {
        write(&users.join(format!("{}.json", administrator.uuid)), &settings.user_file(administrator)?)?;
        lookup.push_str(&format!("{}:{}
", administrator.uuid, administrator.name));
    }
    write(&storage.join("uuidcache.txt"), lookup.as_bytes())?;
    if !settings.group.is_empty() {
        let groups = storage.join("groups");
        create(&groups)?;
        let group = serde_json::to_vec_pretty(&json!({
            "name":settings.group, "permissions":[{"permission":"*","value":true}],
        }))
        .map_err(fail)?;
        write(&groups.join(format!("{}.json", settings.group)), &group)?;
    }
    Ok(())
}

pub(super) fn dispatch(root: &Path, verb: &str, args: &[String]) -> Option<Result<Value>> {
    Some(match (verb, args) {
        ("test-admin-settings", []) => load(root).and_then(|(settings, revision, _)| response(root, &settings, revision)),
        ("save-test-admin-settings", [raw]) => store(root, raw),
        ("test-admin-fetch-luckperms", []) => fetch(root),
        ("test-admin-settings" | "save-test-admin-settings" | "test-admin-fetch-luckperms", _) => {
            Err("InvalidTestAdministratorArguments".into())
        }
        _ => return None,
    })
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);
    // Obviously fake identities: never use a real player's UUID here.
    const FAKE_UUID: &str = "00000000-0000-4000-8000-000000000001";
    const OTHER_UUID: &str = "00000000000040008000000000000002";

    struct Workspace(PathBuf);

    impl Workspace {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "ih-test-admins-{}-{}-{}", std::process::id(), clock(), NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            super::super::dispatch(&[path.to_string_lossy().into_owned(), "init".into()]).unwrap();
            Self(safe_directory(&path).unwrap())
        }
    }

    impl Drop for Workspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn admin(name: &str, uuid: &str) -> Administrator {
        Administrator { name: name.into(), uuid: uuid.into() }
    }

    fn profile(family: &str, minecraft: &str) -> CoreProfile {
        CoreProfile {
            unverified: false, java: None, family: family.into(), minecraft: minecraft.into(),
            build: "1".into(), sha256: "0".repeat(64),
        }
    }

    pub(in super::super) fn save_fake_administrators(root: &Path, group: &str) {
        let (_, revision, _) = load(root).unwrap();
        let raw = json!({
            "administrators":[{"name":"Fake_Admin","uuid":FAKE_UUID},{"name":"OtherFake","uuid":OTHER_UUID}],
            "group":group, "revision":revision,
        });
        store(root, &raw.to_string()).unwrap();
    }

    /// Places a stand-in file in the cache. It never matches the real pin.
    fn cache_stand_in(root: &Path, platform: Platform) {
        let directory = cache_directory(root, true).unwrap().unwrap();
        fs::write(directory.join(pin(platform).unwrap().file_name), format!("stand-in for {}", platform.key())).unwrap();
    }

    /// A grant whose pin is a stand-in file, for tests that stage instances.
    pub(in super::super) fn stand_in_grant(root: &Path, platform: Platform) -> Grant {
        save_fake_administrators(root, "");
        let source = root.join("luckperms-stand-in.jar");
        fs::write(&source, format!("stand-in for {}", platform.key())).unwrap();
        let mut pin = pin(platform).unwrap();
        pin.sha256 = content_revision::file_revision(&source).unwrap();
        let (settings, revision, _) = load(root).unwrap();
        Grant { platform, source, pin, settings, revision }
    }

    #[test]
    fn settings_normalize_identities_and_reject_unsafe_values() {
        let settings = Settings::normalized(
            vec![admin(" Fake_Admin ", &FAKE_UUID.to_uppercase()), admin("OtherFake", OTHER_UUID)],
            String::new(),
        )
        .unwrap();
        assert_eq!(settings.administrators[0], admin("Fake_Admin", FAKE_UUID));
        assert_eq!(settings.administrators[1].uuid, "00000000-0000-4000-8000-000000000002");
        for (items, group, error) in [
            (vec![admin("no", FAKE_UUID)], "", "InvalidTestAdministratorName"),
            (vec![admin("bad-name", FAKE_UUID)], "", "InvalidTestAdministratorName"),
            (vec![admin("Fake_Admin", "not-a-uuid")], "", "InvalidTestAdministratorUuid"),
            (vec![admin("Fake_Admin", FAKE_UUID), admin("OtherFake", FAKE_UUID)], "", "DuplicateTestAdministrator"),
            (vec![], "default", "InvalidTestAdministratorGroup"),
            (vec![], "Admins", "InvalidTestAdministratorGroup"),
            (vec![admin("Fake_Admin", FAKE_UUID); MAX_ADMINISTRATORS + 1], "", "TooManyTestAdministrators"),
        ] {
            assert_eq!(Settings::normalized(items, group.into()).unwrap_err(), error);
        }
    }

    #[test]
    fn settings_round_trip_with_revision_conflicts() {
        let workspace = Workspace::new();
        let root = &workspace.0;
        let (empty, first, exists) = load(root).unwrap();
        assert!(!empty.enabled() && !exists);
        let saved = store(root, &json!({"administrators":[{"name":"Fake_Admin","uuid":FAKE_UUID}],"revision":first}).to_string()).unwrap();
        assert_eq!(saved["administrators"][0]["uuid"], FAKE_UUID);
        assert_eq!(saved["artifacts"].as_array().unwrap().len(), 3);
        assert!(saved["artifacts"].as_array().unwrap().iter().all(|row| row["state"] == "Missing"));
        assert_eq!(
            store(root, &json!({"administrators":[],"revision":first}).to_string()).unwrap_err(),
            "TestAdministratorSettingsConflict"
        );
        let cleared = store(root, &json!({"administrators":[],"revision":saved["revision"]}).to_string()).unwrap();
        assert_eq!(cleared["administrators"], json!([]));
        assert!(!load(root).unwrap().0.enabled());
    }

    #[test]
    fn repository_policy_names_only_pinned_official_artifacts() {
        for platform in PLATFORMS {
            let pin = pin(platform).unwrap();
            assert!(pin.file_name.starts_with("LuckPerms-"));
            assert!(OFFICIAL_SOURCES.iter().any(|source| pin.url.starts_with(source)));
        }
        assert_eq!(pin(Platform::Velocity).unwrap().file_name, "LuckPerms-Velocity-5.5.85.jar");
    }

    #[test]
    fn platform_follows_core_family_and_java_line() {
        assert_eq!(Platform::for_core(&profile("PAPER", "26.3")).unwrap(), Platform::Bukkit);
        assert_eq!(Platform::for_core(&profile("FOLIA", "26.1.2")).unwrap(), Platform::Bukkit);
        assert_eq!(Platform::for_core(&profile("SPIGOT", "1.8.8")).unwrap(), Platform::BukkitJava8);
        assert_eq!(Platform::for_core(&profile("PAPER", "1.14.4")).unwrap(), Platform::BukkitJava8);
        assert_eq!(Platform::for_core(&profile("VELOCITY", "3.4.0")).unwrap(), Platform::Velocity);
        assert_eq!(Platform::for_core(&profile("FORGE", "1.20.1")).unwrap_err(), "UnsupportedTestPermissionPlatform");
    }

    #[test]
    fn plan_requires_request_identities_and_a_verified_cache() {
        let workspace = Workspace::new();
        let root = &workspace.0;
        let paper = profile("PAPER", "26.3");
        assert!(plan(root, &paper, true).unwrap().1.is_none(), "no identities configured");
        save_fake_administrators(root, "");
        assert_eq!(plan(root, &paper, false).unwrap().0, 2);
        assert!(plan(root, &paper, false).unwrap().1.is_none(), "not requested");
        assert_eq!(plan(root, &paper, true).err().unwrap(), "TestPermissionArtifactMissing");
        cache_stand_in(root, Platform::Bukkit);
        assert_eq!(plan(root, &paper, true).err().unwrap(), "TestPermissionArtifactMismatch");
        assert_eq!(artifacts(root).unwrap()[0]["state"], "Mismatch");
    }

    #[test]
    fn seeding_writes_json_storage_and_never_replaces_existing_data() {
        let workspace = Workspace::new();
        let directory = workspace.0.join("instances").join("seeded");
        fs::create_dir_all(directory.join("plugins")).unwrap();
        let settings = Settings::normalized(
            vec![admin("Fake_Admin", FAKE_UUID), admin("OtherFake", OTHER_UUID)],
            String::new(),
        )
        .unwrap();
        seed(&directory, Platform::Bukkit, &settings).unwrap();
        let data = directory.join("plugins").join("LuckPerms");
        assert!(fs::read_to_string(data.join("config.yml")).unwrap().contains("\nstorage-method: json\n"));
        let user: Value = serde_json::from_slice(
            &fs::read(data.join("json-storage").join("users").join(format!("{FAKE_UUID}.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(user["name"], "Fake_Admin");
        assert_eq!(user["primaryGroup"], "default");
        assert!(user["permissions"].as_array().unwrap().contains(&json!({"permission":"*","value":true})));
        assert_eq!(
            fs::read_to_string(data.join("json-storage").join("uuidcache.txt")).unwrap(),
            format!("# LuckPerms UUID lookup cache
{FAKE_UUID}:Fake_Admin
00000000-0000-4000-8000-000000000002:OtherFake
")
        );
        assert!(!data.join("json-storage").join("groups").exists());
        assert!(!directory.join("ops.json").exists(), "LuckPerms grants never add OP");
        assert_eq!(seed(&directory, Platform::Bukkit, &settings).unwrap_err(), "TestPermissionDataExists");
    }

    #[test]
    fn group_mode_grants_through_a_seeded_group_and_velocity_uses_its_own_folder() {
        let workspace = Workspace::new();
        let directory = workspace.0.join("instances").join("proxy");
        fs::create_dir_all(directory.join("plugins")).unwrap();
        let settings = Settings::normalized(vec![admin("Fake_Admin", FAKE_UUID)], "test-admin".into()).unwrap();
        seed(&directory, Platform::Velocity, &settings).unwrap();
        let storage = directory.join("plugins").join("luckperms").join("json-storage");
        let user = fs::read_to_string(storage.join("users").join(format!("{FAKE_UUID}.json"))).unwrap();
        assert!(user.contains("group.test-admin") && !user.contains("\"*\""));
        let group: Value = serde_json::from_slice(&fs::read(storage.join("groups").join("test-admin.json")).unwrap()).unwrap();
        assert_eq!(group["permissions"], json!([{"permission":"*","value":true}]));
    }

    #[test]
    fn only_paths_under_the_agent_folder_default_to_no_grant() {
        let root = Path::new(r"C:\workspace");
        assert!(agent_path(root, &root.join("instances").join("Agent").join("Lobby").join("a")));
        assert!(!agent_path(root, &root.join("instances").join("paper")));
        assert!(!agent_path(root, &root.join("instances").join("Agents").join("a")));
    }
}
