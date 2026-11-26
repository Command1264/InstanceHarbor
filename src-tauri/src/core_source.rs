//! Read-only core release checks. Source results are tied to an observed local JAR hash.
use super::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    os::windows::ffi::OsStringExt,
    sync::{atomic::{AtomicUsize, Ordering}, Condvar, Mutex},
};

pub(super) const MAX_TARGETS: usize = 64;
const MAX_BODY: u64 = 4 * 1024 * 1024;
const MAX_ARTIFACT_BYTES: u64 = 256 * 1024 * 1024;
const MAX_PARALLEL_GROUPS: usize = 2;
const STATUS_LINE_BYTES: u64 = 4;
const CONTACT_ENV: &str = "INSTANCEHARBOR_PAPERMC_CONTACT_URL";
const DEFAULT_CONTACT: &str = "https://github.com/Command1264/InstanceHarbor";
const FILL_PROJECTS: &str = "https://fill.papermc.io/v3/projects";
#[path = "core_source/creation_assets.rs"]
mod creation_assets;

const SPIGOT_VERSIONS: &str = "https://hub.spigotmc.org/versions";
const SPIGOT_COMMITS: &str = "https://hub.spigotmc.org/stash/rest/api/1.0/projects/SPIGOT/repos";
const SPIGOT_188_HASH: &str = "6ed6046687ce9255228fe7bd31e097da49beff2a0c577e7be56d14f09753239c";
const SPIGOT_REF: &str = "3c60ece1480c9b686b06f33daa6ca23c8883e9f2";
const CRAFTBUKKIT_REF: &str = "741a1bdf3db8c4d5237407df2872d9857427bfaf";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SourceKey {
    family: String,
    version: String,
}

struct PaperBuild {
    id: u64,
    stable: bool,
    /// Velocity publishes every build of a `-SNAPSHOT` version as `BETA`.
    beta: bool,
    size: Option<u64>,
    sha256: Option<String>,
    url: Option<String>,
}

enum Release {
    Paper(Vec<PaperBuild>),
    Spigot { build: String, spigot_ref: String, craftbukkit_ref: String },
}

struct Metadata {
    url: String,
    release: Result<Release>,
    native_distance: Option<Result<u64>>,
}

static SOURCE_WORKERS: (Mutex<usize>, Condvar) = (Mutex::new(0), Condvar::new());

struct SourceSlot;

impl SourceSlot {
    fn acquire() -> Result<Self> {
        let (in_use, available) = &SOURCE_WORKERS;
        let active = in_use.lock().map_err(|_| "CoreSourceWorkerFailed")?;
        let (mut active, _) = available
            .wait_timeout_while(active, Duration::from_secs(15), |count| {
                *count >= MAX_PARALLEL_GROUPS
            })
            .map_err(|_| "CoreSourceWorkerFailed")?;
        if *active >= MAX_PARALLEL_GROUPS {
            return Err("CoreSourceQueueTimeout".into());
        }
        *active += 1;
        Ok(Self)
    }
}

impl Drop for SourceSlot {
    fn drop(&mut self) {
        let (in_use, available) = &SOURCE_WORKERS;
        if let Ok(mut active) = in_use.lock() {
            *active -= 1;
            available.notify_one();
        }
    }
}

fn contact_url(value: &str) -> bool {
    // The contact is placed in a request header. Allow ordinary printable HTTPS URLs
    // with a DNS host. Userinfo, whitespace, header delimiters and IPs are out of scope.
    if value.len() > 256
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b":/._~!$&'*+,;=?%#-".contains(&byte))
    {
        return false;
    }
    let Some(rest) = value.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let Some((_, suffix)) = host.rsplit_once('.') else {
        return false;
    };
    suffix.len() >= 2
        && suffix.bytes().all(|byte| byte.is_ascii_alphabetic())
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

fn paper_user_agent(override_contact: Option<&OsStr>) -> Result<String> {
    let contact = match override_contact {
        Some(value) => value.to_str().ok_or("CoreSourceInvalidContact")?,
        None => DEFAULT_CONTACT,
    };
    if !contact_url(contact) {
        return Err("CoreSourceInvalidContact".into());
    }
    Ok(format!("InstanceHarbor/2.0 ({contact})"))
}

fn configured_paper_user_agent() -> Result<String> {
    paper_user_agent(std::env::var_os(CONTACT_ENV).as_deref())
}

fn system_curl() -> Result<PathBuf> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
    }
    let mut system = [0u16; 32768];
    // Using the OS's system directory avoids trusting PATH and environment overrides.
    let length = unsafe { GetSystemDirectoryW(system.as_mut_ptr(), system.len() as u32) } as usize;
    if length == 0 || length >= system.len() {
        return Err("CoreSystemDirectoryUnavailable".into());
    }
    let path = PathBuf::from(OsString::from_wide(&system[..length])).join("curl.exe");
    if !path.is_file() {
        return Err("CoreSystemCurlUnavailable".into());
    }
    Ok(path)
}

fn fetch_json(url: &str, user_agent: &str) -> Result<Value> {
    let (code, body) = fetch_status(url, user_agent, true)?;
    if code != 200 {
        return Err("CoreSourceHttpError".into());
    }
    serde_json::from_slice(&body).map_err(|_| "CoreSourceInvalidJson".into())
}

/// One public JSON lookup that reports the HTTP status instead of failing on
/// it, so the caller can tell "not found" from "rate limited". No retries.
pub(super) fn fetch_profile(url: &str) -> Result<(u16, Vec<u8>)> {
    fetch_status(url, &configured_paper_user_agent()?, false)
}

