//! Checks GitHub Releases for a newer InstanceHarbor (#65).
//!
//! Only the GUI asks; the Manager never checks. One check sends one
//! `GET /repos/Command1264/InstanceHarbor/releases?per_page=10` through the system
//! `curl.exe` (see `core_source::fetch_status`) and nothing about the user or the
//! workspace. The settings and the last result live in the workspace's
//! `app-update.json`, which older versions do not read, so a rollback keeps working.
//!
//! A download goes to `staging/updates/<version>/` (at most one version is kept) and
//! must match the size and the SHA-256 `digest` GitHub publishes. The Setup build then
//! starts that Setup, which closes InstanceHarbor and waits for servers (#61); the
//! portable build only opens the folder. Versions not newer than the running one are
//! deleted when the GUI starts.
use super::*;
use crate::app_version::Version;

const RELEASES_URL: &str = "https://api.github.com/repos/Command1264/InstanceHarbor/releases?per_page=10";
const DOWNLOAD_PREFIX: &str = "https://github.com/Command1264/InstanceHarbor/releases/download/";
const STATE_FILE: &str = "app-update.json";
const LOCK_FILE: &str = "app-update.lock";
const MAX_STATE_BYTES: u64 = 1 << 20;
/// Automatic checks run at GUI start at most this often.
const AUTO_INTERVAL: u64 = 24 * 60 * 60;
const MAX_RELEASES: usize = 10;
const MAX_ASSETS: usize = 32;
const MAX_ASSET_BYTES: u64 = 512 * 1024 * 1024;
/// Development builds only: read the release list from this JSON file instead of GitHub.
#[cfg(feature = "development-channel")]
const DEV_RELEASES_ENV: &str = "IH_DEV_UPDATE_RELEASES";
/// Development builds only: copy release files from this folder instead of downloading.
#[cfg(feature = "development-channel")]
const DEV_ASSETS_ENV: &str = "IH_DEV_UPDATE_ASSETS";
const UPDATES_FOLDER: &str = "updates";
/// Held while a download runs; a second download is refused, not queued.
const DOWNLOAD_LOCK_FILE: &str = "app-update-download.lock";
const FILE_SHARE_READ: u32 = 1;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const REPARSE_POINT: u32 = 0x400;

/// Release asset names carry `<platform>-<architecture>`. #66 adds rows here.
const PLATFORMS: &[(&str, &str, &str)] = &[("windows", "x86_64", "windows-x64")];

