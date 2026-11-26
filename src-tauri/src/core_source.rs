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
#[path = "core_source/spigot_revision.rs"]
mod spigot_revision;
use spigot_revision::SpigotRevision;

/// The official Purpur API. v2 publishes MD5 only (#41).
const PURPUR_API: &str = "https://api.purpurmc.org/v2/purpur";
/// The public directory BuildTools reads: one `<Minecraft version>.json` per version
/// line and `latest.json` for the newest one (#75).
const SPIGOT_VERSIONS: &str = "https://hub.spigotmc.org/versions";
const SPIGOT_AGENT: &str = "InstanceHarbor/2.0";

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

/// A Purpur build whose official build result is `SUCCESS`.
struct PurpurBuild {
    id: u64,
    md5: String,
}

enum Release {
    Paper(Vec<PaperBuild>),
    Purpur(Vec<PurpurBuild>),
    Spigot { build: String, spigot_ref: String, craftbukkit_ref: String },
}

struct Metadata {
    url: String,
    release: Result<Release>,
}

/// What a Spigot row is compared with besides its own version file: the commits
/// read from the JAR, and `latest.json`, fetched once per check.
struct SpigotLocal<'a> {
    revision: &'a Result<SpigotRevision>,
    latest: Option<&'a Result<Release>>,
}