fn fetch_status(url: &str, user_agent: &str, fail_on_http: bool) -> Result<(u16, Vec<u8>)> {
    let mut command = Command::new(system_curl()?);
    // --disable has to be first to prevent loading a user's curl config.
    command.arg("--disable");
    if fail_on_http {
        command.arg("--fail");
    }
    let mut child = command
        .args([
            "--silent",
            "--globoff",
            "--proto",
            "=https",
            "--connect-timeout",
            "4",
            "--max-time",
            "8",
            "--max-filesize",
            "4194304",
            "--user-agent",
            user_agent,
            "--header",
            "Accept: application/json",
            "--write-out",
            "\n%{http_code}",
            "--url",
            url,
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .env_remove("SSLKEYLOGFILE")
        .env_remove("CURL_CA_BUNDLE")
        .env_remove("SSL_CERT_FILE")
        .env_remove("SSL_CERT_DIR")
        .creation_flags(HIDDEN)
        .spawn()
        .map_err(|_| "CoreSourceLaunchFailed")?;
    let mut body = Vec::new();
    let read = child
        .stdout
        .take()
        .ok_or("CoreSourcePipeUnavailable")
        .and_then(|stdout| {
            stdout
                .take(MAX_BODY + STATUS_LINE_BYTES + 1)
                .read_to_end(&mut body)
                .map_err(|_| "CoreSourceReadFailed")
        });
    if read.is_err() || body.len() as u64 > MAX_BODY + STATUS_LINE_BYTES {
        let _ = child.kill();
    }
    let status = child.wait().map_err(|_| "CoreSourceWaitFailed")?;
    read?;
    if body.len() as u64 > MAX_BODY + STATUS_LINE_BYTES || status.code() == Some(63) {
        return Err("CoreSourceResponseTooLarge".into());
    }
    if !status.success() {
        return Err(match status.code() {
            Some(28) => "CoreSourceTimeout",
            Some(22) => "CoreSourceHttpError",
            _ => "CoreSourceTransportError",
        }
        .into());
    }
    let split = body.iter().rposition(|&byte| byte == b'\n').ok_or("CoreSourceHttpError")?;
    let code = std::str::from_utf8(&body[split + 1..])
        .ok()
        .and_then(|text| text.parse::<u16>().ok())
        .ok_or("CoreSourceHttpError")?;
    body.truncate(split);
    Ok((code, body))
}

fn sha256(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn source_key(profile: &CoreProfile) -> Option<SourceKey> {
    if !matches!(
        profile.family.as_str(),
        "PAPER" | "FOLIA" | "VELOCITY" | "SPIGOT"
    ) || profile.minecraft.len() > 32
        || !profile
            .minecraft
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_digit)
        || !profile
            .minecraft
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-".contains(&byte))
        || !sha256(&profile.sha256)
    {
        return None;
    }
    Some(SourceKey {
        family: profile.family.clone(),
        version: profile.minecraft.clone(),
    })
}

fn candidate_url(url: &str, hash: &str) -> bool {
    let Some(name) = url
        .strip_prefix("https://fill-data.papermc.io/v1/objects/")
        .and_then(|path| {
            let (digest, name) = path.split_once('/')?;
            digest.eq_ignore_ascii_case(hash).then_some(name)
        })
    else {
        return false;
    };
    name.len() <= 160
        && name.ends_with(".jar")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

fn paper_builds(value: Value) -> Result<Vec<PaperBuild>> {
    let rows = value.as_array().ok_or("CoreSourceInvalidBuilds")?;
    let mut builds = BTreeMap::new();
    for row in rows {
        let id = row["id"].as_u64().ok_or("CoreSourceInvalidBuilds")?;
        let channel = row["channel"].as_str().ok_or("CoreSourceInvalidBuilds")?;
        let download = &row["downloads"]["server:default"];
        let hash = download["checksums"]["sha256"]
            .as_str()
            .filter(|text| sha256(text));
        let url = download["url"]
            .as_str()
            .filter(|url| hash.is_some_and(|digest| candidate_url(url, digest)));
        let build = PaperBuild {
            id,
            stable: channel == "STABLE",
            beta: channel == "BETA",
            size: artifact_size(download),
            sha256: hash.map(str::to_ascii_lowercase),
            url: url.map(str::to_owned),
        };
        if builds.insert(id, build).is_some() {
            return Err("CoreSourceAmbiguousBuilds".into());
        }
    }
    Ok(builds.into_values().collect())
}

fn spigot_release(value: Value) -> Result<Release> {
    let build = value["name"]
        .as_str()
        .ok_or("CoreSourceInvalidSpigotRefs")?;
    let number = build
        .strip_suffix(|character: char| character.is_ascii_alphabetic())
        .unwrap_or(build);
    if number.is_empty()
        || !number.bytes().all(|byte| byte.is_ascii_digit())
        || number.parse::<u64>().is_err()
    {
        return Err("CoreSourceInvalidSpigotRefs".into());
    }
    let reference = |key: &str| -> Result<&str> {
        value["refs"][key]
            .as_str()
            .filter(|text| {
                matches!(text.len(), 7..=40) && text.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
            .ok_or_else(|| "CoreSourceInvalidSpigotRefs".into())
    };
    let spigot = reference("Spigot")?.to_ascii_lowercase();
    let craft = reference("CraftBukkit")?.to_ascii_lowercase();
    Ok(Release::Spigot {
        build: build.into(),
        spigot_ref: spigot,
        craftbukkit_ref: craft,
    })
}

fn native_spigot(fetch: &impl Fn(&str, &str) -> Result<Value>) -> Result<u64> {
    let count = |project, reference: &str| -> Result<u64> {
        let url = format!(
            "{SPIGOT_COMMITS}/{project}/commits?since={}&withCounts=true",
            &reference[..7]
        );
        let result = fetch(&url, "InstanceHarbor/2.0")?;
        result["totalCount"]
            .as_u64()
            .filter(|count| *count <= i32::MAX as u64)
            .ok_or_else(|| "CoreSourceInvalidSpigotDistance".into())
    };
    let craft = count("craftbukkit", CRAFTBUKKIT_REF);
    let spigot = count("spigot", SPIGOT_REF);
    let sum = craft? + spigot?;
    if sum > i32::MAX as u64 {
        return Err("CoreSourceInvalidSpigotDistance".into());
    }
    Ok(sum)
}

fn group_url(key: &SourceKey) -> String {
    if key.family == "SPIGOT" {
        return format!("{SPIGOT_VERSIONS}/{}.json", key.version);
    }
    let project = key.family.to_ascii_lowercase();
    format!("{FILL_PROJECTS}/{project}/versions/{}/builds", key.version)
}

fn fetch_group(
    key: &SourceKey,
    paper_agent: std::result::Result<&str, &str>,
    fetch: &impl Fn(&str, &str) -> Result<Value>,
) -> Metadata {
    let url = group_url(key);
    let _slot = match SourceSlot::acquire() {
        Ok(slot) => slot,
        Err(error) => {
            return Metadata {
                url,
                release: Err(error),
                native_distance: None,
            }
        }
    };
    if key.family == "SPIGOT" {
        let release = fetch(&url, "InstanceHarbor/2.0").and_then(spigot_release);
        return Metadata {
            url,
            release,
            native_distance: (key.version == "1.8.8").then(|| native_spigot(fetch)),
        };
    }
    let release = match paper_agent {
        Ok(agent) => {
            fetch(&url, agent)
                .and_then(paper_builds)
                .map(Release::Paper)
        }
        Err(error) => Err(error.into()),
    };
    Metadata {
        url,
        release,
        native_distance: None,
    }
}

fn creation_project(family: &str) -> Result<&'static str> {
    match family {
        "PAPER" => Ok("paper"),
        "FOLIA" => Ok("folia"),
        _ => Err("UnsupportedCreationCore".into()),
    }
}

fn release_version(version: &str) -> Option<Vec<u32>> {
    if version.len() > 32 {
        return None;
    }
    let parts: Vec<_> = version.split('.').collect();
    if !(2..=4).contains(&parts.len()) {
        return None;
    }
    parts.into_iter().map(|part| {
        if part.is_empty() || (part.len() > 1 && part.starts_with('0'))
            || !part.bytes().all(|byte| byte.is_ascii_digit())
        {
            return None;
        }
        part.parse().ok()
    }).collect()
}

fn project_versions(value: Value, project: &str) -> Result<Vec<String>> {
    if value["project"]["id"].as_str() != Some(project) {
        return Err("CoreCatalogInvalid".into());
    }
    let groups = value["versions"].as_object().ok_or("CoreCatalogInvalid")?;
    let mut versions = BTreeMap::new();
    for group in groups.values() {
        for entry in group.as_array().ok_or("CoreCatalogInvalid")? {
            let version = entry.as_str().ok_or("CoreCatalogInvalid")?;
            if let Some(order) = release_version(version) {
                versions.insert(order, version.to_owned());
            }
        }
    }
    Ok(versions.into_values().rev().collect())
}

pub(super) fn creation_versions(family: &str) -> Result<Value> {
    let project = creation_project(family)?;
    let agent = configured_paper_user_agent()?;
    let _slot = SourceSlot::acquire()?;
    let versions = project_versions(fetch_json(&format!("{FILL_PROJECTS}/{project}"), &agent)?, project)?;
    Ok(json!({"versions": versions}))
}

fn creation_builds(family: &str, version: &str, agent: &str) -> Result<Vec<PaperBuild>> {
    creation_project(family)?;
    release_version(version).ok_or("UnsupportedCreationCore")?;
    let key = SourceKey { family: family.to_owned(), version: version.to_owned() };
    match fetch_group(&key, Ok(agent), &fetch_json).release? {
        Release::Paper(builds) => Ok(builds),
        Release::Spigot { .. } => Err("UnsupportedCreationCore".into()),
    }
}

/// Confirm a local core that the built-in table does not know. The family and
/// version come from the JAR's own metadata; only an exact SHA-256 match in the
/// official PaperMC catalog makes the result trusted.
pub(super) fn catalog_profile(family: &str, version: &str, sha256: &str) -> Result<Option<CoreProfile>> {
    catalog_profile_with(family, version, sha256, &fetch_json)
}

fn catalog_profile_with(
    family: &str,
    version: &str,
    sha256: &str,
    fetch: &impl Fn(&str, &str) -> Result<Value>,
) -> Result<Option<CoreProfile>> {
    if !matches!(family, "PAPER" | "FOLIA" | "VELOCITY")
        || version.is_empty()
        || version.len() > 32
        || !version.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return Ok(None);
    }
    let agent = configured_paper_user_agent()?;
    let key = SourceKey { family: family.to_owned(), version: version.to_owned() };
    let builds = match fetch_group(&key, Ok(&agent), fetch).release {
        Ok(Release::Paper(builds)) => builds,
        Ok(Release::Spigot { .. }) => return Ok(None),
        Err(_) => return Err("CoreCatalogUnavailable".into()),
    };
    Ok(builds
        .into_iter()
        .find(|build| build.sha256.as_deref() == Some(sha256))
        .map(|build| CoreProfile {
            family: family.to_owned(),
            minecraft: version.to_owned(),
            build: build.id.to_string(),
            sha256: sha256.to_owned(),
            unverified: false,
            java: None,
        }))
}

fn creation_jar_name(family: &str, version: &str, build: u64) -> Result<String> {
    let project = creation_project(family)?;
    Ok(format!("{project}-{version}-{build}.jar"))
}

fn latest_stable<'a>(builds: &'a [PaperBuild], family: &str, version: &str) -> Result<&'a PaperBuild> {
    let build = builds.iter().filter(|build| build.stable).max_by_key(|build| build.id)
        .ok_or("CreationNoStableBuild")?;
    let name = creation_jar_name(family, version, build.id)?;
    if build.sha256.is_none() || build.url.as_deref().and_then(|url| url.rsplit('/').next()) != Some(name.as_str()) {
        return Err("CoreCandidateUnverified".into());
    }
    Ok(build)
}

pub(super) fn latest_creation_core(family: &str, version: &str) -> Result<CoreProfile> {
    let agent = configured_paper_user_agent()?;
    let builds = creation_builds(family, version, &agent)?;
    let build = latest_stable(&builds, family, version)?;
    Ok(CoreProfile { unverified: false, java: None,
        family: family.to_owned(),
        minecraft: version.to_owned(),
        build: build.id.to_string(),
        sha256: build.sha256.clone().ok_or("CoreCandidateUnverified")?,
    })
}

fn exact_stable_download<'a>(builds: &'a [PaperBuild], profile: &CoreProfile) -> Result<(&'a str, Option<u64>)> {
    let id = profile.build.parse::<u64>().map_err(|_| "CreationPreviewChanged")?;
    let name = creation_jar_name(&profile.family, &profile.minecraft, id)?;
    builds.iter().find(|build| {
        build.stable && build.id == id && build.sha256.as_deref() == Some(profile.sha256.as_str())
    }).and_then(|build| {
        let url = build.url.as_deref()?;
        (url.rsplit('/').next() == Some(name.as_str())).then_some((url, build.size))
    })
        .ok_or_else(|| "CreationPreviewChanged".into())
}