fn platform_label(os: &str, arch: &str) -> Option<&'static str> {
    PLATFORMS.iter().find(|(o, a, _)| *o == os && *a == arch).map(|(_, _, label)| *label)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Channel {
    #[default]
    Stable,
    /// Prereleases and releases.
    Prerelease,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InstallKind {
    /// Setup wrote `instanceharbor.install` beside the executable.
    Setup,
    Portable,
}

impl InstallKind {
    fn current() -> Self {
        let marker = std::env::current_exe().ok().map(|exe| exe.with_file_name("instanceharbor.install"));
        match marker.and_then(|path| fs::symlink_metadata(path).ok()) {
            Some(meta) if meta.is_file() => Self::Setup,
            _ => Self::Portable,
        }
    }

    fn suffix(self) -> &'static str {
        match self {
            Self::Setup => "-setup.exe",
            Self::Portable => ".zip",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Setup => "setup",
            Self::Portable => "portable",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Asset {
    name: String,
    url: String,
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Release {
    tag: String,
    prerelease: bool,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    assets: Vec<Asset>,
}

fn yes() -> bool {
    true
}

/// `app-update.json`. Unknown fields are kept out of the struct but tolerated, so a
/// newer version's file still loads after a rollback.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct State {
    #[serde(default)]
    channel: Channel,
    #[serde(default = "yes")]
    auto_check: bool,
    /// Unix seconds of the last check that was started, successful or not.
    #[serde(default)]
    last_attempt: Option<u64>,
    #[serde(default)]
    last_success: Option<u64>,
    /// The reason code of the last check, when it failed.
    #[serde(default)]
    last_error: Option<String>,
    /// The releases the last successful check read, so the result survives a restart
    /// and a channel change without asking GitHub again.
    #[serde(default)]
    releases: Vec<Release>,
}

impl Default for State {
    fn default() -> Self {
        Self { channel: Channel::Stable, auto_check: true, last_attempt: None, last_success: None, last_error: None, releases: vec![] }
    }
}

struct Context {
    now: u64,
    version: Version,
    platform: Option<&'static str>,
    kind: InstallKind,
}

impl Context {
    fn current() -> Result<Self> {
        Ok(Self {
            now: SystemTime::now().duration_since(UNIX_EPOCH).map_err(fail)?.as_secs(),
            version: Version::parse(crate::about::VERSION).ok_or("InvalidApplicationVersion")?,
            platform: platform_label(std::env::consts::OS, std::env::consts::ARCH),
            kind: InstallKind::current(),
        })
    }

    fn platform_text(&self) -> String {
        self.platform.map(str::to_owned).unwrap_or_else(|| format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH))
    }
}

fn state_path(root: &Path) -> PathBuf {
    root.join(STATE_FILE)
}

/// Any unreadable file means the defaults: it holds two settings and a cache.
fn load(root: &Path) -> State {
    use std::os::windows::fs::MetadataExt;
    let path = state_path(root);
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_file() && meta.file_attributes() & 0x400 == 0 && meta.len() <= MAX_STATE_BYTES => {}
        _ => return State::default(),
    }
    fs::read(&path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

fn store(root: &Path, state: &State) -> Result<()> {
    let path = state_path(root);
    if path.exists() {
        no_redirect(&path)?;
    }
    policy_settings::replace_file(&path, &serde_json::to_vec_pretty(state).map_err(fail)?)
}

/// Serializes read-modify-write of `app-update.json` within this workspace.
fn lock(root: &Path) -> Result<File> {
    let path = root.join(LOCK_FILE);
    if path.exists() {
        no_redirect(&path)?;
    }
    let file = OpenOptions::new().create(true).truncate(false).write(true).open(&path)
        .map_err(|error| file_error("open-update-lock", &path, error))?;
    file.lock().map_err(fail)?;
    Ok(file)
}

fn update(root: &Path, change: impl FnOnce(&mut State)) -> Result<State> {
    let _lock = lock(root)?;
    let mut state = load(root);
    change(&mut state);
    store(root, &state)?;
    Ok(state)
}

fn sha256_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

/// Reads the GitHub release list. Drafts and tags that are not versions are skipped.
fn parse_releases(body: &[u8]) -> Result<Vec<Release>> {
    let value: Value = serde_json::from_slice(body).map_err(|_| "UpdateCheckInvalidResponse")?;
    let list = value.as_array().ok_or("UpdateCheckInvalidResponse")?;
    let mut releases = vec![];
    for item in list.iter().take(MAX_RELEASES) {
        if item["draft"].as_bool() != Some(false) {
            continue;
        }
        let (Some(tag), Some(prerelease)) = (item["tag_name"].as_str(), item["prerelease"].as_bool()) else {
            continue;
        };
        if Version::parse(tag).is_none() {
            continue;
        }
        let assets = item["assets"].as_array().map(Vec::as_slice).unwrap_or_default().iter().take(MAX_ASSETS)
            .filter_map(|asset| {
                Some(Asset {
                    name: asset["name"].as_str()?.chars().take(256).collect(),
                    url: asset["browser_download_url"].as_str()?.chars().take(512).collect(),
                    size: asset["size"].as_u64()?,
                    digest: asset["digest"].as_str().filter(|digest| digest.len() <= 80).map(str::to_owned),
                })
            })
            .collect();
        releases.push(Release {
            tag: tag.to_owned(),
            prerelease,
            published_at: item["published_at"].as_str().filter(|text| text.len() <= 40).map(str::to_owned),
            assets,
        });
    }
    Ok(releases)
}

/// What the status reports about the newest release that is newer than this build.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct Available {
    version: String,
    tag: String,
    prerelease: bool,
    published_at: Option<String>,
    /// The file for this platform, architecture and install kind; None when the
    /// release has none (`problem` says why).
    asset: Option<Asset>,
    /// `NoMatchingAsset`, or `UntrustedAsset` when the file is there but its address
    /// or SHA-256 is not what a GitHub release publishes.
    problem: Option<&'static str>,
}

/// The newest release newer than `context.version` in `channel`. Never a downgrade:
/// an older or equal release is never offered, whatever the channel.
fn pick(releases: &[Release], channel: Channel, context: &Context) -> Option<Available> {
    let (release, version) = releases
        .iter()
        .filter(|release| channel == Channel::Prerelease || !release.prerelease)
        .filter_map(|release| Some((release, Version::parse(&release.tag)?)))
        // A release marked stable with a prerelease number is still a prerelease.
        .filter(|(_, version)| channel == Channel::Prerelease || !version.is_prerelease())
        .filter(|(_, version)| *version > context.version)
        .max_by(|(_, a), (_, b)| a.cmp(b))?;
    let shown = version.to_string();
    let found = context.platform.and_then(|platform| {
        let name = format!("InstanceHarbor-{shown}-{platform}{}", context.kind.suffix());
        release.assets.iter().find(|asset| asset.name == name)
    });
    let (asset, problem) = match found {
        None => (None, Some("NoMatchingAsset")),
        Some(asset) if asset.url == format!("{DOWNLOAD_PREFIX}{}/{}", release.tag, asset.name)
            && asset.digest.as_deref().is_some_and(sha256_digest)
            && (1..=MAX_ASSET_BYTES).contains(&asset.size) => (Some(asset.clone()), None),
        Some(_) => (None, Some("UntrustedAsset")),
    };
    Some(Available {
        version: shown,
        tag: release.tag.clone(),
        prerelease: release.prerelease || version.is_prerelease(),
        published_at: release.published_at.clone(),
        asset,
        problem,
    })
}

fn due(state: &State, now: u64) -> bool {
    state.auto_check
        && state.last_attempt.is_none_or(|at| now < at || now - at >= AUTO_INTERVAL)
}

/// `staging/updates`, created when `create`; None when absent and not created.
fn updates_folder(root: &Path, create: bool) -> Result<Option<PathBuf>> {
    let staging = child_directory(root, "staging")?;
    let path = staging.join(UPDATES_FOLDER);
    if !path.exists() {
        if !create {
            return Ok(None);
        }
        fs::create_dir(&path).map_err(|error| file_error("create-update-folder", &path, error))?;
    }
    child_directory(&staging, UPDATES_FOLDER).map(Some)
}

fn plain(path: &Path, directory: bool) -> bool {
    use std::os::windows::fs::MetadataExt;
    fs::symlink_metadata(path).is_ok_and(|meta| {
        meta.file_attributes() & REPARSE_POINT == 0 && if directory { meta.is_dir() } else { meta.is_file() }
    })
}

/// The downloaded file of `available`, when it is there.
fn downloaded(root: &Path, available: &Available) -> Option<PathBuf> {
    let asset = available.asset.as_ref()?;
    let path = updates_folder(root, false).ok()??.join(&available.version).join(&asset.name);
    plain(&path, false).then_some(path)
}

/// Deletes the version folders in `staging/updates` that `remove` selects. Other
/// names, files and links are left alone.
fn remove_versions(updates: &Path, remove: impl Fn(&Version) -> bool) -> Result<Vec<String>> {
    let mut removed = vec![];
    for entry in fs::read_dir(updates).map_err(|error| file_error("list-update-folder", updates, error))? {
        let path = entry.map_err(fail)?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()).map(str::to_owned) else { continue };
        let Some(version) = Version::parse(&name).filter(|version| version.to_string() == name) else { continue };
        if remove(&version) && plain(&path, true) {
            fs::remove_dir_all(&path).map_err(|error| file_error("remove-update", &path, error))?;
            removed.push(name);
        }
    }
    Ok(removed)
}

fn status(root: &Path, state: &State, context: &Context) -> Value {
    let update = pick(&state.releases, state.channel, context);
    let download = update.as_ref().filter(|available| downloaded(root, available).is_some())
        .map(|available| json!({"version": available.version, "name": available.asset.as_ref().map(|asset| &asset.name)}));
    json!({
        "currentVersion": context.version.to_string(),
        "channel": state.channel,
        "autoCheck": state.auto_check,
        "platform": context.platform_text(),
        "install": context.kind.name(),
        "lastAttempt": state.last_attempt,
        "lastSuccess": state.last_success,
        "lastError": state.last_error,
        "due": due(state, context.now),
        "update": update,
        "download": download,
    })
}

/// Maps the HTTP status and transport errors to the reasons the GUI explains.
fn releases_from(response: Result<(u16, Vec<u8>)>) -> Result<Vec<Release>> {
    match response {
        Ok((200, body)) => parse_releases(&body),
        Ok((403 | 429, _)) => Err("UpdateCheckRateLimited".into()),
        Ok((404, _)) => Err("UpdateCheckNotFound".into()),
        Ok(_) => Err("UpdateCheckHttpError".into()),
        Err(error) => Err(match error.as_str() {
            "CoreSourceTimeout" => "UpdateCheckTimeout",
            "CoreSourceTransportError" => "UpdateCheckOffline",
            "CoreSourceResponseTooLarge" => "UpdateCheckInvalidResponse",
            "CoreSystemCurlUnavailable" | "CoreSystemDirectoryUnavailable" | "CoreSourceLaunchFailed" => "UpdateCheckUnavailable",
            _ => "UpdateCheckFailed",
        }
        .into()),
    }
}

fn fetch() -> Result<(u16, Vec<u8>)> {
    #[cfg(feature = "development-channel")]
    if let Some(path) = std::env::var_os(DEV_RELEASES_ENV) {
        return fs::read(path).map(|body| (200, body)).map_err(|_| "CoreSourceTransportError".into());
    }
    core_source::fetch_status(RELEASES_URL, &format!("InstanceHarbor/{}", crate::about::VERSION), false)
}

/// `automatic` checks only when due, so restarting the GUI does not ask again.
fn check(root: &Path, automatic: bool, context: &Context, fetch: impl FnOnce() -> Result<(u16, Vec<u8>)>) -> Result<Value> {
    let state = load(root);
    if automatic && !due(&state, context.now) {
        return Ok(status(root, &state, context));
    }
    // Recorded first: a GUI that ends during the request does not retry at once.
    update(root, |state| state.last_attempt = Some(context.now))?;
    let result = releases_from(fetch());
    let state = update(root, |state| match result {
        Ok(releases) => {
            state.releases = releases;
            state.last_success = Some(context.now);
            state.last_error = None;
        }
        // The last good result stays; only the reason is shown.
        Err(error) => state.last_error = Some(error),
    })?;
    Ok(status(root, &state, context))
}

fn settings(root: &Path, channel: &str, auto_check: &str, context: &Context) -> Result<Value> {
    let channel = match channel {
        "stable" => Channel::Stable,
        "prerelease" => Channel::Prerelease,
        _ => return Err("InvalidUpdateChannel".into()),
    };
    let auto_check = match auto_check {
        "true" => true,
        "false" => false,
        _ => return Err("InvalidUpdateAutoCheck".into()),
    };
    let state = update(root, |state| {
        state.channel = channel;
        state.auto_check = auto_check;
    })?;
    Ok(status(root, &state, context))
}

/// The update `tag` from the last check, with a file this build can use.
fn wanted(root: &Path, tag: &str, context: &Context) -> Result<(Available, Asset)> {
    let state = load(root);
    let available = pick(&state.releases, state.channel, context)
        .filter(|available| available.tag == tag)
        .ok_or("UpdateNotAvailable")?;
    let asset = available.asset.clone().ok_or("UpdateAssetUnavailable")?;
    Ok((available, asset))
}

/// Opens `path` so nobody can change it while it is held, and checks its size and
/// SHA-256 against what GitHub published. The caller keeps the handle while it
/// uses the file, so the bytes checked are the bytes used.
fn verified(path: &Path, asset: &Asset) -> Result<File> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    let mut file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|error| file_error("open-update", path, error))?;
    let meta = file.metadata().map_err(fail)?;
    if !meta.is_file() || meta.file_attributes() & REPARSE_POINT != 0 {
        return Err("UpdateDownloadInvalid".into());
    }
    if meta.len() != asset.size {
        return Err("UpdateDownloadSizeMismatch".into());
    }
    let expected = asset.digest.as_deref().and_then(|digest| digest.strip_prefix("sha256:")).ok_or("UpdateAssetUnavailable")?;
    if content_revision::open_file_revision(&mut file, path)? != expected {
        return Err("UpdateDownloadDigestMismatch".into());
    }
    Ok(file)
}

