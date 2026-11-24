//! Test administrators configured for one workspace, the pinned LuckPerms
//! cache, and the permission data seeded for them into new instances.
//!
//! The identities live only in the workspace's `test-administrators.json`.
//! Creation reads the cache and never downloads; only the explicit fetch
//! action downloads, and only the pinned files from the policy's sources.
//! The one other network call is the Mojang profile lookup that turns a typed
//! name into its online UUID.
use super::*;

const SETTINGS_FILE: &str = "test-administrators.json";
const MAX_SETTINGS_BYTES: usize = 8192;
pub(super) const MAX_ADMINISTRATORS: usize = 10;
const MAX_PERMISSION_NODES: usize = 50;
const MAX_PERMISSION_NODE_LENGTH: usize = 200;
const POLICY: &str = include_str!("../../config/test-admin-permissions.json");
const OFFICIAL_SOURCES: [&str; 3] = [
    "https://download.luckperms.net/",
    "https://ci.lucko.me/job/LuckPerms/",
    "https://cdn.modrinth.com/data/Vebnzrzj/",
];
const PROFILE_LOOKUP: &str = "https://api.minecraftservices.com/minecraft/profile/lookup/name/";
const SEEDED_CONFIG: &str = "# Written by InstanceHarbor when this instance was created.\n\
# LuckPerms uses its defaults for every setting not listed here.\n\
storage-method: json\n";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Settings {
    administrators: Vec<Administrator>,
    /// Files written before OP existed have only LuckPerms data.
    #[serde(default = "enabled_by_default")]
    luckperms: bool,
    /// Writes `ops.json` (level 4) when an instance is created.
    #[serde(default)]
    op: bool,
    /// `None` grants `*`. Otherwise one node per entry; a leading `-` sets false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    permissions: Option<Vec<String>>,
    /// Custom nodes kept while `*` is chosen, so switching back restores them. Never granted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    kept_permissions: Vec<String>,
    /// Empty puts the nodes on the user; otherwise members join this group.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    group: String,
}

fn enabled_by_default() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            administrators: Vec::new(),
            luckperms: true,
            op: false,
            permissions: None,
            kept_permissions: Vec::new(),
            group: String::new(),
        }
    }
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
    #[serde(default = "enabled_by_default")]
    luckperms: bool,
    #[serde(default)]
    op: bool,
    #[serde(default)]
    permissions: Option<Vec<String>>,
    #[serde(default)]
    kept_permissions: Vec<String>,
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

/// One LuckPerms node as typed: lowercase letters, digits and `._*-`; a
/// single leading `-` means the node is set to false.
fn normalized_node(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let name = raw.strip_prefix('-').unwrap_or(raw);
    let valid = (1..=MAX_PERMISSION_NODE_LENGTH).contains(&name.len())
        && !name.starts_with('-')
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._*-".contains(&b));
    valid.then(|| raw.to_owned())
}