/// Whether a build may be offered as an update. Paper and Folia offer only
/// `STABLE` builds. Velocity also offers `BETA`, the only channel of its
/// `-SNAPSHOT` versions (#41); `ALPHA` builds are never offered.
fn update_eligible(family: &str, build: &PaperBuild) -> bool {
    build.stable || (family == "VELOCITY" && build.beta)
}

/// Families whose candidates the prefetch store prepares and shares.
pub(super) fn prefetch_family(family: &str) -> bool {
    matches!(family, "PAPER" | "VELOCITY")
}

fn unknown(name: &str, reason: &str) -> Value {
    json!({"name":name,"status":"Unknown","reason":reason})
}

fn result(name: &str, profile: &CoreProfile, metadata: &Metadata) -> Value {
    let mut row = json!({"name":name,"family":profile.family,"minecraft":profile.minecraft,
        "build":profile.build,"sha256":profile.sha256,"source":metadata.url,"status":"Unknown","reason":null});
    match &metadata.release {
        Err(error) => row["reason"] = json!(error),
        Ok(Release::Spigot { build, spigot_ref, craftbukkit_ref }) => {
            row["remoteBuild"] = json!(build);
            // A BuildTools version name alone does not attest to the contents of
            // a locally built Spigot JAR. Only this exact pinned JAR has verified
            // source revisions. Other pinned builds remain unknown until their
            // own source references can be independently established.
            if profile.minecraft == "1.8.8" && profile.sha256 == SPIGOT_188_HASH {
                let release_match = profile.build == *build
                    && spigot_ref == SPIGOT_REF
                    && craftbukkit_ref == CRAFTBUKKIT_REF;
                row["releaseMatch"] = json!(release_match);
                match &metadata.native_distance {
                    Some(Ok(0)) if release_match => row["status"] = json!("Current"),
                    Some(Ok(0)) => row["reason"] = json!("CoreSourceSpigotRevisionsDiffer"),
                    Some(Ok(_)) => row["status"] = json!("NativeBehind"),
                    Some(Err(_)) | None => row["reason"] = json!("CoreSourceSpigotNativeCheckFailed"),
                }
            } else {
                row["reason"] = json!("CoreSourceSpigotLocalRefsUnverified");
            }
        }
        Ok(Release::Paper(builds)) => {
            let mut hashes = builds
                .iter()
                .filter(|build| build.sha256.as_deref() == Some(profile.sha256.as_str()));
            let local = hashes.next();
            let newest = builds
                .iter()
                .filter(|build| update_eligible(&profile.family, build))
                .max_by_key(|build| build.id);
            match (local, hashes.next(), newest) {
                (Some(local), None, _) if profile.build.parse::<u64>().ok() != Some(local.id) => {
                    row["reason"] = json!("CoreSourceLocalBuildMismatch");
                }
                (Some(_), None, None) => row["status"] = json!("NoStableBuild"),
                (Some(local), None, Some(newest)) => {
                    row["remoteBuild"] = json!(newest.id.to_string());
                    if local.id == newest.id {
                        row["status"] = json!("Current");
                    } else if local.id < newest.id {
                        match (&newest.sha256, &newest.url) {
                            (Some(hash), Some(url))
                                if builds
                                    .iter()
                                    .filter(|build| build.sha256.as_ref() == Some(hash))
                                    .count()
                                    == 1 =>
                            {
                                row["status"] = json!("UpdateAvailable");
                                row["candidate"] = json!({"build":newest.id.to_string(),
                                    "sha256":hash,"url":url,"size":newest.size});
                            }
                            _ => row["reason"] = json!("CoreSourceUnverifiedCandidate"),
                        }
                    } else {
                        row["reason"] = json!("CoreSourceLocalNewerThanStable");
                    }
                }
                _ => row["reason"] = json!("CoreSourceUnverifiedLocalHash"),
            }
        }
    }
    if let Some(distance) = &metadata.native_distance {
        row["nativeVersion"] = match distance {
            Ok(0) => json!({"status":"Current","behind":0}),
            Ok(count) => json!({"status":"Behind","behind":count}),
            Err(error) => json!({"status":"Unknown","behind":null,"reason":error}),
        };
    }
    row
}