/// Downloads with the system curl. The address is the GitHub download URL that
/// `pick` accepted; GitHub redirects it to its file host, over HTTPS only.
fn transfer(asset: &Asset, output: &Path) -> Result<()> {
    #[cfg(feature = "development-channel")]
    if let Some(folder) = std::env::var_os(DEV_ASSETS_ENV) {
        return fs::copy(Path::new(&folder).join(&asset.name), output).map(drop).map_err(|_| "UpdateDownloadFailed".into());
    }
    let reply = Command::new(core_source::system_curl().map_err(|_| "UpdateCheckUnavailable")?)
        .args(["--disable", "--silent", "--fail", "--globoff", "--location", "--max-redirs", "5",
            "--proto", "=https", "--proto-redir", "=https", "--tlsv1.2",
            "--connect-timeout", "10", "--max-time", "900"])
        .arg("--max-filesize").arg(asset.size.to_string())
        .arg("--user-agent").arg(format!("InstanceHarbor/{}", crate::about::VERSION))
        .args(["--write-out", "%{http_code}", "--output"]).arg(output)
        .arg("--url").arg(&asset.url)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .env_remove("SSLKEYLOGFILE")
        .env_remove("CURL_CA_BUNDLE")
        .env_remove("SSL_CERT_FILE")
        .env_remove("SSL_CERT_DIR")
        .creation_flags(HIDDEN)
        .output()
        .map_err(|_| "UpdateCheckUnavailable")?;
    match reply.status.code() {
        Some(0) if reply.stdout == b"200" => Ok(()),
        Some(28) => Err("UpdateDownloadTimeout".into()),
        Some(63) => Err("UpdateDownloadSizeMismatch".into()),
        Some(6 | 7) => Err("UpdateCheckOffline".into()),
        _ => Err("UpdateDownloadFailed".into()),
    }
}