/// What a check reads from the local JAR beyond its SHA-256.
enum Local<'a> {
    /// Purpur: the MD5 its API publishes (#41).
    Md5(&'a str),
    /// Spigot: the revisions the JAR states (#75).
    Spigot(SpigotLocal<'a>),
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

pub(super) fn system_curl() -> Result<PathBuf> {
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

/// SpigotMC refuses some automated clients, so a refusal and a missing version
/// file are told apart from other HTTP failures (#75). No retries.
fn fetch_spigot_json(url: &str, user_agent: &str) -> Result<Value> {
    let (code, body) = fetch_status(url, user_agent, false)?;
    spigot_response(code, &body)
}

fn spigot_response(code: u16, body: &[u8]) -> Result<Value> {
    match code {
        200 => serde_json::from_slice(body).map_err(|_| "CoreSourceInvalidJson".into()),
        404 => Err("CoreSourceSpigotVersionMissing".into()),
        401 | 403 | 429 => Err("CoreSourceRefused".into()),
        _ => Err("CoreSourceHttpError".into()),
    }
}

fn fetch_source_json(url: &str, user_agent: &str) -> Result<Value> {
    if url.starts_with(SPIGOT_VERSIONS) {
        fetch_spigot_json(url, user_agent)
    } else {
        fetch_json(url, user_agent)
    }
}

/// One public JSON lookup that reports the HTTP status instead of failing on
/// it, so the caller can tell "not found" from "rate limited". No retries.
pub(super) fn fetch_profile(url: &str) -> Result<(u16, Vec<u8>)> {
    fetch_status(url, &configured_paper_user_agent()?, false)
}

pub(super) fn fetch_status(url: &str, user_agent: &str, fail_on_http: bool) -> Result<(u16, Vec<u8>)> {
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
        "PAPER" | "FOLIA" | "VELOCITY" | "SPIGOT" | "PURPUR"
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

fn md5_text(text: &str) -> bool {
    text.len() == 32 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The successful builds of one Purpur version from `?detailed=true`.
fn purpur_builds(value: Value, version: &str) -> Result<Vec<PurpurBuild>> {
    if value["project"] != "purpur" || value["version"] != version {
        return Err("CoreSourceInvalidBuilds".into());
    }
    let rows = value["builds"]["all"].as_array().ok_or("CoreSourceInvalidBuilds")?;
    let mut builds = BTreeMap::new();
    for row in rows {
        let id = row["build"].as_str().and_then(|id| {
            (!id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit())).then(|| id.parse::<u64>().ok()).flatten()
        }).ok_or("CoreSourceInvalidBuilds")?;
        if row["result"] != "SUCCESS" {
            continue;
        }
        let md5 = row["md5"].as_str().filter(|text| md5_text(text)).ok_or("CoreSourceInvalidBuilds")?;
        if builds.insert(id, PurpurBuild { id, md5: md5.to_ascii_lowercase() }).is_some() {
            return Err("CoreSourceAmbiguousBuilds".into());
        }
    }
    Ok(builds.into_values().collect())
}

/// The official download of one Purpur build.
fn purpur_url(version: &str, build: u64) -> String {
    format!("{PURPUR_API}/{version}/{build}/download")
}

fn spigot_release(value: Value) -> Result<Release> {
    let build = value["name"]
        .as_str()
        .filter(|name| spigot_revision::build_name(name))
        .ok_or("CoreSourceInvalidSpigotRefs")?;
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

fn group_url(key: &SourceKey) -> String {
    if key.family == "SPIGOT" {
        return format!("{SPIGOT_VERSIONS}/{}.json", key.version);
    }
    if key.family == "PURPUR" {
        return format!("{PURPUR_API}/{}?detailed=true", key.version);
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
            }
        }
    };
    if key.family == "SPIGOT" {
        let release = fetch(&url, SPIGOT_AGENT).and_then(spigot_release);
        return Metadata { url, release };
    }
    let release = match paper_agent {
        Ok(agent) if key.family == "PURPUR" => fetch(&url, agent)
            .and_then(|value| purpur_builds(value, &key.version))
            .map(Release::Purpur),
        Ok(agent) => {
            fetch(&url, agent)
                .and_then(paper_builds)
                .map(Release::Paper)
        }
        Err(error) => Err(error.into()),
    };
    Metadata { url, release }
}

fn spigot_latest(fetch: &impl Fn(&str, &str) -> Result<Value>) -> Result<Release> {
    let _slot = SourceSlot::acquire()?;
    fetch(&format!("{SPIGOT_VERSIONS}/latest.json"), SPIGOT_AGENT).and_then(spigot_release)
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
        Release::Spigot { .. } | Release::Purpur(_) => Err("UnsupportedCreationCore".into()),
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
        Ok(Release::Spigot { .. } | Release::Purpur(_)) => return Ok(None),
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

/// Whether Spigot's newest version line is this one. `latest.json` names only a
/// Jenkins build, so a newer line is reported by its build, not a version name.
fn latest_minecraft(latest: &Result<Release>, spigot: &str, craftbukkit: &str) -> Value {
    match latest {
        Ok(Release::Spigot { build, spigot_ref, craftbukkit_ref }) => {
            let same = spigot_ref == spigot && craftbukkit_ref == craftbukkit;
            json!({"status": if same { "Same" } else { "Newer" }, "build": build})
        }
        Ok(Release::Paper(_) | Release::Purpur(_)) => json!({"status":"Unknown","reason":"CoreSourceInvalidSpigotRefs"}),
        Err(error) => json!({"status":"Unknown","reason":error}),
    }
}

fn result(name: &str, profile: &CoreProfile, metadata: &Metadata, local: Option<&Local>) -> Value {
    let local_md5 = match local { Some(Local::Md5(md5)) => Some(*md5), _ => None };
    let spigot = match local { Some(Local::Spigot(spigot)) => Some(spigot), _ => None };
    let mut row = json!({"name":name,"family":profile.family,"minecraft":profile.minecraft,
        "build":profile.build,"sha256":profile.sha256,"source":metadata.url,"status":"Unknown","reason":null});
    match &metadata.release {
        Err(error) => row["reason"] = json!(error),
        Ok(Release::Purpur(builds)) => {
            // The local build is the one whose official MD5 matches the JAR.
            // A candidate carries its MD5; its SHA-256 is computed after download.
            row["verification"] = json!("purpurMd5");
            let local = local_md5.and_then(|md5| builds.iter().find(|build| build.md5 == md5));
            let newest = builds.iter().max_by_key(|build| build.id);
            match (local, newest) {
                (None, _) => row["reason"] = json!("CoreSourceUnverifiedLocalHash"),
                (Some(_), None) => row["status"] = json!("NoStableBuild"),
                (Some(local), Some(newest)) => {
                    row["build"] = json!(local.id.to_string());
                    row["remoteBuild"] = json!(newest.id.to_string());
                    if local.id == newest.id {
                        row["status"] = json!("Current");
                    } else if local.id < newest.id
                        && builds.iter().filter(|build| build.md5 == newest.md5).count() == 1
                    {
                        row["status"] = json!("UpdateAvailable");
                        row["candidate"] = json!({"build":newest.id.to_string(),"md5":newest.md5,
                            "url":purpur_url(&profile.minecraft, newest.id)});
                    } else if local.id < newest.id {
                        row["reason"] = json!("CoreSourceUnverifiedCandidate");
                    } else {
                        row["reason"] = json!("CoreSourceLocalNewerThanStable");
                    }
                }
            }
        }
        Ok(Release::Spigot { build, spigot_ref, craftbukkit_ref }) => {
            row["remoteBuild"] = json!(build);
            if let Some(latest) = spigot.and_then(|local| local.latest) {
                row["latestMinecraft"] = latest_minecraft(latest, spigot_ref, craftbukkit_ref);
            }
            // The JAR states the first seven digits of the commits it was built
            // from; both must match this version line's newest build (#75).
            match spigot.map(|local| local.revision) {
                Some(Ok(local)) => {
                    row["localBuild"] = json!(local.build);
                    row["localRevision"] =
                        json!({"spigot":local.spigot,"craftBukkit":local.craftbukkit});
                    let current = spigot_ref.get(..7) == Some(local.spigot.as_str())
                        && craftbukkit_ref.get(..7) == Some(local.craftbukkit.as_str());
                    row["status"] = json!(if current { "Current" } else { "BuildAvailable" });
                }
                Some(Err(error)) => row["reason"] = json!(error),
                None => row["reason"] = json!(spigot_revision::UNREADABLE),
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
        });
    }
    metadata
}

/// Spigot has no official binary to match by hash, so an unlisted Spigot core is
/// compared by what its JAR states (#75). It never becomes an install target.
fn spigot_profile(jar: &Path, sha256: String) -> Option<CoreProfile> {
    let identity = core_identity::read_identity(jar).ok().flatten()?;
    (identity.family == "Spigot").then(|| CoreProfile {
        family: "SPIGOT".into(),
        minecraft: identity.minecraft_version,
        build: identity.build,
        sha256,
        unverified: true,
        java: None,
    })
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
            let hash = content_revision::file_revision(&jar)?;
            // An unverified Purpur is checked by the official MD5 of its JAR (#41),
            // an unlisted Spigot by the revisions its JAR states (#75).
            let profile = core_maintenance::trusted_profile(&config, hash.clone())
                .or_else(|| core_maintenance::purpur_identity(&config, &jar, &hash))
                .or_else(|| spigot_profile(&jar, hash.clone()))
                .ok_or("CoreIdentityUnverified")?;
            let key = source_key(&profile).ok_or("CoreSourceUnsupported")?;
            let md5 = (profile.family == "PURPUR").then(|| content_revision::file_md5(&jar)).transpose()?;
            let revision = (profile.family == "SPIGOT").then(|| spigot_revision::read(&jar));
            keys.insert(key.clone());
            Ok((jar, profile, key, registration, md5, revision))
        })();
        observed.push((name, identity));
    }
    let spigot = keys.iter().any(|key| key.family == "SPIGOT");
    let metadata = groups(keys, paper_agent, &fetch_source_json);
    // One `latest.json` read serves every Spigot row of this check.
    let latest = spigot.then(|| spigot_latest(&fetch_source_json));
    let rows: Vec<_> = observed
        .into_iter()
        .map(|(name, identity)| {
            let (jar, profile, key, registration, md5, revision) = match identity {
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
            let local = match (&md5, &revision) {
                (Some(md5), _) => Some(Local::Md5(md5)),
                (_, Some(revision)) => Some(Local::Spigot(SpigotLocal { revision, latest: latest.as_ref() })),
                _ => None,
            };
            let mut row = result(name, &profile, &metadata[&key], local.as_ref());
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

#[allow(clippy::too_many_arguments)]
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
    // A Purpur candidate is named by its MD5 and never prepared in advance.
    let lease = store.as_ref().filter(|_| sha256(expected_candidate))
        .map(|store| store.lease(expected_candidate)).transpose()?;
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
    if row["family"] == "PURPUR" {
        return install_purpur(root, name, request, expected_current, expected_candidate, registration, row, &agent, progress, begin_commit);
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

/// Purpur (#41): the candidate is named by the official MD5 of its build. The
/// download uses HTTPS and must match that MD5; its SHA-256 is then computed and
/// registered, so later checks and shared cores use SHA-256 only.
#[allow(clippy::too_many_arguments)]
fn install_purpur(
    root: &Path,
    name: &str,
    request: &str,
    expected_current: &str,
    expected_md5: &str,
    registration: &str,
    row: &Value,
    agent: &str,
    progress: &dyn Fn(CoreUpdatePhase),
    begin_commit: impl FnOnce() -> Result<()>,
) -> Result<Value> {
    let candidate = &row["candidate"];
    let md5 = candidate["md5"].as_str().filter(|md5| md5_text(md5)).ok_or("CoreCandidateUnverified")?;
    if !md5.eq_ignore_ascii_case(expected_md5) {
        return Err("CoreCandidateChanged".into());
    }
    let field = |value: &Value| value.as_str().map(str::to_owned).ok_or_else(|| "CoreCandidateUnverified".to_owned());
    let minecraft = field(&row["minecraft"])?;
    let local_build = field(&row["build"])?;
    let build = field(&candidate["build"])?;
    let id = build.parse::<u64>().map_err(|_| "CoreCandidateUnverified")?;
    let url = field(&candidate["url"])?;
    if url != purpur_url(&minecraft, id) {
        return Err("CoreCandidateUnverified".into());
    }
    let java = instance(root, name)?.core_profile.and_then(|profile| profile.java);
    let profile = |build: String, sha256: String| CoreProfile {
        family: "PURPUR".into(), minecraft: minecraft.clone(), build, sha256, unverified: false, java,
    };
    let before = profile(local_build, expected_current.to_owned());
    let staging = child_directory(root, "staging")?;
    let stage = staging.join(format!("core-download-{}-{}", std::process::id(), clock()));
    fs::create_dir(&stage).map_err(fail)?;
    let output = stage.join("candidate.jar");
    let outcome = (|| {
        let report = report_download(progress);
        // The API states no size, so the bar shows the downloaded amount only.
        download_verified_with(&url, agent, &output, &md5.to_ascii_lowercase(), None, Some(&report))?;
        let after = profile(build.clone(), content_revision::file_revision(&output)?);
        let _writer = instance_writer_lock(root, name)?;
        if registration_revision(root, name)? != registration {
            return Err("RegistrationChanged".into());
        }
        core_maintenance::install_identified(
            root, name, request, expected_current, &output, &before, &after, progress, begin_commit,
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

/// Velocity versions in the order the fill API lists them: version groups and
/// the versions in each group from newest to oldest (#41). `serde_json::Value`
/// would sort the group keys, so the reply is read in document order.
struct OrderedVersions(Vec<String>);

impl<'de> Deserialize<'de> for OrderedVersions {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct Groups;
        impl<'de> serde::de::Visitor<'de> for Groups {
            type Value = OrderedVersions;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("version groups")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> std::result::Result<Self::Value, A::Error> {
                let mut versions = Vec::new();
                while let Some((_, group)) = map.next_entry::<String, Vec<String>>()? {
                    versions.extend(group);
                }
                Ok(OrderedVersions(versions))
            }
        }
        deserializer.deserialize_map(Groups)
    }
}

#[derive(Deserialize)]
struct ProjectVersions {
    project: ProjectId,
    versions: OrderedVersions,
}

#[derive(Deserialize)]
struct ProjectId {
    id: String,
}

fn velocity_version_token(version: &str) -> bool {
    !version.is_empty() && version.len() <= 32
        && version.as_bytes()[0].is_ascii_digit()
        && version.bytes().all(|byte| byte.is_ascii_alphanumeric() || b".-".contains(&byte))
}

/// The Velocity versions newer than `current`, newest first. "Newer" follows
/// the order of the official list, which includes `-SNAPSHOT` versions.
fn newer_velocity_versions(body: &[u8], current: &str) -> Result<Vec<String>> {
    let reply: ProjectVersions = serde_json::from_slice(body).map_err(|_| "CoreCatalogInvalid")?;
    if reply.project.id != "velocity" || reply.versions.0.iter().any(|version| !velocity_version_token(version)) {
        return Err("CoreCatalogInvalid".into());
    }
    let position = reply.versions.0.iter().position(|version| version == current)
        .ok_or("CoreSourceVersionUnlisted")?;
    let mut seen = BTreeSet::new();
    Ok(reply.versions.0[..position].iter().filter(|version| seen.insert(version.as_str())).cloned().collect())
}

/// The verified Velocity profile of an instance's current core.
fn velocity_current(root: &Path, name: &str) -> Result<CoreProfile> {
    let config = instance(root, name)?;
    if !admitted(root, name)? || config.removed {
        return Err("CoreTargetUnavailable".into());
    }
    let jar = core_artifact::jar(&instance_path(root, name)?, &config);
    no_redirect(&jar)?;
    let profile = core_maintenance::trusted_profile(&config, content_revision::file_revision(&jar)?)
        .ok_or("CoreProfileUnverified")?;
    if profile.family != "VELOCITY" {
        return Err("CoreSourceUnsupported".into());
    }
    Ok(profile)
}

fn velocity_versions_with(current: &str, fetch: &impl Fn(&str, &str) -> Result<Vec<u8>>, agent: &str) -> Result<Vec<String>> {
    newer_velocity_versions(&fetch(&format!("{FILL_PROJECTS}/velocity"), agent)?, current)
}

fn fetch_bytes(url: &str, agent: &str) -> Result<Vec<u8>> {
    let _slot = SourceSlot::acquire()?;
    let (code, body) = fetch_status(url, agent, true)?;
    if code != 200 {
        return Err("CoreSourceHttpError".into());
    }
    Ok(body)
}

/// `core-velocity-versions NAME`: the Velocity versions newer than the current
/// one, for the first step of a cross-version change (#41).
pub(super) fn velocity_versions(root: &Path, name: &str) -> Result<Value> {
    let current = velocity_current(root, name)?;
    let versions = velocity_versions_with(&current.minecraft, &fetch_bytes, &configured_paper_user_agent()?)?;
    let rows: Vec<_> = versions.iter()
        .map(|version| json!({"version":version,"snapshot":version.ends_with("-SNAPSHOT")}))
        .collect();
    Ok(json!({"current":current,"versions":rows}))
}

/// The newest offered build of `version`, with its official SHA-256.
fn velocity_target_with(
    current: &CoreProfile,
    version: &str,
    agent: &str,
    fetch_body: &impl Fn(&str, &str) -> Result<Vec<u8>>,
    fetch: &impl Fn(&str, &str) -> Result<Value>,
) -> Result<(CoreProfile, String, Option<u64>, bool)> {
    if !velocity_versions_with(&current.minecraft, fetch_body, agent)?.iter().any(|newer| newer == version) {
        return Err("CoreTargetVersionNotNewer".into());
    }
    let key = SourceKey { family: "VELOCITY".into(), version: version.to_owned() };
    let Release::Paper(builds) = fetch_group(&key, Ok(agent), fetch).release? else {
        return Err("CoreSourceUnsupported".into());
    };
    let newest = builds.iter().filter(|build| update_eligible("VELOCITY", build)).max_by_key(|build| build.id)
        .ok_or("NoStableBuild")?;
    let (Some(hash), Some(url)) = (&newest.sha256, &newest.url) else {
        return Err("CoreSourceUnverifiedCandidate".into());
    };
    if builds.iter().filter(|build| build.sha256.as_ref() == Some(hash)).count() != 1 || hash == &current.sha256 {
        return Err("CoreSourceUnverifiedCandidate".into());
    }
    let profile = CoreProfile { unverified: false, java: None, family: "VELOCITY".into(),
        minecraft: version.to_owned(), build: newest.id.to_string(), sha256: hash.clone() };
    Ok((profile, url.clone(), newest.size, newest.stable))
}

/// `core-velocity-target NAME VERSION`: what a change to `VERSION` installs,
/// for the summary step of a cross-version change (#41).
pub(super) fn velocity_target(root: &Path, name: &str, version: &str) -> Result<Value> {
    let current = velocity_current(root, name)?;
    let (target, _, size, stable) = velocity_target_with(&current, version, &configured_paper_user_agent()?, &fetch_bytes, &fetch_json)?;
    Ok(json!({"current":current,"target":target,"size":size,"channel":if stable {"STABLE"} else {"BETA"}}))
}

/// `core-install-version NAME REQUEST CURRENT VERSION CANDIDATE`: moves a stopped
/// Velocity proxy to the newest build of another, newer Velocity version (#41).
pub(super) fn install_version(
    root: &Path,
    args: &[String],
    progress: &dyn Fn(CoreUpdatePhase),
) -> Result<Value> {
    let [name, request, expected_current, version, expected_candidate] = args else {
        return Err("InvalidCoreArguments".into());
    };
    if !instance_name(name) || !token(request) || !sha256(expected_current) || !sha256(expected_candidate) {
        return Err("InvalidCoreMaintenanceRequest".into());
    }
    let registration = registration_revision(root, name)?;
    progress(CoreUpdatePhase::Checking);
    let current = velocity_current(root, name)?;
    if &current.sha256 != expected_current {
        return Err("CoreArtifactChanged".into());
    }
    let agent = configured_paper_user_agent()?;
    let (target, url, size, _) = velocity_target_with(&current, version, &agent, &fetch_bytes, &fetch_json)?;
    if &target.sha256 != expected_candidate {
        return Err("CoreCandidateChanged".into());
    }
    let staging = child_directory(root, "staging")?;
    let stage = staging.join(format!("core-download-{}-{}", std::process::id(), clock()));
    fs::create_dir(&stage).map_err(fail)?;
    let output = stage.join("candidate.jar");
    let outcome = (|| {
        let report = report_download(progress);
        download_verified_with(&url, &agent, &output, &target.sha256, size, Some(&report))?;
        let _writer = instance_writer_lock(root, name)?;
        if registration_revision(root, name)? != registration {
            return Err("RegistrationChanged".into());
        }
        core_maintenance::install_cross_version(root, name, request, expected_current, &output, &target, progress)
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

/// Where a download reports its bytes: the official size, when known, and the
/// callback that receives `(received, total)`.
type DownloadProgress<'a> = Option<(Option<u64>, &'a dyn Fn(u64, Option<u64>))>;

/// Runs a download and, with `progress`, reports the bytes written so far.
/// `total` is the size the official source states. Without one, only the
/// received bytes are reported, never a guessed total.
fn complete_download(
    mut command: Command,
    output: &Path,
    expected: &str,
    progress: DownloadProgress,
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
    // `expected` is a SHA-256, or for Purpur the official MD5 (#41).
    let observed = if md5_text(expected) {
        content_revision::file_md5(output)?
    } else {
        content_revision::file_revision(output)?
    };
    if observed != expected {
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
            assert_eq!(result("target", &existing, &source, None)["status"], "Current");
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
        let reply = result("first", &existing, &source, None);
        assert_eq!(reply["status"], "UpdateAvailable");
        assert_eq!(reply["candidate"]["build"], "222");
        let newest = profile("PAPER", "26.3", "222", CANDIDATE);
        assert_eq!(result("second", &newest, &source, None)["status"], "Current");
        let wrong = profile("PAPER", "26.3", "205", &"1".repeat(64));
        assert_eq!(
            result("wrong", &wrong, &source, None)["reason"],
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
        let refused = result("first", &existing, &poisoned, None);
        assert_eq!(refused["reason"], "CoreSourceUnverifiedCandidate");
        assert!(refused.get("candidate").is_none());
    }

    /// `<Minecraft version>.json` as BuildTools reads it, trimmed to the fields used.
    fn spigot_version(name: &str, spigot: &str, craftbukkit: &str) -> Value {
        json!({"name":name,"description":format!("Jenkins build {name}"),
            "refs":{"BuildData":"39e62795554dcf6b62e922bc061705b3dc228f88","Bukkit":"29025a7ed5789807e11158a2263316b13ed50c3b",
                "CraftBukkit":craftbukkit,"Spigot":spigot},"toolsVersion":197,"javaVersions":[69,70]})
    }

    const SPIGOT_112: &str = "79a30d7aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const CRAFT_112: &str = "f4830a1bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const SPIGOT_263: &str = "bed90fca391bc35f0dbbcdc7ce5d7bb28e533681";
    const CRAFT_263: &str = "227878d5186a258bc57909ced9a19cd2db25acf1";

    /// Compare a JAR fixture with a version file through the same path a check takes.
    fn spigot_row(jar: &[u8], version: Result<Value>, latest: Result<Value>) -> Value {
        let path = spigot_revision::tests::temporary(jar);
        let profile = spigot_profile(&path, LOCAL.into()).expect("the fixture is a Spigot core");
        let revision = spigot_revision::read(&path);
        fs::remove_file(&path).unwrap();
        let key = source_key(&profile).unwrap();
        let expected = format!("https://hub.spigotmc.org/versions/{}.json", profile.minecraft);
        let metadata = fetch_group(&key, Err("CoreSourceInvalidContact"), &|url, agent| {
            assert_eq!((url, agent), (expected.as_str(), SPIGOT_AGENT));
            version.clone()
        });
        let latest = spigot_latest(&|url, _| {
            assert_eq!(url, "https://hub.spigotmc.org/versions/latest.json");
            latest.clone()
        });
        let row = result("spigot", &profile, &metadata, Some(&Local::Spigot(SpigotLocal { revision: &revision, latest: Some(&latest) })));
        assert!(row.get("candidate").is_none(), "Spigot is never an install target");
        row
    }

    #[test]
    fn legacy_spigot_jar_is_current_newer_or_not_comparable() {
        let jar = spigot_revision::tests::legacy_jar("git-Spigot-79a30d7-f4830a1");
        let current = spigot_row(&jar, Ok(spigot_version("1573k", SPIGOT_112, CRAFT_112)),
            Ok(spigot_version("4665", SPIGOT_263, CRAFT_263)));
        assert_eq!((current["status"].as_str(), current["minecraft"].as_str()), (Some("Current"), Some("1.12.2")));
        assert_eq!(current["remoteBuild"], "1573k");
        assert_eq!(current["localBuild"], Value::Null);
        assert_eq!(current["localRevision"], json!({"spigot":"79a30d7","craftBukkit":"f4830a1"}));
        assert_eq!(current["latestMinecraft"], json!({"status":"Newer","build":"4665"}));

        let newer = spigot_row(&jar, Ok(spigot_version("1574", SPIGOT_112, "0000000ccccccccccccccccccccccccccccccccc")),
            Ok(spigot_version("4665", SPIGOT_263, CRAFT_263)));
        assert_eq!(newer["status"], "BuildAvailable");
        assert_eq!(newer["remoteBuild"], "1574");

        let unreadable = spigot_row(&spigot_revision::tests::legacy_jar("unknown"),
            Ok(spigot_version("1573k", SPIGOT_112, CRAFT_112)), Ok(spigot_version("4665", SPIGOT_263, CRAFT_263)));
        assert_eq!((unreadable["status"].as_str(), unreadable["reason"].as_str()),
            (Some("Unknown"), Some("CoreSourceSpigotRevisionUnreadable")));
    }

    #[test]
    fn bundler_spigot_jar_is_current_newer_or_not_comparable() {
        let jar = spigot_revision::tests::bundler_jar("26.3", "4665-Spigot-bed90fc-227878d");
        let latest = || Ok(spigot_version("4665", SPIGOT_263, CRAFT_263));
        let current = spigot_row(&jar, latest(), latest());
        assert_eq!((current["status"].as_str(), current["minecraft"].as_str()), (Some("Current"), Some("26.3")));
        assert_eq!(current["localBuild"], "4665");
        assert_eq!(current["latestMinecraft"], json!({"status":"Same","build":"4665"}));

        let older = spigot_revision::tests::bundler_jar("26.3", "4663-Spigot-bed90fc-c4db4c3");
        let newer = spigot_row(&older, latest(), latest());
        assert_eq!(newer["status"], "BuildAvailable");
        assert_eq!((newer["localBuild"].as_str(), newer["remoteBuild"].as_str()), (Some("4663"), Some("4665")));

        // A version file that is missing or malformed is reported, not guessed.
        let missing = spigot_row(&jar, Err("CoreSourceSpigotVersionMissing".into()), latest());
        assert_eq!((missing["status"].as_str(), missing["reason"].as_str()),
            (Some("Unknown"), Some("CoreSourceSpigotVersionMissing")));
        let malformed = spigot_row(&jar, Ok(json!({"name":"4665","refs":{"Spigot":"not-a-commit"}})), latest());
        assert_eq!(malformed["reason"], "CoreSourceInvalidSpigotRefs");
        let unreadable = spigot_row(&spigot_revision::tests::bundler_jar("26.3", "4665-Paper-bed90fc-227878d"),
            latest(), latest());
        assert_eq!(unreadable["reason"], "CoreSourceSpigotRevisionUnreadable");
    }

    #[test]
    fn spigot_source_failures_keep_their_reason() {
        let jar = spigot_revision::tests::bundler_jar("26.3", "4665-Spigot-bed90fc-227878d");
        for reason in ["CoreSourceRefused", "CoreSourceTimeout", "CoreSourceTransportError", "CoreSourceHttpError"] {
            let row = spigot_row(&jar, Err(reason.into()), Err(reason.into()));
            assert_eq!((row["status"].as_str(), row["reason"].as_str()), (Some("Unknown"), Some(reason)));
            assert_eq!(row["latestMinecraft"], Value::Null, "no hint without the version file");
        }
        // `latest.json` failing alone leaves the same-line result in place.
        let row = spigot_row(&jar, Ok(spigot_version("4665", SPIGOT_263, CRAFT_263)), Err("CoreSourceRefused".into()));
        assert_eq!(row["status"], "Current");
        assert_eq!(row["latestMinecraft"], json!({"status":"Unknown","reason":"CoreSourceRefused"}));

        assert_eq!(spigot_response(200, br#"{"name":"1"}"#).unwrap()["name"], "1");
        for (code, reason) in [(403, "CoreSourceRefused"), (429, "CoreSourceRefused"),
            (404, "CoreSourceSpigotVersionMissing"), (500, "CoreSourceHttpError")] {
            assert_eq!(spigot_response(code, b"<html>").unwrap_err(), reason, "{code}");
        }
        assert_eq!(spigot_response(200, b"<html>Just a moment...</html>").unwrap_err(), "CoreSourceInvalidJson");
    }

    #[test]
    fn a_pinned_spigot_core_is_compared_by_its_jar_too() {
        let pinned = pinned_core_profile("27a348344d42ed2ba76aad6fceebb7a26365f09c9ac6eea4c0918ba83ba7e3dd".into()).unwrap();
        let key = source_key(&pinned).unwrap();
        assert_eq!(group_url(&key), "https://hub.spigotmc.org/versions/26.2.json");
        let metadata = fetch_group(&key, Err("CoreSourceInvalidContact"), &|_, _| {
            Ok(spigot_version("4648", "8db49a2b72ef138a615856934957da2978f72f08", "96ae8cd2f4f8bfcee57193455d0714f2bd4fcd1f"))
        });
        let revision = spigot_revision::parse_version("4648-Spigot-8db49a2-96ae8cd").ok_or_else(|| "unparsed".to_owned());
        let row = result("pinned", &pinned, &metadata, Some(&Local::Spigot(SpigotLocal { revision: &revision, latest: None })));
        assert_eq!(row["status"], "Current");
        assert!(row.get("latestMinecraft").is_none());
        assert!(row.get("candidate").is_none());
        // A plain JAR that is not Spigot is not compared as one.
        let path = spigot_revision::tests::temporary(&core_identity::zip(&["plugin.yml"]));
        assert!(spigot_profile(&path, LOCAL.into()).is_none());
        fs::remove_file(&path).unwrap();
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
                result("target", profile, &source[&source_key(profile).unwrap()], None)["status"],
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
            assert_eq!(result("target", &existing, &refused, None)["reason"], "CoreSourceInvalidContact");
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
        let row = result("proxy", &existing, &source, None);
        assert_eq!((row["status"].as_str(), row["candidate"]["build"].as_str()), (Some("UpdateAvailable"), Some("43")));
        assert_eq!(result("proxy", &profile("VELOCITY", "4.2.1-SNAPSHOT", "43", CANDIDATE), &source, None)["status"], "Current");
        // The same channels for Paper offer nothing newer than STABLE.
        let paper = profile("PAPER", "26.3", "205", LOCAL);
        let paper_source = fetch_group(&source_key(&paper).unwrap(), Ok(TEST_AGENT), &|_, _| Ok(json!([
            build(222, "BETA", CANDIDATE, None), build(205, "STABLE", LOCAL, None),
        ])));
        assert_eq!(result("server", &paper, &paper_source, None)["status"], "Current");
        assert!(prefetch_family("VELOCITY") && prefetch_family("PAPER") && !prefetch_family("FOLIA"));
    }

    // #41: Purpur builds come from the official v2 API. Only SUCCESS builds
    // count, the local build is found by MD5, and the candidate carries its MD5.
    #[test]
    fn purpur_matches_the_local_build_by_md5_and_offers_the_newest_success() {
        let (old, new, failed) = ("a".repeat(32), "b".repeat(32), "c".repeat(32));
        let registered = CoreProfile { unverified: true, ..profile("PURPUR", "26.3", "", LOCAL) };
        let key = source_key(&registered).unwrap();
        assert_eq!(group_url(&key), "https://api.purpurmc.org/v2/purpur/26.3?detailed=true");
        let reply = json!({"project":"purpur","version":"26.3","builds":{"latest":"2661","all":[
            {"build":"2634","result":"SUCCESS","md5":old},
            {"build":"2660","result":"SUCCESS","md5":new.to_uppercase()},
            {"build":"2661","result":"FAILURE","md5":failed},
        ]}});
        let source = fetch_group(&key, Ok(TEST_AGENT), &|url, _| {
            assert_eq!(url, "https://api.purpurmc.org/v2/purpur/26.3?detailed=true");
            Ok(reply.clone())
        });
        let row = result("purpur", &registered, &source, Some(&Local::Md5(&old)));
        assert_eq!((row["status"].as_str(), row["build"].as_str(), row["verification"].as_str()),
            (Some("UpdateAvailable"), Some("2634"), Some("purpurMd5")));
        assert_eq!(row["candidate"], json!({"build":"2660","md5":new,
            "url":"https://api.purpurmc.org/v2/purpur/26.3/2660/download"}));
        assert!(row["candidate"].get("sha256").is_none(), "the SHA-256 is known only after download");
        assert_eq!(result("purpur", &registered, &source, Some(&Local::Md5(&new)))["status"], "Current");
        let unknown = result("purpur", &registered, &source, Some(&Local::Md5(&"d".repeat(32))));
        assert_eq!((unknown["status"].as_str(), unknown["reason"].as_str()), (Some("Unknown"), Some("CoreSourceUnverifiedLocalHash")));
        // A reply for another version or project is refused.
        assert!(purpur_builds(json!({"project":"purpur","version":"26.2","builds":{"all":[]}}), "26.3").is_err());
        assert!(purpur_builds(json!({"project":"purpur","version":"26.3","builds":{"all":[{"build":"1","result":"SUCCESS","md5":"x"}]}}), "26.3").is_err());
    }

    // #41: newer Velocity versions follow the order of the official list, with
    // groups newest first, and include -SNAPSHOT versions.
    #[test]
    fn velocity_versions_keep_the_official_order_and_offer_only_newer_ones() {
        let body = br#"{"project":{"id":"velocity","name":"Velocity"},"versions":{
            "4.0.0":["4.3.2-SNAPSHOT","4.3.1","4.3.0","4.2.1-SNAPSHOT","4.2.0"],
            "3.0.0":["3.5.1","3.4.0"]}}"#;
        assert_eq!(newer_velocity_versions(body, "4.2.1-SNAPSHOT").unwrap(), ["4.3.2-SNAPSHOT", "4.3.1", "4.3.0"]);
        assert_eq!(newer_velocity_versions(body, "3.5.1").unwrap().len(), 5, "groups are read in document order");
        assert!(newer_velocity_versions(body, "4.3.2-SNAPSHOT").unwrap().is_empty());
        assert_eq!(newer_velocity_versions(body, "9.9.9").unwrap_err(), "CoreSourceVersionUnlisted");
        let other = String::from_utf8_lossy(body).replace("\"id\":\"velocity\"", "\"id\":\"paper\"");
        assert_eq!(newer_velocity_versions(other.as_bytes(), "4.2.0").unwrap_err(), "CoreCatalogInvalid");

        let current = profile("VELOCITY", "4.2.1-SNAPSHOT", "43", LOCAL);
        let fetch_body = |_: &str, _: &str| Ok(body.to_vec());
        let builds = |url: &str, _: &str| {
            assert_eq!(url, "https://fill.papermc.io/v3/projects/velocity/versions/4.3.1/builds");
            Ok(json!([
                build(46, "ALPHA", &"f".repeat(64), Some(&format!("https://fill-data.papermc.io/v1/objects/{}/velocity-4.3.1-46.jar", "f".repeat(64)))),
                build(45, "STABLE", CANDIDATE, Some(&format!("https://fill-data.papermc.io/v1/objects/{CANDIDATE}/velocity-4.3.1-45.jar"))),
            ]))
        };
        let (target, url, _, stable) = velocity_target_with(&current, "4.3.1", TEST_AGENT, &fetch_body, &builds).unwrap();
        assert_eq!((target.minecraft.as_str(), target.build.as_str(), target.sha256.as_str(), stable), ("4.3.1", "45", CANDIDATE, true));
        assert!(url.ends_with("/velocity-4.3.1-45.jar"));
        for older in ["4.2.0", "4.2.1-SNAPSHOT", "3.5.1"] {
            assert_eq!(velocity_target_with(&current, older, TEST_AGENT, &fetch_body, &builds).err().as_deref(), Some("CoreTargetVersionNotNewer"));
        }
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
        let row = result("target", &existing, &ambiguous, None);
        assert_eq!(row["reason"], "CoreSourceUnverifiedCandidate");
        assert!(row.get("candidate").is_none());
    }
}