fn groups(
    keys: BTreeSet<SourceKey>,
    paper_agent: std::result::Result<&str, &str>,
    fetch: &(impl Fn(&str, &str) -> Result<Value> + Sync),
) -> BTreeMap<SourceKey, Metadata> {
    let keys: Vec<_> = keys.into_iter().collect();
    let cursor = AtomicUsize::new(0);
    let mut metadata = thread::scope(|scope| {
        let jobs = &keys;
        let cursor = &cursor;
        let workers: Vec<_> = (0..jobs.len().min(MAX_PARALLEL_GROUPS)).map(|_| {
            scope.spawn(move || {
                let mut completed = Vec::new();
                while let Some(key) = jobs.get(cursor.fetch_add(1, Ordering::Relaxed)) {
                    completed.push((key.clone(), fetch_group(key, paper_agent, fetch)));
                }
                completed
            })
        }).collect();
        workers.into_iter().filter_map(|worker| worker.join().ok()).flatten().collect::<BTreeMap<_, _>>()
    });
    for key in keys {
        metadata.entry(key.clone()).or_insert_with(|| Metadata {
            url: group_url(&key),
            release: Err("CoreSourceWorkerFailed".into()),
            native_distance: None,
        });
    }
    metadata
}

pub(super) fn check(root: &Path, names: &[String]) -> Result<Value> {
    let paper_agent = configured_paper_user_agent();
    check_with_agent(root, names, paper_agent.as_deref().map_err(String::as_str))
}

fn check_with_agent(
    root: &Path,
    names: &[String],
    paper_agent: std::result::Result<&str, &str>,
) -> Result<Value> {
    if names.is_empty() || names.len() > MAX_TARGETS || names.iter().any(|name| !instance_name(name)) {
        return Err("InvalidCoreCheckTargets".into());
    }
    let mut observed = Vec::with_capacity(names.len());
    let mut keys = BTreeSet::new();
    for name in names {
        let identity = (|| -> Result<_> {
            let registration = registration_revision(root, name)?;
            let config = instance(root, name)?;
            if !admitted(root, name)? || config.removed {
                return Err("CoreTargetUnavailable".into());
            }
            let jar = core_artifact::jar(&instance_path(root, name)?, &config);
            no_redirect(&jar)?;
            let profile =
                core_maintenance::trusted_profile(&config, content_revision::file_revision(&jar)?)
                    .ok_or("CoreIdentityUnverified")?;
            let key = source_key(&profile).ok_or("CoreSourceUnsupported")?;
            keys.insert(key.clone());
            Ok((jar, profile, key, registration))
        })();
        observed.push((name, identity));
    }
    let metadata = groups(keys, paper_agent, &fetch_json);
    let rows: Vec<_> = observed
        .into_iter()
        .map(|(name, identity)| {
            let (jar, profile, key, registration) = match identity {
                Ok(identity) => identity,
                Err(error) => {
                    return unknown(
                        name,
                        match error.as_str() {
                            "CoreTargetUnavailable" => "CoreTargetUnavailable",
                            "CoreSourceUnsupported" => "CoreSourceUnsupported",
                            _ => "CoreIdentityUnverified",
                        },
                    )
                }
            };
            // A source snapshot belongs to this exact file at this registration, even if a
            // stop/re-register or replace happened while other group members were queried.
            let same = registration_revision(root, name).is_ok_and(|value| value == registration)
                && instance_path(root, name).is_ok_and(|path| {
                    instance(root, name).is_ok_and(|item| core_artifact::jar(&path, &item) == jar)
                })
                && admitted(root, name).is_ok_and(|allowed| allowed)
                && instance(root, name).is_ok_and(|item| !item.removed)
                && no_redirect(&jar).is_ok()
                && content_revision::file_revision(&jar).is_ok_and(|hash| hash == profile.sha256);
            if !same {
                return unknown(name, "CoreIdentityChanged");
            }
            let mut row = result(name, &profile, &metadata[&key]);
            row["registrationRevision"] = json!(registration);
            row
        })
        .collect();
    Ok(json!({"rows":rows,"checkedAt":epoch_ms()}))
}

pub(super) fn install_latest(
    root: &Path,
    name: &str,
    request: &str,
    expected_current: &str,
    expected_candidate: &str,
    progress: &dyn Fn(CoreUpdatePhase),
) -> Result<Value> {
    if !instance_name(name) || !token(request) {
        return Err("InvalidCoreMaintenanceRequest".into());
    }
    let registration = registration_revision(root, name)?;
    install_latest_guarded(
        root, name, request, expected_current, expected_candidate, &registration,
        CandidateCache::Private, progress, || Ok(()),
    )
}

/// Where a downloaded update candidate is kept between installs (#60).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CandidateCache {
    /// One install: a Paper candidate is shared only through the prefetch store.
    Private,
    /// A batch item: a candidate that the prefetch store does not hold is kept in
    /// the batch candidate store, so a later item with the same SHA-256 reuses it.
    Batch,
}

/// Reports bytes as `Downloading`. `total` is the official size, when known.
fn report_download(progress: &dyn Fn(CoreUpdatePhase)) -> impl Fn(u64, Option<u64>) + '_ {
    move |received, total| progress(CoreUpdatePhase::Downloading { received, total })
}

pub(super) fn install_latest_guarded(
    root: &Path,
    name: &str,
    request: &str,
    expected_current: &str,
    expected_candidate: &str,
    registration: &str,
    cache: CandidateCache,
    progress: &dyn Fn(CoreUpdatePhase),
    begin_commit: impl FnOnce() -> Result<()>,
) -> Result<Value> {
    if !instance_name(name) || !token(request) {
        return Err("InvalidCoreMaintenanceRequest".into());
    }
    let store = core_prefetch::Store::enabled(root)?;
    let lease = store.as_ref().map(|store| store.lease(expected_candidate)).transpose()?;
    if let (Some(store), Some(lease)) = (&store, &lease) {
        if let Some(demand) = store.prepared(name, expected_current, registration, expected_candidate)? {
            if let Some(candidate) = lease.ready(expected_candidate)? {
                progress(CoreUpdatePhase::Reusing);
                let _writer = instance_writer_lock(root, name)?;
                if registration_revision(root, name)? != registration {
                    return Err("RegistrationChanged".into());
                }
                return core_maintenance::install_verified_guarded(
                    root, name, request, expected_current, candidate, &demand.profile, progress, begin_commit,
                );
            }
        }
    }
    progress(CoreUpdatePhase::Checking);
    let agent = configured_paper_user_agent()?;
    let response = check_with_agent(root, &[name.to_owned()], Ok(&agent))?;
    let row = response["rows"]
        .as_array()
        .and_then(|rows| rows.first())
        .ok_or("CoreSourceUnavailable")?;
    if row["sha256"].as_str() != Some(expected_current) {
        return Err("CoreArtifactChanged".into());
    }
    let candidate = &row["candidate"];
    let hash = candidate["sha256"]
        .as_str()
        .filter(|hash| sha256(hash))
        .ok_or("CoreCandidateUnverified")?;
    if hash != expected_candidate {
        return Err("CoreCandidateChanged".into());
    }
    let url = candidate["url"]
        .as_str()
        .filter(|url| candidate_url(url, hash))
        .ok_or("CoreCandidateUnverified")?;
    let family = row["family"].as_str().ok_or("CoreCandidateUnverified")?;
    let minecraft = row["minecraft"].as_str().ok_or("CoreCandidateUnverified")?;
    let build = candidate["build"]
        .as_str()
        .ok_or("CoreCandidateUnverified")?;
    let size = artifact_size(candidate);
    let profile = CoreProfile { unverified: false, java: None,
        family: family.to_owned(),
        minecraft: minecraft.to_owned(),
        build: build.to_owned(),
        sha256: hash.to_owned(),
    };

    // The prefetch store prepares Paper and Velocity. A batch keeps any other
    // candidate in its own store, so each SHA-256 is downloaded once per batch.
    let lease = match lease.filter(|_| prefetch_family(&profile.family)) {
        Some(lease) => Some(lease),
        None if cache == CandidateCache::Batch => {
            Some(core_prefetch::Store::batch(root)?.lease(&profile.sha256)?)
        }
        None => None,
    };
    if let Some(lease) = lease {
        let report = report_download(progress);
        let candidate = match lease.ready(&profile.sha256)? {
            Some(candidate) => {
                progress(CoreUpdatePhase::Reusing);
                candidate
            }
            None => lease.ensure(&profile.sha256, |path|
                download_verified_with(url, &agent, path, &profile.sha256, size, Some(&report))
            )?,
        };
        let _writer = instance_writer_lock(root, name)?;
        if registration_revision(root, name)? != registration {
            return Err("RegistrationChanged".into());
        }
        return core_maintenance::install_verified_guarded(
            root, name, request, expected_current, candidate, &profile, progress, begin_commit,
        );
    }

    let staging = child_directory(root, "staging")?;
    let stage = staging.join(format!("core-download-{}-{}", std::process::id(), clock()));
    fs::create_dir(&stage).map_err(fail)?;
    let output = stage.join("candidate.jar");
    let outcome = (|| {
        let report = report_download(progress);
        download_verified_with(url, &agent, &output, &profile.sha256, size, Some(&report))?;
        let _writer = instance_writer_lock(root, name)?;
        if registration_revision(root, name)? != registration {
            return Err("RegistrationChanged".into());
        }
        core_maintenance::install_verified_guarded(
            root, name, request, expected_current, &output, &profile, progress, begin_commit,
        )
    })();
    if let Err(error) = fs::remove_file(&output) {
        if error.kind() != io::ErrorKind::NotFound {
            eprintln!("Core download cleanup failed: {error}");
        }
    }
    if let Err(error) = fs::remove_dir(stage) {
        eprintln!("Core download staging cleanup failed: {error}");
    }
    outcome
}

