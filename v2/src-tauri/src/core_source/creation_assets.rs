//! Materialize a confirmed official core without publishing a server or trusting renderer URLs.
use super::*;

const MOJANG_MANIFEST: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const MOJANG_PACKAGES: &str = "https://piston-meta.mojang.com/v1/packages/";

struct LauncherContext {
    sha256: String,
    object: String,
}

fn sha1(text: &str) -> bool {
    text.len() == 40 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn mojang_object(url: &str) -> Option<&str> {
    let path = url
        .strip_prefix("https://piston-data.mojang.com/v1/objects/")
        .or_else(|| url.strip_prefix("https://launcher.mojang.com/v1/objects/"))?;
    let hash = path.strip_suffix("/server.jar")?;
    sha1(hash).then_some(hash)
}

fn context(hash: &str, url: &str) -> Option<LauncherContext> {
    sha256(hash).then_some(())?;
    Some(LauncherContext {
        sha256: hash.to_ascii_lowercase(),
        object: mojang_object(url)?.to_ascii_lowercase(),
    })
}

fn modern_context(bytes: &[u8], version: &str) -> Option<LauncherContext> {
    let line = std::str::from_utf8(bytes).ok()?;
    let mut fields = line.trim_end_matches(['\r', '\n']).split('\t');
    let hash = fields.next()?;
    let url = fields.next()?;
    let file = fields.next()?;
    if file != format!("mojang_{version}.jar") || fields.next().is_some() {
        return None;
    }
    context(hash, url)
}

fn legacy_context(bytes: &[u8], version: &str) -> Option<LauncherContext> {
    let mut entries = BTreeMap::new();
    for line in std::str::from_utf8(bytes).ok()?.lines().map(str::trim) {
        if line.is_empty() || line.starts_with(['#', '!']) {
            continue;
        }
        let (key, value) = line.split_once('=')?;
        if entries.insert(key, value).is_some() {
            return None;
        }
    }
    if entries.get("version") != Some(&version) {
        return None;
    }
    let url = entries
        .get("sourceUrl")?
        .replacen("https\\://", "https://", 1);
    context(entries.get("originalHash")?, &url)
}

fn launcher_context(path: &Path, profile: &CoreProfile) -> Result<LauncherContext> {
    no_redirect(path)?;
    let mut file = File::open(path).map_err(fail)?;
    let metadata = plugin_descriptor::named_metadata(&mut file, b"META-INF/download-context")
        .ok_or("CoreDownloadUnsupportedLayout")?;
    let parsed = match metadata {
        Some(bytes) => modern_context(&bytes, &profile.minecraft),
        None if profile.family == "PAPER" => {
            let legacy = plugin_descriptor::named_metadata(&mut file, b"patch.properties")
                .ok_or("CoreDownloadUnsupportedLayout")?;
            legacy.and_then(|bytes| legacy_context(&bytes, &profile.minecraft))
        }
        None => None,
    };
    parsed.ok_or_else(|| "CoreDownloadUnsupportedLayout".into())
}

fn version_metadata_url(manifest: &Value, minecraft: &str) -> Result<String> {
    let versions = manifest["versions"]
        .as_array()
        .ok_or("MojangMetadataUnverified")?;
    let mut matches = versions
        .iter()
        .filter(|entry| entry["id"].as_str() == Some(minecraft));
    let entry = matches.next().ok_or("MojangMetadataUnverified")?;
    if matches.next().is_some() || entry["type"] != "release" {
        return Err("MojangMetadataUnverified".into());
    }
    let hash = entry["sha1"]
        .as_str()
        .filter(|hash| sha1(hash))
        .ok_or("MojangMetadataUnverified")?;
    let url = format!("{MOJANG_PACKAGES}{hash}/{minecraft}.json");
    if entry["url"].as_str() != Some(&url) {
        return Err("MojangMetadataUnverified".into());
    }
    Ok(url)
}

fn mojang_server_download<'a>(version: &'a Value, minecraft: &str, object: &str) -> Result<(&'a str, Option<u64>)> {
    let server = &version["downloads"]["server"];
    let url = server["url"].as_str().ok_or("MojangMetadataUnverified")?;
    if version["id"] != minecraft
        || version["type"] != "release"
        || server["sha1"].as_str() != Some(object)
        || mojang_object(url) != Some(object)
    {
        return Err("MojangMetadataUnverified".into());
    }
    Ok((url, artifact_size(server)))
}

fn verify_mojang_source(
    profile: &CoreProfile,
    context: &LauncherContext,
    agent: &str,
) -> Result<(String, Option<u64>)> {
    let _slot = SourceSlot::acquire()?;
    let manifest = fetch_json(MOJANG_MANIFEST, agent)?;
    let metadata = version_metadata_url(&manifest, &profile.minecraft)?;
    let version = fetch_json(&metadata, agent)?;
    let (url, size) = mojang_server_download(&version, &profile.minecraft, &context.object)?;
    Ok((url.to_owned(), size))
}