fn download(root: &Path, tag: &str, context: &Context, transfer: impl FnOnce(&Asset, &Path) -> Result<()>) -> Result<Value> {
    let lock_path = root.join(DOWNLOAD_LOCK_FILE);
    if lock_path.exists() {
        no_redirect(&lock_path)?;
    }
    let busy = OpenOptions::new().create(true).truncate(false).write(true).open(&lock_path)
        .map_err(|error| file_error("open-update-lock", &lock_path, error))?;
    busy.try_lock().map_err(|_| "UpdateDownloadBusy")?;
    let (available, asset) = wanted(root, tag, context)?;
    let updates = updates_folder(root, true)?.ok_or("UpdateFolderUnavailable")?;
    // At most one version is kept.
    remove_versions(&updates, |version| version.to_string() != available.version)?;
    let folder = updates.join(&available.version);
    if !folder.exists() {
        fs::create_dir(&folder).map_err(|error| file_error("create-update-folder", &folder, error))?;
    }
    let folder = child_directory(&updates, &available.version)?;
    let target = folder.join(&asset.name);
    if plain(&target, false) && verified(&target, &asset).is_ok() {
        return Ok(status(root, &load(root), context));
    }
    let part = folder.join(format!("{}.part", asset.name));
    for stale in [&target, &part] {
        if stale.exists() {
            no_redirect(stale)?;
            fs::remove_file(stale).map_err(|error| file_error("remove-update", stale, error))?;
        }
    }
    let checked = transfer(&asset, &part).and_then(|()| verified(&part, &asset).map(drop));
    if let Err(error) = checked {
        // A file that does not match is never kept.
        let _ = fs::remove_file(&part);
        return Err(error);
    }
    fs::rename(&part, &target).map_err(|error| file_error("publish-update", &target, error))?;
    Ok(status(root, &load(root), context))
}

/// The verified download of `tag`, held open. A file that no longer matches is deleted.
fn verified_download(root: &Path, tag: &str, context: &Context) -> Result<(PathBuf, File)> {
    let (available, asset) = wanted(root, tag, context)?;
    let path = downloaded(root, &available).ok_or("UpdateDownloadMissing")?;
    match verified(&path, &asset) {
        Ok(file) => Ok((path, file)),
        Err(error) => {
            if matches!(error.as_str(), "UpdateDownloadDigestMismatch" | "UpdateDownloadSizeMismatch") {
                fs::remove_file(&path).map_err(|cleanup| format!("{error}; {}", file_error("remove-update", &path, cleanup)))?;
            }
            Err(error)
        }
    }
}