pub(super) fn download_prefetch(url: &str, path: &Path, expected: &str) -> Result<()> {
    if !sha256(expected) || !candidate_url(url, expected) {
        return Err("CoreCandidateUnverified".into());
    }
    download_verified(url, &configured_paper_user_agent()?, path, expected)
}

/// Downloads a pinned plugin artifact. The caller supplies the policy's
/// official URL; the file is accepted only with the exact SHA-256.
pub(super) fn download_pinned(url: &str, path: &Path, expected: &str) -> Result<()> {
    if !sha256(expected) || !url.starts_with("https://") {
        return Err("TestPermissionArtifactUnverified".into());
    }
    download_verified(url, &configured_paper_user_agent()?, path, expected)
}

fn artifact_size(metadata: &Value) -> Option<u64> {
    metadata["size"].as_u64().filter(|size| (1..=MAX_ARTIFACT_BYTES).contains(size))
}

fn download_verified(url: &str, agent: &str, output: &Path, expected: &str) -> Result<()> {
    complete_download(download_command(url, agent, output)?, output, expected, None)
}

/// Like `download_verified`, and reports the received bytes to `report`.
fn download_verified_with(
    url: &str,
    agent: &str,
    output: &Path,
    expected: &str,
    size: Option<u64>,
    report: Option<&dyn Fn(u64, Option<u64>)>,
) -> Result<()> {
    let progress = report.map(|emit| (size, emit));
    complete_download(download_command(url, agent, output)?, output, expected, progress)
}

fn download_command(url: &str, agent: &str, output: &Path) -> Result<Command> {
    let mut command = Command::new(system_curl()?);
    command
        .args([
            "--disable",
            "--silent",
            "--fail",
            "--globoff",
            "--proto",
            "=https",
            "--tlsv1.2",
            "--connect-timeout",
            "5",
            "--max-time",
            "45",
            "--max-filesize",
            &MAX_ARTIFACT_BYTES.to_string(),
            "--user-agent",
            agent,
            "--write-out",
            "%{http_code}",
            "--output",
        ])
        .arg(output)
        .arg("--url")
        .arg(url)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .env_remove("SSLKEYLOGFILE")
        .env_remove("CURL_CA_BUNDLE")
        .env_remove("SSL_CERT_FILE")
        .env_remove("SSL_CERT_DIR")
        .creation_flags(HIDDEN)
        .stdout(Stdio::piped());
    Ok(command)
}

