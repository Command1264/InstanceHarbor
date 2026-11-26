//! Checks GitHub Releases for a newer InstanceHarbor (#65).
//!
//! Only the GUI asks; the Manager never checks. One check sends one
//! `GET /repos/Command1264/InstanceHarbor/releases?per_page=10` through the system
//! `curl.exe` (see `core_source::fetch_status`) and nothing about the user or the
//! workspace. The settings and the last result live in the workspace's
//! `app-update.json`, which older versions do not read, so a rollback keeps working.
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

fn status(state: &State, context: &Context) -> Value {
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
        "update": pick(&state.releases, state.channel, context),
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
        return Ok(status(&state, context));
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
    Ok(status(&state, context))
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
    Ok(status(&state, context))
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
        ("app-update-status", []) => Ok(status(&load(root), &context)),
        ("app-update-check", [origin]) if matches!(origin.as_str(), "auto" | "manual") => {
            check(root, origin == "auto", &context, fetch)
        }
        ("app-update-settings", [channel, auto_check]) => settings(root, channel, auto_check, &context),
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
        let initial = status(&load(&root), &current);
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
}