/// Starts the downloaded Setup after the user confirmed. Setup closes the GUI and the
/// Manager and waits while servers run (#61); this build only starts it.
fn install(root: &Path, tag: &str, context: &Context, launch: impl FnOnce(&Path) -> Result<u32>) -> Result<Value> {
    if context.kind != InstallKind::Setup {
        return Err("UpdateInstallNeedsSetup".into());
    }
    let (path, held) = verified_download(root, tag, context)?;
    let pid = launch(&path)?;
    drop(held);
    Ok(json!({"started": true, "pid": pid}))
}

fn launch_setup(path: &Path) -> Result<u32> {
    Command::new(path)
        .current_dir(path.parent().ok_or("UpdateFolderUnavailable")?)
        .stdin(Stdio::null())
        .spawn()
        .map(|child| child.id())
        .map_err(|error| format!("UpdateInstallFailed: {error}"))
}

/// The portable build never replaces running files: it shows the verified ZIP.
fn reveal(root: &Path, tag: &str, context: &Context, open: impl FnOnce(&Path) -> Result<()>) -> Result<Value> {
    if context.kind != InstallKind::Portable {
        return Err("UpdateRevealNeedsPortable".into());
    }
    let (path, held) = verified_download(root, tag, context)?;
    drop(held);
    open(&path)?;
    Ok(json!({"opened": true, "path": path_format::display(&path)}))
}

fn open_folder(path: &Path) -> Result<()> {
    Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{}\"", path.display()))
        .spawn()
        .map(drop)
        .map_err(|error| format!("UpdateRevealFailed: {error}"))
}

/// At GUI start: deletes downloads of versions not newer than this build.
fn cleanup(root: &Path, context: &Context) -> Result<Value> {
    let removed = match updates_folder(root, false)? {
        Some(updates) => remove_versions(&updates, |version| *version <= context.version)?,
        None => vec![],
    };
    Ok(json!({"removed": removed}))
}