/// Runs a download and, with `progress`, reports the bytes written so far.
/// `total` is the size the official source states. Without one, only the
/// received bytes are reported, never a guessed total.
fn complete_download(
    mut command: Command,
    output: &Path,
    expected: &str,
    progress: Option<(Option<u64>, &dyn Fn(u64, Option<u64>))>,
) -> Result<()> {
    let mut child = command.spawn().map_err(fail)?;
    // The last reported byte count, so each report is strictly larger.
    let mut last = 0;
    if let Some((total, emit)) = progress {
        emit(0, total);
        while child.try_wait().map_err(fail)?.is_none() {
            if let Ok(metadata) = fs::metadata(output) {
                let length = metadata.len();
                if length > last && total.is_none_or(|total| length < total) {
                    emit(length, total);
                    last = length;
                }
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
    let reply = child.wait_with_output().map_err(fail)?;
    if !reply.status.success() || reply.stdout != b"200" {
        return Err("CoreDownloadUnverified".into());
    }
    if content_revision::file_revision(output)? != expected {
        return Err("CoreArtifactDigestMismatch".into());
    }
    if let Some((total, emit)) = progress {
        let length = fs::metadata(output).map_err(fail)?.len();
        if total.is_some_and(|total| length != total) {
            return Err("CoreArtifactSizeMismatch".into());
        }
        if length > last || total.is_some() {
            emit(length, total);
        }
    }
    Ok(())
}

pub(super) fn fixed_creation_target(family: &str, version: &str) -> Result<(CoreProfile, &'static str)> {
    let (hash, mojang_url) = match (family, version) {
        ("PAPER", "26.3") => (
            "a0a58f532f08054ea92eb916d3edb6ed3c86429d77175317304f6e995cce9968",
            "https://piston-data.mojang.com/v1/objects/33680f5f2ac32864d6d7cf5e56a705fdb3e05f4c/server.jar",
        ),
        ("PAPER", "26.2") => (
            "3cce0a4922baf79bd8e1275ee5ff9f073f24675770e09b0d4be9442d513e7a6b",
            "https://piston-data.mojang.com/v1/objects/823e2250d24b3ddac457a60c92a6a941943fcd6a/server.jar",
        ),
        ("FOLIA", "26.1.2") => (
            "607afd1c3320008e1ffd2eaee6780ace4419d5f8c527b75e79f259be79ebf57b",
            "https://piston-data.mojang.com/v1/objects/97ccd4c0ed3f81bbb7bfacddd1090b0c56f9bc51/server.jar",
        ),
        _ => return Err("UnsupportedCreationCore".into()),
    };
    Ok((
        pinned_core_profile(hash.into()).ok_or("UnverifiedCreationCore")?,
        mojang_url,
    ))
}

fn verify_cached(path: &Path, expected: &str) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(fail(error)),
        Ok(metadata) if !metadata.is_file() => return Err("CoreDownloadCacheInvalid".into()),
        Ok(_) => (),
    }
    no_redirect(path)?;
    if content_revision::file_revision(path)? != expected {
        return Err("CoreDownloadCacheInvalid".into());
    }
    Ok(true)
}

pub(super) fn download_directory(root: &Path, profile: &CoreProfile) -> Result<PathBuf> {
    Ok(child_directory(root, "staging")?.join(format!("download-{}",profile.sha256)))
}

pub(super) fn publish_downloads(root: &Path, profile: &CoreProfile, mojang_url: &str) -> Result<()> {
    let staging = child_directory(root, "staging")?;
    let directory = download_directory(root, profile)?;
    let expected_mojang = profile.mojang_sha256().ok_or("MissingMojangArtifact")?;
    let paths = [directory.join("server.jar"), directory.join("mojang.jar")];
    let _lock = command_lock(root, &format!("download-{}.lock", profile.sha256), false)?;
    if directory.exists() {
        no_redirect(&directory)?;
        if verify_cached(&paths[0], &profile.sha256)? && verify_cached(&paths[1], expected_mojang)?
        {
            return Ok(());
        }
        return Err("CoreDownloadCacheInvalid".into());
    }
    let agent = configured_paper_user_agent()?;
    let key = source_key(profile).ok_or("CoreSourceUnsupported")?;
    let metadata = fetch_group(&key, Ok(&agent), &fetch_json);
    let Release::Paper(builds) = metadata.release? else {
        return Err("CoreSourceUnsupported".into());
    };
    let build = builds
        .iter()
        .find(|build| {
            profile.build.parse::<u64>().ok() == Some(build.id)
                && build.sha256.as_deref() == Some(profile.sha256.as_str())
        })
        .ok_or("CoreCandidateUnverified")?;
    let url = build.url.as_deref().ok_or("CoreCandidateUnverified")?;
    let scratch = staging.join(format!(
        "pending-download-{}-{}",
        std::process::id(),
        clock()
    ));
    fs::create_dir(&scratch).map_err(fail)?;
    let server = scratch.join("server.jar");
    let mojang = scratch.join("mojang.jar");
    let outcome = (|| {
        download_verified(url, &agent, &server, &profile.sha256)?;
        download_verified(mojang_url, &agent, &mojang, expected_mojang)?;
        fs::rename(&scratch, &directory).map_err(fail)
    })();
    if outcome.is_err() {
        for file in [&server, &mojang] {
            if let Err(error) = fs::remove_file(file) {
                if error.kind() != io::ErrorKind::NotFound {
                    eprintln!("Creation download cleanup failed: {error}");
                }
            }
        }
        if let Err(error) = fs::remove_dir(&scratch) {
            eprintln!("Creation download staging cleanup failed: {error}");
        }
    }
    outcome
}

pub(super) fn prepare_creation_download(
    root: &Path,
    profile: &CoreProfile,
    progress: &dyn Fn(CreationPhase),
) -> Result<(PathBuf, Option<(PathBuf, String)>)> {
    creation_assets::prepare(root, profile, progress)
}

pub(super) fn create_download(root: &Path, args: &[String]) -> Result<Value> {
    let [name, request, java_path, port, family, minecraft] = args else {
        return Err("InvalidProvisionArguments".into());
    };
    if !instance_name(name) || !token(request) || !port.parse::<u16>().is_ok_and(|port| port != 0) {
        return Err("InvalidProvisionArguments".into());
    }
    let (profile, mojang_url) = fixed_creation_target(family, minecraft)?;
    let directory = download_directory(root, &profile)?;
    let receipt = root.join("jobs").join(format!("{request}.json"));
    if !receipt.exists() {
        java(java_path)?;
        publish_downloads(root, &profile, mojang_url)?;
    }
    dispatch(&[
        root.to_string_lossy().into_owned(),
        "create".into(),
        name.clone(),
        request.clone(),
        java_path.clone(),
        port.clone(),
        directory.join("server.jar").to_string_lossy().into_owned(),
        directory.join("mojang.jar").to_string_lossy().into_owned(),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCAL: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const CANDIDATE: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const TEST_AGENT: &str = "InstanceHarbor/2.0 (https://operator.example)";

    fn profile(family: &str, version: &str, build: &str, hash: &str) -> CoreProfile {
        CoreProfile { unverified: false, java: None,
            family: family.into(),
            minecraft: version.into(),
            build: build.into(),
            sha256: hash.into(),
        }
    }

    fn build(id: u64, channel: &str, hash: &str, url: Option<&str>) -> Value {
        let url = url.map(str::to_owned).unwrap_or_else(|| {
            format!("https://fill-data.papermc.io/v1/objects/{hash}/paper-26.3-{id}.jar")
        });
        json!({"id":id,"channel":channel,"downloads":{"server:default":{
            "checksums":{"sha256":hash},"url":url
        }}})
    }

    #[test]
    fn an_unlisted_core_is_trusted_only_by_an_exact_catalog_hash() {
        let wanted = "1b243442fc3b3cbe945628f8af7911e4be439ca8b22106fb1568ac81d5a5792b";
        let other = "a".repeat(64);
        let seen = Mutex::new(Vec::new());
        let fetch = |url: &str, _: &str| {
            seen.lock().unwrap().push(url.to_owned());
            Ok(json!([build(201, "STABLE", &other, None), build(202, "STABLE", wanted, None)]))
        };
        let found = catalog_profile_with("PAPER", "26.3", wanted, &fetch).unwrap().unwrap();
        assert_eq!((found.family.as_str(), found.minecraft.as_str(), found.build.as_str()), ("PAPER", "26.3", "202"));
        assert_eq!(seen.lock().unwrap()[0], format!("{FILL_PROJECTS}/paper/versions/26.3/builds"));
        assert!(catalog_profile_with("PAPER", "26.3", &"b".repeat(64), &fetch).unwrap().is_none());
        // Spigot has no official binary catalog, and a malformed version is never sent.
        assert!(catalog_profile_with("SPIGOT", "1.8.8", wanted, &fetch).unwrap().is_none());
        assert!(catalog_profile_with("PAPER", "26.3/../x", wanted, &fetch).unwrap().is_none());
        assert_eq!(seen.lock().unwrap().len(), 2);
        let offline = |_: &str, _: &str| -> Result<Value> { Err("network".into()) };
        assert_eq!(catalog_profile_with("VELOCITY", "4.2.1-SNAPSHOT", wanted, &offline).err().as_deref(), Some("CoreCatalogUnavailable"));
    }

    #[test]
    fn isolated_curl_reports_real_bytes_only_with_a_trusted_total() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let chunk = [b'a'; 32 * 1024];
        let expected = content_revision::revision(&chunk.repeat(3)).unwrap();
        let fixture = std::env::temp_dir().join(format!("ih-download-{}-{}", std::process::id(), clock()));
        fs::create_dir(&fixture).unwrap();
        let fixture = safe_directory(&fixture).unwrap();

        let server = thread::spawn(move || {
            for known in [true, false] {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                let mut request = [0; 4096];
                assert!(stream.read(&mut request).unwrap() > 0);
                if known {
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", chunk.len() * 3).unwrap();
                } else {
                    stream.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n").unwrap();
                }
                for _ in 0..3 {
                    stream.write_all(&chunk).unwrap();
                    thread::sleep(Duration::from_millis(220));
                }
            }
        });
        for known in [true, false] {
            let output = fixture.join(format!("{known}.jar"));
            let mut command = download_command(&format!("http://{address}/jar"), "InstanceHarbor/test", &output).unwrap();
            // Only the test transport accepts plain HTTP, bound to this ephemeral loopback listener.
            command.args(["--proto", "=http", "--noproxy", "*"]);
            let metadata = if known { json!({ "size": chunk.len() * 3 }) } else { json!({}) };
            let received = std::cell::RefCell::new(Vec::new());
            let report = |bytes, limit| received.borrow_mut().push((bytes, limit));
            // #60: without an official size, bytes are still reported, with no total.
            let progress = Some((artifact_size(&metadata), &report as &dyn Fn(u64, Option<u64>)));
            let result = complete_download(command, &output, &expected, progress);
            if output.exists() {
                fs::remove_file(&output).unwrap();
            }
            result.unwrap();
            let totals = received.into_inner();
            let length = (chunk.len() * 3) as u64;
            let total = known.then_some(length);
            assert_eq!(totals.first(), Some(&(0, total)));
            assert_eq!(totals.last(), Some(&(length, total)));
            assert!(totals.len() > 2, "{totals:?}");
            assert!(totals.windows(2).all(|pair| pair[0].0 < pair[1].0));
            assert!(totals.iter().all(|(_, limit)| *limit == total));
        }
        server.join().unwrap();
        fs::remove_dir(&fixture).unwrap();
    }

    #[test]
    fn creation_versions_expect_the_requested_project_and_sort_release_components() {
        let response = json!({
            "project": {"id": "paper"},
            "versions": {"1.21": ["1.21.8", "1.21.11", "1.21.11-rc3"],
                         "26.1": ["26.1.1", "26.1.2", "26.1.2"], "26.3": ["26.3", "26.3-rc-1"]}
        });
        assert!(project_versions(response.clone(), "folia").is_err());
        assert_eq!(project_versions(response, "paper").unwrap(),
            ["26.3", "26.1.2", "26.1.1", "1.21.11", "1.21.8"]);
    }

    #[test]
    fn creation_preview_requires_newest_stable_and_confirm_requires_same_build() {
        let builds = paper_builds(json!([
            build(21, "EXPERIMENTAL", CANDIDATE, None),
            build(10, "STABLE", LOCAL, None),
            build(20, "STABLE", CANDIDATE, None)
        ])).unwrap();
        assert_eq!(latest_stable(&builds, "PAPER", "26.3").unwrap().id, 20);
        assert!(exact_stable_download(&builds, &profile("PAPER", "26.3", "10", LOCAL)).is_ok());
        assert!(exact_stable_download(&builds, &profile("PAPER", "26.3", "10", CANDIDATE)).is_err());
        assert!(exact_stable_download(&builds, &profile("PAPER", "26.3", "21", CANDIDATE)).is_err());
        assert!(exact_stable_download(&builds, &profile("FOLIA", "26.3", "20", CANDIDATE)).is_err());

        let unavailable = paper_builds(json!([build(21, "EXPERIMENTAL", CANDIDATE, None)])).unwrap();
        assert!(matches!(latest_stable(&unavailable, "PAPER", "26.3"), Err(error) if error == "CreationNoStableBuild"));
        let broken_latest = paper_builds(json!([
            build(18, "STABLE", LOCAL, None),
            build(22, "STABLE", CANDIDATE, Some("https://example.com/paper.jar"))
        ])).unwrap();
        assert!(matches!(latest_stable(&broken_latest, "PAPER", "26.3"), Err(error) if error == "CoreCandidateUnverified"));
    }

    #[test]
    fn https_contact_does_not_allow_userinfo_or_header_delimiters() {
        assert!(contact_url(
            "https://operator.example/contact?q=InstanceHarbor"
        ));
        for unsafe_contact in [
            "http://operator.example",
            "https://127.0.0.1",
            "https://a.example@b.example",
            "https://a.example/\r\nX: value",
            "https://a.example/)",
            "https://bad-.example",
        ] {
            assert!(!contact_url(unsafe_contact), "{unsafe_contact}");
        }
    }

    #[test]
    fn absent_contact_uses_default_and_valid_explicit_contact_overrides_it() {
        let existing = profile("PAPER", "26.3", "205", LOCAL);
        let key = source_key(&existing).unwrap();
        for (contact, expected) in [
            (None, "InstanceHarbor/2.0 (https://github.com/Command1264/InstanceHarbor)"),
            (Some(OsStr::new("https://operator.example")), TEST_AGENT),
        ] {
            let agent = paper_user_agent(contact).unwrap();
            let source = fetch_group(&key, Ok(&agent), &|_, actual| {
                assert_eq!(actual, expected);
                Ok(json!([build(205, "STABLE", LOCAL, None)]))
            });
            assert_eq!(result("target", &existing, &source)["status"], "Current");
        }
    }

    #[test]
    fn stable_result_requires_exact_local_hash_and_verified_newest_artifact() {
        let existing = profile("PAPER", "26.3", "205", LOCAL);
        let key = source_key(&existing).unwrap();
        let fetch = |url: &str, ua: &str| {
            assert_eq!(
                url,
                "https://fill.papermc.io/v3/projects/paper/versions/26.3/builds"
            );
            assert_eq!(ua, "InstanceHarbor/2.0 (https://operator.example/contact)");
            Ok(json!([
                build(230, "ALPHA", &"f".repeat(64), None),
                build(222, "STABLE", CANDIDATE, None),
                build(205, "STABLE", LOCAL, None)
            ]))
        };
        let agent = paper_user_agent(Some(OsStr::new("https://operator.example/contact"))).unwrap();
        let source = fetch_group(&key, Ok(&agent), &fetch);
        let reply = result("first", &existing, &source);
        assert_eq!(reply["status"], "UpdateAvailable");
        assert_eq!(reply["candidate"]["build"], "222");
        let newest = profile("PAPER", "26.3", "222", CANDIDATE);
        assert_eq!(result("second", &newest, &source)["status"], "Current");
        let wrong = profile("PAPER", "26.3", "205", &"1".repeat(64));
        assert_eq!(
            result("wrong", &wrong, &source)["reason"],
            "CoreSourceUnverifiedLocalHash"
        );

        let poisoned = fetch_group(&key, Ok(TEST_AGENT), &|_, _| {
            Ok(json!([
                build(205, "STABLE", LOCAL, None),
                build(210, "STABLE", &"2".repeat(64), None),
                build(
                    222,
                    "STABLE",
                    CANDIDATE,
                    Some("https://untrusted.example/test.jar")
                )
            ]))
        });
        let refused = result("first", &existing, &poisoned);
        assert_eq!(refused["reason"], "CoreSourceUnverifiedCandidate");
        assert!(refused.get("candidate").is_none());
    }

    #[test]
    fn spigot_distance_is_independent_of_same_version_release() {
        let profile = profile("SPIGOT", "1.8.8", "582b", SPIGOT_188_HASH);
        let key = source_key(&profile).unwrap();
        let metadata = fetch_group(&key, Err("CoreSourceInvalidContact"), &|url, _| match url {
            "https://hub.spigotmc.org/versions/1.8.8.json" => Ok(
                json!({"name":"582b","refs":{"Spigot":SPIGOT_REF,"CraftBukkit":CRAFTBUKKIT_REF}}),
            ),
            url if url.ends_with("/craftbukkit/commits?since=741a1bd&withCounts=true") => {
                Ok(json!({"totalCount":19}))
            }
            url if url.ends_with("/spigot/commits?since=3c60ece&withCounts=true") => {
                Ok(json!({"totalCount":33}))
            }
            _ => panic!("unexpected request {url}"),
        });
        let row = result("old", &profile, &metadata);
        assert_eq!(row["status"], "NativeBehind");
        assert_eq!(row["releaseMatch"], true);
        assert_eq!(row["nativeVersion"], json!({"status":"Behind","behind":52}));
        assert!(row.get("candidate").is_none());
    }

    #[test]
    fn spigot_current_requires_matching_local_refs_and_zero_native_distance() {
        let profile = profile("SPIGOT", "1.8.8", "582b", SPIGOT_188_HASH);
        let key = source_key(&profile).unwrap();
        let fetch = |url: &str, _: &str| {
            if url.ends_with("/1.8.8.json") {
                Ok(json!({"name":"582b","refs":{"Spigot":SPIGOT_REF,"CraftBukkit":CRAFTBUKKIT_REF}}))
            } else {
                Ok(json!({"totalCount":0}))
            }
        };
        let metadata = fetch_group(&key, Err("CoreSourceInvalidContact"), &fetch);
        assert_eq!(result("old", &profile, &metadata)["status"], "Current");
        let mismatched = fetch_group(&key, Err("CoreSourceInvalidContact"), &|url, _| {
            if url.ends_with("/1.8.8.json") {
                Ok(json!({"name":"582b","refs":{"Spigot":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","CraftBukkit":CRAFTBUKKIT_REF}}))
            } else {
                Ok(json!({"totalCount":0}))
            }
        });
        let row = result("old", &profile, &mismatched);
        assert_eq!(row["status"], "Unknown");
        assert_eq!(row["reason"], "CoreSourceSpigotRevisionsDiffer");
        assert!(row.get("candidate").is_none());
        let failed_distance = fetch_group(&key, Err("CoreSourceInvalidContact"), &|url, _: &str| {
            if url.ends_with("/1.8.8.json") {
                Ok(json!({"name":"582b","refs":{"Spigot":SPIGOT_REF,"CraftBukkit":CRAFTBUKKIT_REF}}))
            } else {
                Err("CoreSourceHttpError".into())
            }
        });
        let row = result("old", &profile, &failed_distance);
        assert_eq!(row["status"], "Unknown");
        assert_eq!(row["reason"], "CoreSourceSpigotNativeCheckFailed");
        assert_eq!(row["nativeVersion"]["reason"], "CoreSourceHttpError");
    }

    #[test]
    fn other_pinned_spigot_version_is_queried_but_local_refs_stay_unverified() {
        let pinned = pinned_core_profile("27a348344d42ed2ba76aad6fceebb7a26365f09c9ac6eea4c0918ba83ba7e3dd".into()).unwrap();
        let key = source_key(&pinned).unwrap();
        assert_eq!(group_url(&key), "https://hub.spigotmc.org/versions/26.2.json");
        let metadata = fetch_group(&key, Err("CoreSourceInvalidContact"), &|url, _| {
            assert_eq!(url, "https://hub.spigotmc.org/versions/26.2.json");
            Ok(json!({"name":"4648","refs":{"Spigot":"8db49a2b72ef138a615856934957da2978f72f08","CraftBukkit":"96ae8cd2f4f8bfcee57193455d0714f2bd4fcd1f"}}))
        });
        let row = result("new", &pinned, &metadata);
        assert_eq!(row["remoteBuild"], "4648");
        assert_eq!(row["status"], "Unknown");
        assert_eq!(row["reason"], "CoreSourceSpigotLocalRefsUnverified");
        assert!(row.get("candidate").is_none());
        assert!(row.get("nativeVersion").is_none());
        // A locally hash-bound but unpinned Spigot core may still look up its
        // version metadata. It cannot claim source parity or an install target.
        let unpinned = profile("SPIGOT", "1.14", "2096", LOCAL);
        let key = source_key(&unpinned).unwrap();
        assert_eq!(group_url(&key), "https://hub.spigotmc.org/versions/1.14.json");
        let older_metadata = fetch_group(&key, Err("CoreSourceInvalidContact"), &|url, _: &str| {
            assert_eq!(url, "https://hub.spigotmc.org/versions/1.14.json");
            Ok(json!({"name":"2096","refs":{"Spigot":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","CraftBukkit":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}}))
        });
        let row = result("unpinned", &unpinned, &older_metadata);
        assert_eq!(row["status"], "Unknown");
        assert_eq!(row["reason"], "CoreSourceSpigotLocalRefsUnverified");
        assert!(row.get("candidate").is_none());
    }

    #[test]
    fn each_group_fetches_once_and_failure_keeps_other_group_results() {
        let profiles = [
            profile("PAPER", "26.3", "205", LOCAL),
            profile("PAPER", "26.3", "222", CANDIDATE),
            profile("FOLIA", "26.1.2", "8", LOCAL),
        ];
        let keys = profiles
            .iter()
            .map(|profile| source_key(profile).unwrap())
            .collect();
        let calls = Mutex::new(Vec::new());
        let source = groups(keys, Ok(TEST_AGENT), &|url, _| {
            calls.lock().unwrap().push(url.to_owned());
            if url.contains("/folia/") {
                return Err("CoreSourceTimeout".into());
            }
            Ok(json!([
                build(205, "STABLE", LOCAL, None),
                build(222, "STABLE", CANDIDATE, None)
            ]))
        });
        assert_eq!(calls.into_inner().unwrap().len(), 2);
        for (profile, status) in profiles
            .iter()
            .zip(["UpdateAvailable", "Current", "Unknown"])
        {
            assert_eq!(
                result("target", profile, &source[&source_key(profile).unwrap()])["status"],
                status
            );
        }
    }

    #[test]
    fn invalid_explicit_contact_prevents_http() {
        let existing = profile("PAPER", "26.3", "205", LOCAL);
        let key = source_key(&existing).unwrap();
        let non_unicode = OsString::from_wide(&[0xd800]);
        for contact in [OsStr::new(""), OsStr::new("http://operator.example"), &non_unicode] {
            let agent = paper_user_agent(Some(contact));
            let refused = fetch_group(
                &key,
                agent.as_deref().map_err(String::as_str),
                &|_, _| panic!("invalid override must not trigger HTTP"),
            );
            assert_eq!(result("target", &existing, &refused)["reason"], "CoreSourceInvalidContact");
        }
    }

    // #41: Velocity -SNAPSHOT versions publish only BETA builds. They are
    // offered within the same version; ALPHA never is, and Paper keeps STABLE only.
    #[test]
    fn velocity_offers_the_newest_beta_build_of_the_same_version() {
        let velocity = |id: u64, channel: &str, hash: &str| build(id, channel, hash, Some(&format!(
            "https://fill-data.papermc.io/v1/objects/{hash}/velocity-4.2.1-SNAPSHOT-{id}.jar")));
        let existing = profile("VELOCITY", "4.2.1-SNAPSHOT", "38", LOCAL);
        let key = source_key(&existing).unwrap();
        assert_eq!(group_url(&key), "https://fill.papermc.io/v3/projects/velocity/versions/4.2.1-SNAPSHOT/builds");
        let source = fetch_group(&key, Ok(TEST_AGENT), &|_, _| Ok(json!([
            velocity(44, "ALPHA", &"f".repeat(64)),
            velocity(43, "BETA", CANDIDATE),
            velocity(38, "BETA", LOCAL),
        ])));
        let row = result("proxy", &existing, &source);
        assert_eq!((row["status"].as_str(), row["candidate"]["build"].as_str()), (Some("UpdateAvailable"), Some("43")));
        assert_eq!(result("proxy", &profile("VELOCITY", "4.2.1-SNAPSHOT", "43", CANDIDATE), &source)["status"], "Current");
        // The same channels for Paper offer nothing newer than STABLE.
        let paper = profile("PAPER", "26.3", "205", LOCAL);
        let paper_source = fetch_group(&source_key(&paper).unwrap(), Ok(TEST_AGENT), &|_, _| Ok(json!([
            build(222, "BETA", CANDIDATE, None), build(205, "STABLE", LOCAL, None),
        ])));
        assert_eq!(result("server", &paper, &paper_source)["status"], "Current");
        assert!(prefetch_family("VELOCITY") && prefetch_family("PAPER") && !prefetch_family("FOLIA"));
    }

    #[test]
    fn ambiguous_artifacts_cannot_be_candidates() {
        let existing = profile("PAPER", "26.3", "205", LOCAL);
        let key = source_key(&existing).unwrap();
        let ambiguous = fetch_group(&key, Ok(TEST_AGENT), &|_, _| {
            Ok(json!([
                build(205, "STABLE", LOCAL, None),
                build(222, "STABLE", CANDIDATE, None),
                build(230, "STABLE", CANDIDATE, None),
            ]))
        });
        let row = result("target", &existing, &ambiguous);
        assert_eq!(row["reason"], "CoreSourceUnverifiedCandidate");
        assert!(row.get("candidate").is_none());
    }
}