pub(super) fn prepare(
    root: &Path,
    profile: &CoreProfile,
    progress: &dyn Fn(CreationPhase),
) -> Result<(PathBuf, Option<(PathBuf, String)>)> {
    let agent = configured_paper_user_agent()?;
    let builds = creation_builds(&profile.family, &profile.minecraft, &agent)?;
    let (url, size) = exact_stable_download(&builds, profile)?;
    let staging = child_directory(root, "staging")?;
    let directory = download_directory(root, profile)?;
    let _lock = command_lock(root, &format!("download-{}.lock", profile.sha256), false)?;
    let server = directory.join("server.jar");
    let mojang = directory.join("mojang.jar");

    if directory.exists() {
        let _directory = batch_import::hold_directories(root, &directory)?;
        if !verify_cached(&server, &profile.sha256)? {
            return Err("CoreDownloadCacheInvalid".into());
        }
        let context = launcher_context(&server, profile)?;
        verify_mojang_source(profile, &context, &agent)?;
        if !verify_cached(&mojang, &context.sha256)? {
            return Err("CoreDownloadCacheInvalid".into());
        }
        return Ok((server, Some((mojang, context.sha256))));
    }

    let scratch = staging.join(format!(
        "pending-download-{}-{}",
        std::process::id(),
        clock()
    ));
    fs::create_dir(&scratch).map_err(fail)?;
    let scratch_server = scratch.join("server.jar");
    let scratch_mojang = scratch.join("mojang.jar");
    let outcome = (|| {
        let scratch_directory = batch_import::hold_directories(root, &scratch)?;
        progress(CreationPhase::DownloadingCore);
        let report_core = |received, total| progress(CreationPhase::CoreBytes { received, total });
        complete_download(
            download_command(url, &agent, &scratch_server)?,
            &scratch_server,
            &profile.sha256,
            size.map(|total| (total, &report_core as &dyn Fn(u64, u64))),
        )?;
        let context = launcher_context(&scratch_server, profile)?;
        progress(CreationPhase::DownloadingMojang);
        let (mojang_url, size) = verify_mojang_source(profile, &context, &agent)?;
        let report_mojang = |received, total| progress(CreationPhase::MojangBytes { received, total });
        complete_download(
            download_command(&mojang_url, &agent, &scratch_mojang)?,
            &scratch_mojang,
            &context.sha256,
            size.map(|total| (total, &report_mojang as &dyn Fn(u64, u64))),
        )?;
        // Windows cannot rename a directory while its no-delete guard is live.
        drop(scratch_directory);
        core_maintenance::move_durable(&scratch, &directory, false)?;
        Ok((server, Some((mojang, context.sha256))))
    })();
    if outcome.is_err() {
        for path in [&scratch_server, &scratch_mojang] {
            if let Err(error) = fs::remove_file(path) {
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

#[cfg(test)]
mod tests {
    use super::*;

    const SHA1: &str = "33680f5f2ac32864d6d7cf5e56a705fdb3e05f4c";
    const SHA256: &str = "d052f14d7a173734fba553711e5b570162e2f2a313267ee31a21b975a679be64";

    #[test]
    fn embedded_context_uses_only_official_mojang_objects_for_the_exact_version() {
        let object = format!("https://piston-data.mojang.com/v1/objects/{SHA1}/server.jar");
        let modern = format!("{SHA256}\t{object}\tmojang_26.3.jar\n");
        let metadata = modern_context(modern.as_bytes(), "26.3").unwrap();
        assert_eq!(metadata.object, SHA1);
        assert_eq!(metadata.sha256, SHA256);
        assert!(modern_context(modern.as_bytes(), "26.2").is_none());
        assert!(modern_context(
            modern
                .replace("piston-data.mojang.com", "example.com")
                .as_bytes(),
            "26.3"
        )
        .is_none());

        let legacy = format!("#Paperclip\nversion=1.16.5\noriginalHash={}\nsourceUrl=https\\://launcher.mojang.com/v1/objects/{SHA1}/server.jar\n", SHA256.to_uppercase());
        let metadata = legacy_context(legacy.as_bytes(), "1.16.5").unwrap();
        assert_eq!(metadata.object, SHA1);
        assert_eq!(metadata.sha256, SHA256);
        assert!(legacy_context(
            format!("{legacy}originalHash={SHA256}\n").as_bytes(),
            "1.16.5"
        )
        .is_none());
    }

    #[test]
    fn mojang_official_metadata_must_match_embedded_object_and_release() {
        let metadata_url = format!("{MOJANG_PACKAGES}{SHA1}/26.3.json");
        let manifest = json!({"versions": [{"id": "26.3", "type": "release", "sha1": SHA1, "url": metadata_url}]});
        assert_eq!(
            version_metadata_url(&manifest, "26.3").unwrap(),
            metadata_url
        );
        assert!(version_metadata_url(&manifest, "26.2").is_err());
        let url = format!("https://piston-data.mojang.com/v1/objects/{SHA1}/server.jar");
        let mut metadata = json!({"id": "26.3", "type": "release", "downloads": {"server": {"sha1": SHA1, "url": url}}});
        assert_eq!(mojang_server_download(&metadata, "26.3", SHA1).unwrap(), (url.as_str(), None));
        metadata["downloads"]["server"]["size"] = json!(32768);
        assert_eq!(mojang_server_download(&metadata, "26.3", SHA1).unwrap(), (url.as_str(), Some(32768)));
        metadata["type"] = json!("snapshot");
        assert!(mojang_server_download(&metadata, "26.3", SHA1).is_err());
    }
}