pub(super) fn dispatch(root: &Path, verb: &str, args: &[String]) -> Option<Result<Value>> {
    if !verb.starts_with("app-update-") {
        return None;
    }
    let context = match Context::current() {
        Ok(context) => context,
        Err(error) => return Some(Err(error)),
    };
    Some(match (verb, args) {
        ("app-update-status", []) => Ok(status(root, &load(root), &context)),
        ("app-update-check", [origin]) if matches!(origin.as_str(), "auto" | "manual") => {
            check(root, origin == "auto", &context, fetch)
        }
        ("app-update-settings", [channel, auto_check]) => settings(root, channel, auto_check, &context),
        ("app-update-download", [tag]) => download(root, tag, &context, transfer),
        ("app-update-install", [tag]) => install(root, tag, &context, launch_setup),
        ("app-update-reveal", [tag]) => reveal(root, tag, &context, open_folder),
        ("app-update-cleanup", []) => cleanup(root, &context),
        _ => Err("InvalidUpdateArguments".into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "sha256:9e39c30439a3bcda170d1c25007ac13028dfd8bd468f4290a840efbbf119dc97";

    fn context(version: &str, kind: InstallKind) -> Context {
        Context { now: 1_000_000, version: Version::parse(version).unwrap(), platform: Some("windows-x64"), kind }
    }

    fn release(tag: &str, prerelease: bool) -> Value {
        let version = tag.trim_start_matches('v');
        let asset = |suffix: &str| json!({
            "name": format!("InstanceHarbor-{version}-windows-x64{suffix}"),
            "browser_download_url": format!("{DOWNLOAD_PREFIX}{tag}/InstanceHarbor-{version}-windows-x64{suffix}"),
            "size": 8_677_295, "digest": DIGEST, "state": "uploaded",
        });
        json!({"tag_name": tag, "draft": false, "prerelease": prerelease, "published_at": "2026-11-25T18:54:32Z",
            "html_url": format!("https://github.com/Command1264/InstanceHarbor/releases/tag/{tag}"),
            "body": "notes", "assets": [asset("-setup.exe"), asset(".zip")]})
    }

    fn releases(items: &[Value]) -> Vec<Release> {
        parse_releases(&serde_json::to_vec(&Value::Array(items.to_vec())).unwrap()).unwrap()
    }

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("ih-app-update-{name}-{}-{}", std::process::id(), clock()));
        fs::create_dir(&root).unwrap();
        fs::canonicalize(root).unwrap()
    }

    #[test]
    fn stable_channel_skips_prereleases_and_prerelease_channel_offers_them() {
        let list = releases(&[release("v1.2.0-rc.1", true), release("v1.1.1", false), release("v1.1.0", false)]);
        let current = context("1.1.0", InstallKind::Setup);
        assert_eq!(pick(&list, Channel::Stable, &current).unwrap().version, "1.1.1");
        let pre = pick(&list, Channel::Prerelease, &current).unwrap();
        assert_eq!((pre.version.as_str(), pre.prerelease), ("1.2.0-rc.1", true));
        // A stable-marked release with a prerelease number stays out of the stable channel.
        let mislabeled = releases(&[release("v1.3.0-rc.1", false)]);
        assert_eq!(pick(&mislabeled, Channel::Stable, &current), None);
    }

    #[test]
    fn never_suggests_a_downgrade_or_the_same_version() {
        let list = releases(&[release("v1.1.0", false), release("v1.0.1", false), release("v1.2.0-rc.1", true)]);
        assert_eq!(pick(&list, Channel::Stable, &context("1.1.0", InstallKind::Setup)), None);
        // Back on the stable channel while running a newer prerelease: nothing.
        assert_eq!(pick(&list, Channel::Stable, &context("1.2.0-rc.1", InstallKind::Setup)), None);
        assert_eq!(pick(&list, Channel::Prerelease, &context("1.2.0-rc.1", InstallKind::Setup)), None);
        // The release of a prerelease the user runs is an update.
        let list = releases(&[release("v1.2.0", false)]);
        assert_eq!(pick(&list, Channel::Stable, &context("1.2.0-rc.2", InstallKind::Setup)).unwrap().version, "1.2.0");
    }

    #[test]
    fn picks_the_asset_for_the_install_kind() {
        let list = releases(&[release("v1.2.0", false)]);
        let setup = pick(&list, Channel::Stable, &context("1.1.0", InstallKind::Setup)).unwrap();
        assert_eq!(setup.asset.unwrap().name, "InstanceHarbor-1.2.0-windows-x64-setup.exe");
        let zip = pick(&list, Channel::Stable, &context("1.1.0", InstallKind::Portable)).unwrap();
        let asset = zip.asset.unwrap();
        assert_eq!(asset.name, "InstanceHarbor-1.2.0-windows-x64.zip");
        assert_eq!(asset.url, "https://github.com/Command1264/InstanceHarbor/releases/download/v1.2.0/InstanceHarbor-1.2.0-windows-x64.zip");
        assert_eq!(asset.digest.as_deref(), Some(DIGEST));
    }

    #[test]
    fn a_release_without_a_file_for_this_platform_is_reported_not_replaced() {
        let mut item = release("v1.2.0", false);
        item["assets"] = json!([{"name": "InstanceHarbor-1.2.0-windows-arm64-setup.exe",
            "browser_download_url": format!("{DOWNLOAD_PREFIX}v1.2.0/InstanceHarbor-1.2.0-windows-arm64-setup.exe"),
            "size": 1, "digest": DIGEST}]);
        let found = pick(&releases(&[item, release("v1.1.1", false)]), Channel::Stable, &context("1.1.0", InstallKind::Setup)).unwrap();
        assert_eq!((found.version.as_str(), found.asset, found.problem), ("1.2.0", None, Some("NoMatchingAsset")));
        let mut other = context("1.1.0", InstallKind::Setup);
        other.platform = None;
        let found = pick(&releases(&[release("v1.2.0", false)]), Channel::Stable, &other).unwrap();
        assert_eq!(found.problem, Some("NoMatchingAsset"));
        assert_eq!(platform_label("windows", "x86_64"), Some("windows-x64"));
        assert_eq!(platform_label("windows", "aarch64"), None);
    }

    #[test]
    fn an_asset_outside_github_downloads_or_without_sha256_is_untrusted() {
        let current = context("1.1.0", InstallKind::Portable);
        for (field, value) in [
            ("browser_download_url", json!("https://example.com/InstanceHarbor-1.2.0-windows-x64.zip")),
            ("browser_download_url", json!(format!("{DOWNLOAD_PREFIX}v1.1.0/InstanceHarbor-1.2.0-windows-x64.zip"))),
            ("digest", Value::Null),
            ("digest", json!("md5:abc")),
            ("digest", json!(DIGEST.to_uppercase())),
            ("size", json!(0)),
        ] {
            let mut item = release("v1.2.0", false);
            item["assets"][1][field] = value.clone();
            let found = pick(&releases(&[item]), Channel::Stable, &current).unwrap();
            assert_eq!((found.asset, found.problem), (None, Some("UntrustedAsset")), "{field} {value}");
        }
    }

    #[test]
    fn parsing_skips_drafts_and_tags_that_are_not_versions() {
        let mut draft = release("v9.0.0", false);
        draft["draft"] = json!(true);
        let mut nightly = release("nightly", true);
        nightly["tag_name"] = json!("nightly");
        let list = releases(&[draft, nightly, release("v1.2.0", false)]);
        assert_eq!(list.iter().map(|release| release.tag.as_str()).collect::<Vec<_>>(), ["v1.2.0"]);
        assert_eq!(parse_releases(b"{\"message\":\"x\"}"), Err("UpdateCheckInvalidResponse".into()));
        assert_eq!(parse_releases(b"<html>"), Err("UpdateCheckInvalidResponse".into()));
    }

    #[test]
    fn http_and_transport_failures_have_their_own_reasons() {
        for (response, reason) in [
            (Ok((403, vec![])), "UpdateCheckRateLimited"),
            (Ok((429, vec![])), "UpdateCheckRateLimited"),
            (Ok((404, vec![])), "UpdateCheckNotFound"),
            (Ok((500, vec![])), "UpdateCheckHttpError"),
            (Err("CoreSourceTimeout".to_owned()), "UpdateCheckTimeout"),
            (Err("CoreSourceTransportError".to_owned()), "UpdateCheckOffline"),
            (Err("CoreSystemCurlUnavailable".to_owned()), "UpdateCheckUnavailable"),
        ] {
            assert_eq!(releases_from(response), Err(reason.to_owned()));
        }
    }

    #[test]
    fn automatic_checks_run_once_a_day_and_can_be_turned_off() {
        let root = temp_root("auto");
        let body = serde_json::to_vec(&json!([release("v1.2.0", false)])).unwrap();
        let mut current = context("1.1.0", InstallKind::Setup);
        let first = check(&root, true, &current, || Ok((200, body.clone()))).unwrap();
        assert_eq!(first["update"]["version"], "1.2.0");
        assert_eq!(first["due"], false);
        current.now += AUTO_INTERVAL - 1;
        let skipped = check(&root, true, &current, || panic!("must not ask GitHub again")).unwrap();
        assert_eq!(skipped["update"]["version"], "1.2.0", "the cached result survives a restart");
        current.now += 1;
        let again = check(&root, true, &current, || Ok((200, b"[]".to_vec()))).unwrap();
        assert_eq!(again["update"], Value::Null);
        settings(&root, "stable", "false", &current).unwrap();
        current.now += 10 * AUTO_INTERVAL;
        let off = check(&root, true, &current, || panic!("turned off")).unwrap();
        assert_eq!(off["due"], false);
        // A manual check still runs while automatic checks are off.
        check(&root, false, &current, || Ok((200, body.clone()))).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_failed_check_keeps_the_last_result_and_records_only_the_reason() {
        let root = temp_root("failure");
        let body = serde_json::to_vec(&json!([release("v1.2.0", false)])).unwrap();
        let mut current = context("1.1.0", InstallKind::Setup);
        check(&root, false, &current, || Ok((200, body))).unwrap();
        current.now += 60;
        let failed = check(&root, false, &current, || Err("CoreSourceTransportError".into())).unwrap();
        assert_eq!(failed["lastError"], "UpdateCheckOffline");
        assert_eq!(failed["lastAttempt"], current.now);
        assert_eq!(failed["lastSuccess"], current.now - 60);
        assert_eq!(failed["update"]["version"], "1.2.0");
        let recovered = check(&root, false, &current, || Ok((200, b"[]".to_vec()))).unwrap();
        assert_eq!(recovered["lastError"], Value::Null);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn settings_default_to_stable_with_automatic_checks_and_reject_other_values() {
        let root = temp_root("settings");
        let current = context("1.1.0", InstallKind::Portable);
        let initial = status(&root, &load(&root), &current);
        assert_eq!((initial["channel"].as_str(), initial["autoCheck"].as_bool(), initial["due"].as_bool()), (Some("stable"), Some(true), Some(true)));
        assert_eq!((initial["install"].as_str(), initial["platform"].as_str()), (Some("portable"), Some("windows-x64")));
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0, "reading writes nothing");
        let body = serde_json::to_vec(&json!([release("v1.3.0-rc.1", true), release("v1.2.0", false)])).unwrap();
        check(&root, false, &current, || Ok((200, body))).unwrap();
        let saved = settings(&root, "prerelease", "true", &current).unwrap();
        assert_eq!(saved["update"]["version"], "1.3.0-rc.1", "a channel change uses the cached list");
        assert_eq!(settings(&root, "beta", "true", &current), Err("InvalidUpdateChannel".into()));
        assert_eq!(settings(&root, "stable", "yes", &current), Err("InvalidUpdateAutoCheck".into()));
        // A file from a newer version with extra fields still loads.
        let mut value: Value = serde_json::from_slice(&fs::read(state_path(&root)).unwrap()).unwrap();
        value["futureField"] = json!(1);
        fs::write(state_path(&root), serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(load(&root).channel, Channel::Prerelease);
        fs::write(state_path(&root), b"not json").unwrap();
        assert_eq!(load(&root), State::default());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_clock_set_back_makes_the_check_due() {
        let state = State { last_attempt: Some(2_000), ..State::default() };
        assert!(due(&state, 1_000));
        assert!(!due(&state, 2_000 + AUTO_INTERVAL - 1));
    }

    #[test]
    fn the_dispatch_rejects_unknown_arguments() {
        let root = temp_root("dispatch");
        let call = |verb: &str, args: &[&str]| dispatch(&root, verb, &args.iter().map(|a| a.to_string()).collect::<Vec<_>>());
        assert_eq!(call("app-update-check", &["later"]).unwrap(), Err("InvalidUpdateArguments".into()));
        assert_eq!(call("app-update-status", &["x"]).unwrap(), Err("InvalidUpdateArguments".into()));
        assert!(call("app-update-status", &[]).unwrap().is_ok());
        assert!(call("list", &[]).is_none());
        fs::remove_dir_all(root).unwrap();
    }

    const PAYLOAD: &[u8] = b"pretend setup bytes";

    /// A workspace whose last check found 1.2.0 with files whose SHA-256 matches `PAYLOAD`.
    fn ready(name: &str, kind: InstallKind) -> (PathBuf, Context) {
        let root = temp_root(name);
        fs::create_dir(root.join("staging")).unwrap();
        let digest = format!("sha256:{}", content_revision::revision(PAYLOAD).unwrap());
        let mut item = release("v1.2.0", false);
        for asset in item["assets"].as_array_mut().unwrap() {
            asset["digest"] = json!(digest);
            asset["size"] = json!(PAYLOAD.len());
        }
        let current = context("1.1.0", kind);
        check(&root, false, &current, || Ok((200, serde_json::to_vec(&json!([item])).unwrap()))).unwrap();
        (root, current)
    }

    fn write(bytes: &'static [u8]) -> impl FnOnce(&Asset, &Path) -> Result<()> {
        move |_, path| fs::write(path, bytes).map_err(fail)
    }

    fn update_file(root: &Path, name: &str) -> PathBuf {
        root.join("staging").join("updates").join("1.2.0").join(name)
    }

    #[test]
    fn a_download_is_kept_only_when_its_sha256_matches() {
        let (root, current) = ready("download", InstallKind::Setup);
        let name = "InstanceHarbor-1.2.0-windows-x64-setup.exe";
        let wrong = download(&root, "v1.2.0", &current, write(b"pretend setup bytez"));
        assert_eq!(wrong, Err("UpdateDownloadDigestMismatch".into()));
        assert_eq!(fs::read_dir(update_file(&root, name).parent().unwrap()).unwrap().count(), 0, "nothing kept");
        let short = download(&root, "v1.2.0", &current, write(b"short"));
        assert_eq!(short, Err("UpdateDownloadSizeMismatch".into()));
        let done = download(&root, "v1.2.0", &current, write(PAYLOAD)).unwrap();
        assert_eq!(done["download"], json!({"version": "1.2.0", "name": name}));
        assert_eq!(fs::read(update_file(&root, name)).unwrap(), PAYLOAD);
        // A verified file is not downloaded again.
        download(&root, "v1.2.0", &current, |_, _| panic!("already downloaded")).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn only_the_offered_update_with_a_trusted_file_is_downloaded() {
        let (root, current) = ready("offered", InstallKind::Portable);
        assert_eq!(download(&root, "v1.1.9", &current, |_, _| panic!()), Err("UpdateNotAvailable".into()));
        assert_eq!(download(&root, "https://example.com/x", &current, |_, _| panic!()), Err("UpdateNotAvailable".into()));
        let mut state = load(&root);
        state.releases[0].assets[1].url = "https://example.com/InstanceHarbor-1.2.0-windows-x64.zip".into();
        store(&root, &state).unwrap();
        assert_eq!(download(&root, "v1.2.0", &current, |_, _| panic!()), Err("UpdateAssetUnavailable".into()));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn at_most_one_version_is_kept_and_older_ones_go_at_start() {
        let (root, current) = ready("keep-one", InstallKind::Portable);
        let updates = root.join("staging").join("updates");
        for name in ["1.1.5", "1.0.0", "notes"] {
            fs::create_dir_all(updates.join(name)).unwrap();
            fs::write(updates.join(name).join("file"), b"x").unwrap();
        }
        download(&root, "v1.2.0", &current, write(PAYLOAD)).unwrap();
        let mut left: Vec<_> = fs::read_dir(&updates).unwrap().map(|entry| entry.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(left, ["1.2.0", "notes"], "other versions removed, other names untouched");
        // Still older than the update: kept at start.
        assert_eq!(cleanup(&root, &current).unwrap()["removed"], json!([]));
        // After the update is installed, the next start removes it.
        let installed = context("1.2.0", InstallKind::Portable);
        assert_eq!(cleanup(&root, &installed).unwrap()["removed"], json!(["1.2.0"]));
        assert!(updates.join("notes").exists());
        let empty = temp_root("cleanup-empty");
        fs::create_dir(empty.join("staging")).unwrap();
        assert_eq!(cleanup(&empty, &installed).unwrap()["removed"], json!([]));
        assert!(!empty.join("staging").join("updates").exists(), "cleanup creates nothing");
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(empty).unwrap();
    }

    #[test]
    fn setup_starts_only_a_verified_download_after_confirmation() {
        let (root, current) = ready("install", InstallKind::Setup);
        assert_eq!(install(&root, "v1.2.0", &current, |_| panic!("not downloaded")), Err("UpdateDownloadMissing".into()));
        download(&root, "v1.2.0", &current, write(PAYLOAD)).unwrap();
        let path = update_file(&root, "InstanceHarbor-1.2.0-windows-x64-setup.exe");
        // Changed after the download: never started.
        fs::write(&path, b"pretend setup bytez").unwrap();
        assert_eq!(install(&root, "v1.2.0", &current, |_| panic!("tampered")), Err("UpdateDownloadDigestMismatch".into()));
        assert!(!path.exists(), "a file that no longer matches is deleted");
        assert_eq!(status(&root, &load(&root), &current)["download"], Value::Null);
        fs::write(&path, PAYLOAD).unwrap();
        let mut started = None;
        let result = install(&root, "v1.2.0", &current, |launched| {
            // The file cannot be replaced while Setup starts.
            assert!(OpenOptions::new().write(true).open(launched).is_err());
            started = Some(launched.to_owned());
            Ok(42)
        })
        .unwrap();
        assert_eq!((result["started"].as_bool(), result["pid"].as_u64()), (Some(true), Some(42)));
        assert_eq!(started.as_deref(), Some(path.as_path()));
        let portable = context("1.1.0", InstallKind::Portable);
        assert_eq!(install(&root, "v1.2.0", &portable, |_| panic!()), Err("UpdateInstallNeedsSetup".into()));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_portable_build_only_shows_the_verified_zip() {
        let (root, current) = ready("reveal", InstallKind::Portable);
        download(&root, "v1.2.0", &current, write(PAYLOAD)).unwrap();
        let mut shown = None;
        reveal(&root, "v1.2.0", &current, |path| { shown = Some(path.to_owned()); Ok(()) }).unwrap();
        assert_eq!(shown, Some(update_file(&root, "InstanceHarbor-1.2.0-windows-x64.zip")));
        let setup = context("1.1.0", InstallKind::Setup);
        assert_eq!(reveal(&root, "v1.2.0", &setup, |_| panic!()), Err("UpdateRevealNeedsPortable".into()));
        fs::remove_dir_all(root).unwrap();
    }
}