impl Settings {
    fn normalized(self) -> Result<Self> {
        if self.administrators.len() > MAX_ADMINISTRATORS {
            return Err("TooManyTestAdministrators".into());
        }
        let mut seen = Vec::with_capacity(self.administrators.len());
        let mut checked = Vec::with_capacity(self.administrators.len());
        for item in self.administrators {
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
        // The grant content is kept while LuckPerms is off, so it is checked the same way.
        let group = self.group.trim().to_owned();
        if !valid_group(&group) {
            return Err("InvalidTestAdministratorGroup".into());
        }
        let permissions = self.permissions.map(Self::normalized_nodes).transpose()?;
        // Kept nodes only exist beside `*`; choosing custom nodes takes their place.
        let kept_permissions =
            if permissions.is_some() { Vec::new() } else { Self::unique_nodes(&self.kept_permissions)? };
        Ok(Self { administrators: checked, luckperms: self.luckperms, op: self.op, permissions, kept_permissions, group })
    }

    fn normalized_nodes(raw: Vec<String>) -> Result<Vec<String>> {
        let nodes = Self::unique_nodes(&raw)?;
        if nodes.is_empty() {
            return Err("EmptyTestAdministratorPermissions".into());
        }
        Ok(nodes)
    }

    fn unique_nodes(raw: &[String]) -> Result<Vec<String>> {
        let mut nodes: Vec<String> = Vec::with_capacity(raw.len());
        for item in raw.iter().filter(|item| !item.trim().is_empty()) {
            let node = normalized_node(item).ok_or("InvalidTestAdministratorPermission")?;
            if !nodes.contains(&node) {
                nodes.push(node);
            }
        }
        if nodes.len() > MAX_PERMISSION_NODES {
            return Err("TooManyTestAdministratorPermissions".into());
        }
        Ok(nodes)
    }

    /// The feature is on when there is someone to grant and something to grant.
    pub(super) fn enabled(&self) -> bool {
        !self.administrators.is_empty() && (self.luckperms || self.op)
    }

    pub(super) fn count(&self) -> usize {
        self.administrators.len()
    }

    fn permission_entries(&self) -> Vec<Value> {
        match &self.permissions {
            None => vec![json!({"permission":"*","value":true})],
            Some(nodes) => nodes
                .iter()
                .map(|node| match node.strip_prefix('-') {
                    Some(name) => json!({"permission":name,"value":false}),
                    None => json!({"permission":node,"value":true}),
                })
                .collect(),
        }
    }

    fn user_file(&self, administrator: &Administrator) -> Result<Vec<u8>> {
        let mut permissions = vec![json!({"permission":"group.default","value":true})];
        if self.group.is_empty() {
            permissions.extend(self.permission_entries());
        } else {
            permissions.push(json!({"permission":format!("group.{}", self.group),"value":true}));
        }
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
    Ok((stored.normalized()?, content_revision::revision(&bytes)?, true))
}

fn response(root: &Path, settings: &Settings, revision: String) -> Result<Value> {
    Ok(json!({
        "administrators":settings.administrators, "luckperms":settings.luckperms, "op":settings.op,
        "permissions":settings.permissions, "keptPermissions":settings.kept_permissions,
        "group":settings.group, "revision":revision,
        "artifacts":artifacts(root)?,
    }))
}

fn store(root: &Path, raw: &str) -> Result<Value> {
    if raw.len() > MAX_SETTINGS_BYTES {
        return Err("InvalidTestAdministratorSettings".into());
    }
    let edit: Edit = serde_json::from_str(raw).map_err(|_| "InvalidTestAdministratorSettings")?;
    let next = Settings {
        administrators: edit.administrators,
        luckperms: edit.luckperms,
        op: edit.op,
        permissions: edit.permissions,
        kept_permissions: edit.kept_permissions,
        group: edit.group,
    }
    .normalized()?;
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

/// Turns the typed name into the canonical name and online UUID. One request,
/// short timeouts, no retries; the name is validated before anything is sent.
fn lookup(name: &str) -> Result<Value> {
    let name = name.trim();
    if !valid_name(name) {
        return Err("InvalidTestAdministratorName".into());
    }
    let (status, body) = core_source::fetch_profile(&format!("{PROFILE_LOOKUP}{name}"))
        .map_err(|_| "TestAdministratorLookupFailed")?;
    let (canonical, uuid) = parse_lookup(name, status, &body)?;
    Ok(json!({"name":canonical,"uuid":uuid}))
}

fn parse_lookup(requested: &str, status: u16, body: &[u8]) -> Result<(String, String)> {
    match status {
        200 => {}
        204 | 400 | 404 => return Err("TestAdministratorNotFound".into()),
        429 => return Err("TestAdministratorLookupRateLimited".into()),
        _ => return Err("TestAdministratorLookupFailed".into()),
    }
    let profile: Value = serde_json::from_slice(body).map_err(|_| "TestAdministratorLookupFailed")?;
    let name = profile["name"].as_str().filter(|name| valid_name(name) && name.eq_ignore_ascii_case(requested));
    let uuid = profile["id"].as_str().and_then(normalized_uuid);
    match (name, uuid) {
        (Some(name), Some(uuid)) => Ok((name.to_owned(), uuid)),
        _ => Err("TestAdministratorLookupFailed".into()),
    }
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

/// The verified LuckPerms plugin a grant installs.
struct LuckPermsPlan {
    platform: Platform,
    source: PathBuf,
    pin: Pin,
}

/// A verified plan to give one new instance its test administrators.
pub(super) struct Grant {
    luckperms: Option<LuckPermsPlan>,
    op: bool,
    settings: Settings,
    revision: String,
}

impl Grant {
    pub(super) fn summary(&self) -> Value {
        let pin = self.luckperms.as_ref().map(|plan| &plan.pin);
        json!({
            // Kept LuckPerms content does not apply while LuckPerms is off.
            "count":self.settings.administrators.len(),
            "group":if pin.is_some() { self.settings.group.as_str() } else { "" },
            "luckperms":self.luckperms.is_some(), "op":self.op,
            "customPermissions":pin.and(self.settings.permissions.as_ref().map(Vec::len)),
            "fileName":pin.map(|pin| pin.file_name.as_str()), "version":pin.map(|pin| pin.version.as_str()),
            "sha256":pin.map(|pin| pin.sha256.as_str()),
            "settingsRevision":self.revision,
        })
    }
}

/// New instances under `instances/Agent` start without test permissions.
pub(super) fn agent_path(root: &Path, target: &Path) -> bool {
    target.starts_with(root.join("instances").join("Agent"))
}

/// Returns the number of administrators that would be granted and, when
/// requested and configured, the grant. LuckPerms artifacts are only required
/// while the LuckPerms toggle is on; a missing or mismatched cache stops
/// creation then. A proxy has no `ops.json`, so OP alone cannot apply there.
pub(super) fn plan(root: &Path, profile: &CoreProfile, requested: bool) -> Result<(usize, Option<Grant>)> {
    let (settings, revision, _) = load(root)?;
    let configured = if settings.enabled() { settings.administrators.len() } else { 0 };
    if !requested || !settings.enabled() {
        return Ok((configured, None));
    }
    let luckperms = if settings.luckperms {
        let platform = Platform::for_core(profile)?;
        let pin = pin(platform)?;
        let source = match cache_state(root, &pin)? {
            CacheState::Ready(path) => path,
            CacheState::Missing => return Err("TestPermissionArtifactMissing".into()),
            CacheState::Mismatch => return Err("TestPermissionArtifactMismatch".into()),
        };
        Some(LuckPermsPlan { platform, source, pin })
    } else {
        None
    };
    let op = settings.op && profile.family != "VELOCITY";
    if luckperms.is_none() && !op {
        return Err("UnsupportedTestPermissionPlatform".into());
    }
    Ok((configured, Some(Grant { luckperms, op, settings, revision })))
}

/// Copies the verified plugin into a new instance and seeds its data.
pub(super) fn install(directory: &Path, grant: &Grant) -> Result<()> {
    if let Some(plan) = &grant.luckperms {
        let plugins = directory.join("plugins");
        match fs::create_dir(&plugins) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(file_error("create", &plugins, error)),
        }
        let plugins = child_directory(directory, "plugins")?;
        core_maintenance::copy_verified(&plan.source, &plugins.join(&plan.pin.file_name), &plan.pin.sha256)?;
        seed(directory, plan.platform, &grant.settings)?;
    }
    if grant.op {
        seed_ops(directory, &grant.settings)?;
    }
    Ok(())
}

/// Seeds what the settings ask for into a staged Bukkit instance whose
/// LuckPerms JAR is already in place (the `prepare` path).
pub(super) fn seed_prepared(directory: &Path, settings: &Settings) -> Result<()> {
    if settings.luckperms {
        seed(directory, Platform::Bukkit, settings)?;
    }
    if settings.op {
        seed_ops(directory, settings)?;
    }
    Ok(())
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
    let storage = data.join("json-storage");
    let users = storage.join("users");
    for folder in [&data, &storage, &users] {
        create(folder)?;
    }
    write_new(&data.join("config.yml"), SEEDED_CONFIG.as_bytes())?;
    // LuckPerms resolves `lp user <name>` through this cache before a first login.
    let mut lookup = String::from("# LuckPerms UUID lookup cache\n");
    for administrator in &settings.administrators {
        write_new(&users.join(format!("{}.json", administrator.uuid)), &settings.user_file(administrator)?)?;
        lookup.push_str(&format!("{}:{}\n", administrator.uuid, administrator.name));
    }
    write_new(&storage.join("uuidcache.txt"), lookup.as_bytes())?;
    if !settings.group.is_empty() {
        let groups = storage.join("groups");
        create(&groups)?;
        let group = serde_json::to_vec_pretty(&json!({
            "name":settings.group, "permissions":settings.permission_entries(),
        }))
        .map_err(fail)?;
        write_new(&groups.join(format!("{}.json", settings.group)), &group)?;
    }
    Ok(())
}

/// Writes `ops.json` for a new instance. An existing file is never replaced.
pub(super) fn seed_ops(directory: &Path, settings: &Settings) -> Result<()> {
    let path = directory.join("ops.json");
    if fs::symlink_metadata(&path).is_ok() {
        return Err("TestOpDataExists".into());
    }
    let entries: Vec<Value> = settings
        .administrators
        .iter()
        .map(|administrator| {
            json!({"uuid":administrator.uuid,"name":administrator.name,"level":4,"bypassesPlayerLimit":false})
        })
        .collect();
    write_new(&path, &serde_json::to_vec_pretty(&entries).map_err(fail)?)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = File::create_new(path).map_err(|error| file_error("create", path, error))?;
    file.write_all(bytes).and_then(|()| file.sync_all()).map_err(|error| file_error("write", path, error))
}

pub(super) fn dispatch(root: &Path, verb: &str, args: &[String]) -> Option<Result<Value>> {
    Some(match (verb, args) {
        ("test-admin-settings", []) => load(root).and_then(|(settings, revision, _)| response(root, &settings, revision)),
        ("save-test-admin-settings", [raw]) => store(root, raw),
        ("test-admin-fetch-luckperms", []) => fetch(root),
        ("test-admin-lookup", [name]) => lookup(name),
        ("test-admin-settings" | "save-test-admin-settings" | "test-admin-fetch-luckperms" | "test-admin-lookup", _) => {
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

    fn settings(administrators: Vec<Administrator>, group: &str) -> Settings {
        Settings { administrators, group: group.into(), ..Settings::default() }
    }

    fn profile(family: &str, minecraft: &str) -> CoreProfile {
        CoreProfile {
            unverified: false, java: None, family: family.into(), minecraft: minecraft.into(),
            build: "1".into(), sha256: "0".repeat(64),
        }
    }

    pub(in super::super) fn save_fake_administrators(root: &Path, group: &str) {
        save_fake(root, json!({"group":group}));
    }

    /// Saves the two fake administrators with extra fields merged in.
    fn save_fake(root: &Path, extra: Value) {
        let (_, revision, _) = load(root).unwrap();
        let mut raw = json!({
            "administrators":[{"name":"Fake_Admin","uuid":FAKE_UUID},{"name":"OtherFake","uuid":OTHER_UUID}],
            "revision":revision,
        });
        for (key, value) in extra.as_object().unwrap() {
            raw[key] = value.clone();
        }
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
        Grant { luckperms: Some(LuckPermsPlan { platform, source, pin }), op: false, settings, revision }
    }

    /// The same grant with OP switched on as well.
    pub(in super::super) fn with_op(mut grant: Grant) -> Grant {
        grant.op = true;
        grant.settings.op = true;
        grant
    }

    #[test]
    fn settings_normalize_identities_and_reject_unsafe_values() {
        let normalized = settings(
            vec![admin(" Fake_Admin ", &FAKE_UUID.to_uppercase()), admin("OtherFake", OTHER_UUID)],
            "",
        )
        .normalized()
        .unwrap();
        assert_eq!(normalized.administrators[0], admin("Fake_Admin", FAKE_UUID));
        assert_eq!(normalized.administrators[1].uuid, "00000000-0000-4000-8000-000000000002");
        for (items, group, error) in [
            (vec![admin("no", FAKE_UUID)], "", "InvalidTestAdministratorName"),
            (vec![admin("bad-name", FAKE_UUID)], "", "InvalidTestAdministratorName"),
            (vec![admin("Fake_Admin", "not-a-uuid")], "", "InvalidTestAdministratorUuid"),
            (vec![admin("Fake_Admin", FAKE_UUID), admin("OtherFake", FAKE_UUID)], "", "DuplicateTestAdministrator"),
            (vec![], "default", "InvalidTestAdministratorGroup"),
            (vec![], "Admins", "InvalidTestAdministratorGroup"),
            (vec![admin("Fake_Admin", FAKE_UUID); MAX_ADMINISTRATORS + 1], "", "TooManyTestAdministrators"),
        ] {
            assert_eq!(settings(items, group).normalized().unwrap_err(), error);
        }
    }

    #[test]
    fn custom_permission_nodes_are_validated_and_negation_is_kept() {
        let nodes = |lines: &[&str]| lines.iter().map(|line| line.to_string()).collect::<Vec<_>>();
        let custom = |lines: &[&str]| Settings { permissions: Some(nodes(lines)), ..Settings::default() }.normalized();
        let ok = custom(&[" essentials.fly ", "", "-worldedit.*", "essentials.fly"]).unwrap();
        assert_eq!(ok.permissions, Some(nodes(&["essentials.fly", "-worldedit.*"])));
        for (lines, error) in [
            (vec![], "EmptyTestAdministratorPermissions"),
            (vec!["", "  "], "EmptyTestAdministratorPermissions"),
            (vec!["Essentials.Fly"], "InvalidTestAdministratorPermission"),
            (vec!["has space"], "InvalidTestAdministratorPermission"),
            (vec!["--double"], "InvalidTestAdministratorPermission"),
            (vec!["-"], "InvalidTestAdministratorPermission"),
            (vec!["semi;colon"], "InvalidTestAdministratorPermission"),
        ] {
            assert_eq!(custom(&lines).unwrap_err(), error, "{lines:?}");
        }
        let long = "a".repeat(MAX_PERMISSION_NODE_LENGTH + 1);
        assert_eq!(custom(&[&long]).unwrap_err(), "InvalidTestAdministratorPermission");
        let many: Vec<String> = (0..=MAX_PERMISSION_NODES).map(|index| format!("plugin.node{index}")).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        assert_eq!(custom(&refs).unwrap_err(), "TooManyTestAdministratorPermissions");
    }

    #[test]
    fn luckperms_content_is_kept_and_checked_while_luckperms_is_off() {
        let off = |group: &str, permissions: Option<Vec<String>>| {
            Settings { luckperms: false, group: group.into(), permissions, ..Settings::default() }.normalized()
        };
        let kept = off(" staff ", Some(vec!["essentials.fly".into()])).unwrap();
        assert_eq!((kept.group.as_str(), kept.permissions), ("staff", Some(vec!["essentials.fly".into()])));
        assert_eq!(off("default", None).unwrap_err(), "InvalidTestAdministratorGroup");
        // Nodes kept beside `*` survive a save but are never granted.
        let all = Settings { kept_permissions: vec!["a.b".into(), "a.b".into(), " ".into()], ..Settings::default() }
            .normalized()
            .unwrap();
        assert_eq!((all.permissions.clone(), all.kept_permissions.clone()), (None, vec!["a.b".to_owned()]));
        assert_eq!(all.permission_entries(), vec![json!({"permission":"*","value":true})]);
        let custom = Settings { permissions: Some(vec!["c.d".into()]), kept_permissions: vec!["a.b".into()], ..all }
            .normalized()
            .unwrap();
        assert!(custom.kept_permissions.is_empty());
        let bad = Settings { kept_permissions: vec!["Bad".into()], ..Settings::default() }.normalized();
        assert_eq!(bad.unwrap_err(), "InvalidTestAdministratorPermission");
        assert_eq!(off("", Some(vec![])).unwrap_err(), "EmptyTestAdministratorPermissions");
    }

    #[test]
    fn old_settings_files_load_as_luckperms_all_permissions_without_op() {
        let workspace = Workspace::new();
        let root = &workspace.0;
        let old = json!({"administrators":[{"name":"Fake_Admin","uuid":FAKE_UUID}],"group":"test-admin"});
        fs::write(root.join(SETTINGS_FILE), old.to_string()).unwrap();
        let (loaded, _, exists) = load(root).unwrap();
        assert!(exists && loaded.luckperms && !loaded.op && loaded.permissions.is_none());
        assert_eq!(loaded.group, "test-admin");
        let (empty, _, _) = load(&Workspace::new().0).unwrap();
        assert!(empty.luckperms && !empty.op && !empty.enabled());
    }

    #[test]
    fn settings_round_trip_with_revision_conflicts() {
        let workspace = Workspace::new();
        let root = &workspace.0;
        let (empty, first, exists) = load(root).unwrap();
        assert!(!empty.enabled() && !exists);
        let saved = store(root, &json!({"administrators":[{"name":"Fake_Admin","uuid":FAKE_UUID}],"revision":first}).to_string()).unwrap();
        assert_eq!(saved["administrators"][0]["uuid"], FAKE_UUID);
        assert_eq!((&saved["luckperms"], &saved["op"], &saved["permissions"]), (&json!(true), &json!(false), &Value::Null));
        assert_eq!(saved["artifacts"].as_array().unwrap().len(), 3);
        assert!(saved["artifacts"].as_array().unwrap().iter().all(|row| row["state"] == "Missing"));
        assert_eq!(
            store(root, &json!({"administrators":[],"revision":first}).to_string()).unwrap_err(),
            "TestAdministratorSettingsConflict"
        );
        let both = store(root, &json!({
            "administrators":[{"name":"Fake_Admin","uuid":FAKE_UUID}], "luckperms":true, "op":true,
            "permissions":["essentials.fly","-worldedit.*"], "group":"test-admin", "revision":saved["revision"],
        }).to_string()).unwrap();
        assert_eq!(both["permissions"], json!(["essentials.fly","-worldedit.*"]));
        assert_eq!((&both["op"], &both["group"]), (&json!(true), &json!("test-admin")));
        let cleared = store(root, &json!({"administrators":[],"revision":both["revision"]}).to_string()).unwrap();
        assert_eq!(cleared["administrators"], json!([]));
        assert!(!load(root).unwrap().0.enabled());
    }

    #[test]
    fn the_feature_needs_an_administrator_and_a_toggle() {
        let on = |luckperms, op| Settings { luckperms, op, ..settings(vec![admin("Fake_Admin", FAKE_UUID)], "") };
        assert!(on(true, false).enabled() && on(false, true).enabled() && on(true, true).enabled());
        assert!(!on(false, false).enabled());
        assert!(!Settings { op: true, ..Settings::default() }.enabled(), "nobody to grant");
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
    fn op_only_needs_no_luckperms_artifact_and_neither_toggle_grants_nothing() {
        let workspace = Workspace::new();
        let root = &workspace.0;
        let paper = profile("PAPER", "26.3");
        save_fake(root, json!({"luckperms":false,"op":true}));
        let (count, grant) = plan(root, &paper, true).unwrap();
        let grant = grant.expect("OP alone is a grant without any LuckPerms file");
        assert_eq!(count, 2);
        assert!(grant.luckperms.is_none() && grant.op);
        let summary = grant.summary();
        assert_eq!((&summary["luckperms"], &summary["op"], &summary["fileName"]), (&json!(false), &json!(true), &Value::Null));
        assert!(plan(root, &profile("FORGE", "1.20.1"), true).unwrap().1.unwrap().op);
        assert_eq!(plan(root, &profile("VELOCITY", "3.4.0"), true).err().unwrap(), "UnsupportedTestPermissionPlatform");
        save_fake(root, json!({"luckperms":false,"op":false}));
        assert_eq!(plan(root, &paper, true).unwrap().0, 0);
        assert!(plan(root, &paper, true).unwrap().1.is_none());
        save_fake(root, json!({"luckperms":true,"op":true}));
        assert_eq!(plan(root, &paper, true).err().unwrap(), "TestPermissionArtifactMissing");
    }

    #[test]
    fn seeding_writes_json_storage_and_never_replaces_existing_data() {
        let workspace = Workspace::new();
        let directory = workspace.0.join("instances").join("seeded");
        fs::create_dir_all(directory.join("plugins")).unwrap();
        let settings = settings(vec![admin("Fake_Admin", FAKE_UUID), admin("OtherFake", OTHER_UUID)], "")
            .normalized()
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
            format!("# LuckPerms UUID lookup cache\n{FAKE_UUID}:Fake_Admin\n00000000-0000-4000-8000-000000000002:OtherFake\n")
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
        let settings = settings(vec![admin("Fake_Admin", FAKE_UUID)], "test-admin").normalized().unwrap();
        seed(&directory, Platform::Velocity, &settings).unwrap();
        let storage = directory.join("plugins").join("luckperms").join("json-storage");
        let user = fs::read_to_string(storage.join("users").join(format!("{FAKE_UUID}.json"))).unwrap();
        assert!(user.contains("group.test-admin") && !user.contains("\"*\""));
        let group: Value = serde_json::from_slice(&fs::read(storage.join("groups").join("test-admin.json")).unwrap()).unwrap();
        assert_eq!(group["permissions"], json!([{"permission":"*","value":true}]));
    }

    #[test]
    fn custom_nodes_go_on_the_user_or_on_the_group() {
        let workspace = Workspace::new();
        let nodes = Some(vec!["essentials.fly".to_owned(), "-worldedit.*".to_owned()]);
        let expected = json!([
            {"permission":"essentials.fly","value":true},
            {"permission":"worldedit.*","value":false},
        ]);
        let direct = workspace.0.join("instances").join("direct");
        fs::create_dir_all(direct.join("plugins")).unwrap();
        let direct_settings = Settings { permissions: nodes.clone(), ..settings(vec![admin("Fake_Admin", FAKE_UUID)], "") };
        seed(&direct, Platform::Bukkit, &direct_settings).unwrap();
        let storage = direct.join("plugins").join("LuckPerms").join("json-storage");
        let user: Value = serde_json::from_slice(&fs::read(storage.join("users").join(format!("{FAKE_UUID}.json"))).unwrap()).unwrap();
        let permissions = user["permissions"].as_array().unwrap();
        assert_eq!(permissions[0], json!({"permission":"group.default","value":true}));
        assert_eq!(json!(permissions[1..].to_vec()), expected);
        assert!(!storage.join("groups").exists());

        let grouped = workspace.0.join("instances").join("grouped");
        fs::create_dir_all(grouped.join("plugins")).unwrap();
        let grouped_settings = Settings { permissions: nodes, ..settings(vec![admin("Fake_Admin", FAKE_UUID)], "test-admin") };
        seed(&grouped, Platform::Bukkit, &grouped_settings).unwrap();
        let storage = grouped.join("plugins").join("LuckPerms").join("json-storage");
        let user: Value = serde_json::from_slice(&fs::read(storage.join("users").join(format!("{FAKE_UUID}.json"))).unwrap()).unwrap();
        assert_eq!(user["permissions"][1], json!({"permission":"group.test-admin","value":true}));
        assert_eq!(user["permissions"].as_array().unwrap().len(), 2);
        let group: Value = serde_json::from_slice(&fs::read(storage.join("groups").join("test-admin.json")).unwrap()).unwrap();
        assert_eq!(group["permissions"], expected);
    }

    #[test]
    fn ops_json_lists_each_administrator_at_level_four_and_is_never_replaced() {
        let workspace = Workspace::new();
        let directory = workspace.0.join("instances").join("ops");
        fs::create_dir_all(&directory).unwrap();
        let settings = Settings { op: true, ..settings(vec![admin("Fake_Admin", FAKE_UUID), admin("OtherFake", OTHER_UUID)], "") }
            .normalized()
            .unwrap();
        seed_ops(&directory, &settings).unwrap();
        let ops: Value = serde_json::from_slice(&fs::read(directory.join("ops.json")).unwrap()).unwrap();
        assert_eq!(ops, json!([
            {"uuid":FAKE_UUID,"name":"Fake_Admin","level":4,"bypassesPlayerLimit":false},
            {"uuid":"00000000-0000-4000-8000-000000000002","name":"OtherFake","level":4,"bypassesPlayerLimit":false},
        ]));
        assert_eq!(seed_ops(&directory, &settings).unwrap_err(), "TestOpDataExists");
        assert!(!directory.join("plugins").exists(), "OP never installs LuckPerms");
    }

    #[test]
    fn prepared_seeding_follows_the_toggles() {
        let workspace = Workspace::new();
        let make = |name: &str, luckperms, op| {
            let directory = workspace.0.join("instances").join(name);
            fs::create_dir_all(directory.join("plugins")).unwrap();
            let settings = Settings { luckperms, op, ..settings(vec![admin("Fake_Admin", FAKE_UUID)], "") };
            seed_prepared(&directory, &settings).unwrap();
            directory
        };
        let lp_only = make("lp", true, false);
        assert!(lp_only.join("plugins").join("LuckPerms").exists() && !lp_only.join("ops.json").exists());
        let op_only = make("op", false, true);
        assert!(!op_only.join("plugins").join("LuckPerms").exists() && op_only.join("ops.json").exists());
        let both = make("both", true, true);
        assert!(both.join("plugins").join("LuckPerms").exists() && both.join("ops.json").exists());
    }

    #[test]
    fn profile_lookup_parses_canonical_name_and_dashes_the_uuid() {
        let body = br#"{"id":"00000000000040008000000000000001","name":"Fake_Admin"}"#;
        assert_eq!(
            parse_lookup("fake_admin", 200, body).unwrap(),
            ("Fake_Admin".to_owned(), FAKE_UUID.to_owned())
        );
        for (status, error) in [
            (404, "TestAdministratorNotFound"),
            (204, "TestAdministratorNotFound"),
            (400, "TestAdministratorNotFound"),
            (429, "TestAdministratorLookupRateLimited"),
            (500, "TestAdministratorLookupFailed"),
            (503, "TestAdministratorLookupFailed"),
        ] {
            assert_eq!(parse_lookup("Fake_Admin", status, b"{}").unwrap_err(), error, "{status}");
        }
        for bad in [
            &b"not json"[..],
            br#"{"id":"short","name":"Fake_Admin"}"#,
            br#"{"id":"00000000000040008000000000000001","name":"Someone_Else"}"#,
            br#"{"id":"00000000000040008000000000000001"}"#,
        ] {
            assert_eq!(parse_lookup("Fake_Admin", 200, bad).unwrap_err(), "TestAdministratorLookupFailed");
        }
    }

    /// Needs the network: `cargo test -- --ignored live_mojang_lookup`.
    #[test]
    #[ignore]
    fn live_mojang_lookup() {
        let found = lookup("jeb_").unwrap();
        assert_eq!(found["name"], "jeb_");
        assert!(normalized_uuid(found["uuid"].as_str().unwrap()).is_some());
        assert_eq!(lookup("zz_no_such_nm_1").unwrap_err(), "TestAdministratorNotFound");
    }

    #[test]
    fn lookup_rejects_invalid_names_before_any_request() {
        for name in ["", "ab", "has space", "bad-name", "../x", "a_name_that_is_too_long"] {
            assert_eq!(lookup(name).unwrap_err(), "InvalidTestAdministratorName", "{name:?}");
        }
        let workspace = Workspace::new();
        let args = vec!["x y".to_owned()];
        assert_eq!(dispatch(&workspace.0, "test-admin-lookup", &args).unwrap().unwrap_err(), "InvalidTestAdministratorName");
        assert_eq!(dispatch(&workspace.0, "test-admin-lookup", &[]).unwrap().unwrap_err(), "InvalidTestAdministratorArguments");
    }

    #[test]
    fn only_paths_under_the_agent_folder_default_to_no_grant() {
        let root = Path::new(r"C:\workspace");
        assert!(agent_path(root, &root.join("instances").join("Agent").join("Lobby").join("a")));
        assert!(!agent_path(root, &root.join("instances").join("paper")));
        assert!(!agent_path(root, &root.join("instances").join("Agents").join("a")));
    }
}
