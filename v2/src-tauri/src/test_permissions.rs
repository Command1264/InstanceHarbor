//! Pinned plugin artifacts for isolated instance preparation.
use super::*;
use std::os::windows::fs::MetadataExt;

const POLICY: &str = include_str!("../../../config/test-admin-permissions.json");
const PROFILES: &str = include_str!("../../../config/server-profiles.json");

struct Platform {
    file: String,
    digest: String,
    accepted: Vec<String>,
}

pub(super) struct PinnedArtifact {
    source: PathBuf,
    platform: Platform,
}

impl PinnedArtifact {
    pub(super) fn sha256(&self) -> &str {
        &self.platform.digest
    }
}

pub(super) fn diagnostic_plugin(profile: &CoreProfile) -> Option<(String, String)> {
    let platform = platform(profile).ok()?;
    Some((platform.file, platform.digest))
}

fn platform(profile: &CoreProfile) -> Result<Platform> {
    if !matches!(profile.family.as_str(), "SPIGOT" | "PAPER" | "FOLIA") {
        return Err("UnsupportedTestPermissionPlatform".into());
    }
    let policy: Value = serde_json::from_str(POLICY).map_err(fail)?;
    if policy["schemaVersion"] != 1 {
        return Err("InvalidTestPermissionPolicy".into());
    }
    let profiles: Value = serde_json::from_str(PROFILES).map_err(fail)?;
    let legacy = profiles["profiles"]
        .as_array()
        .ok_or("InvalidServerProfiles")?
        .iter()
        .any(|known| {
            known["implementation"].as_str() == Some(profile.family.as_str())
                && known["minecraftVersion"].as_str() == Some(profile.minecraft.as_str())
                && known["javaAlias"] == "local-8"
        });
    let pin = &policy["artifacts"][if legacy { "bukkit-java8" } else { "bukkit" }];
    let file = pin["fileName"]
        .as_str()
        .filter(|name| luckperms_name(name))
        .ok_or("InvalidTestPermissionPolicy")?
        .to_owned();
    let digest = policy_digest(&pin["sha256"])?;
    let mut platform = Platform {
        file,
        accepted: vec![digest.clone()],
        digest,
    };
    if !legacy {
        for prior in policy["preservedArtifacts"]["bukkit"]
            .as_array()
            .ok_or("InvalidTestPermissionPolicy")?
        {
            platform.accepted.push(policy_digest(&prior["sha256"])?);
        }
    }
    Ok(platform)
}

fn policy_digest(value: &Value) -> Result<String> {
    value
        .as_str()
        .filter(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| "InvalidTestPermissionPolicy".into())
}

fn luckperms_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower
        .strip_prefix("luckperms")
        .and_then(|rest| rest.strip_suffix(".jar"))
        .is_some_and(|suffix| suffix.is_empty() || suffix.starts_with(['-', '_', '.']))
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
}

pub(super) fn preflight(source: &Path, profile: &CoreProfile) -> Result<PinnedArtifact> {
    let platform = platform(profile)?;
    if source.to_str().is_none() {
        return Err("InvalidTestPermissionArtifact".into());
    }
    let source = prepare::local_jar(&path_format::display(source))?;
    if content_revision::file_revision(&source)? != platform.digest {
        return Err("TestPermissionArtifactMismatch".into());
    }
    Ok(PinnedArtifact { source, platform })
}

fn optional_entry(path: &Path, directory: bool) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(file_error("inspect-test-permissions", path, error)),
        Ok(metadata)
            if metadata.file_attributes() & 0x400 == 0
                && if directory {
                    metadata.is_dir()
                } else {
                    metadata.is_file()
                } =>
        {
            no_redirect(path)?;
            Ok(true)
        }
        Ok(_) => Err("UnsafeTestPermissionPath".into()),
    }
}

fn plugins(directory: &Path) -> Result<PathBuf> {
    let path = directory.join("plugins");
    if !optional_entry(&path, true)? {
        fs::create_dir(&path).map_err(fail)?;
    }
    child_directory(directory, "plugins")
}

fn installed(plugins: &Path, accepted: &[String]) -> Result<Option<(PathBuf, String)>> {
    let mut matched = None;
    for entry in fs::read_dir(plugins).map_err(fail)? {
        let entry = entry.map_err(fail)?;
        if !entry.file_name().to_str().is_some_and(luckperms_name) {
            continue;
        }
        if matched.is_some() || !optional_entry(&entry.path(), false)? {
            return Err("AmbiguousTestPermissionArtifact".into());
        }
        let hash = content_revision::file_revision(&entry.path())?;
        if !accepted.contains(&hash) {
            return Err("UnknownInstalledTestPermissionArtifact".into());
        }
        matched = Some((entry.path(), hash));
    }
    Ok(matched)
}

pub(super) fn stage(directory: &Path, artifact: &PinnedArtifact) -> Result<()> {
    let folder = plugins(directory)?;
    if installed(&folder, &artifact.platform.accepted)?.is_some() {
        return Err("DuplicateTestPermissionArtifact".into());
    }
    let path = folder.join(&artifact.platform.file);
    core_maintenance::copy_verified(&artifact.source, &path, artifact.sha256())
}

pub(super) fn verify_staged(directory: &Path, profile: &CoreProfile, manifest: &Value) -> Result<()> {
    let platform = platform(profile)?;
    let folder = child_directory(directory, "plugins")?;
    let pinned = folder.join(platform.file);
    no_redirect(&pinned)?;
    if manifest["permissionSha256"] != platform.digest
        || content_revision::file_revision(&pinned)? != platform.digest
    {
        return Err("AgentProfileRecoveryMismatch".into());
    }
    if let Some(expected) = manifest["pluginSha256"].as_str() {
        let jar = folder.join("project-plugin.jar");
        no_redirect(&jar)?;
        if content_revision::file_revision(&jar)? != expected {
            return Err("AgentProfileRecoveryMismatch".into());
        }
    }
    Ok(())
}


pub(super) fn dispatch(verb: &str) -> Option<Result<Value>> {
    match verb {
        "admin-install" | "admin-verify" => Some(Err("FixedTestAdministratorUnavailable".into())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_build_refuses_fixed_administrator_actions() {
        for verb in ["admin-install", "admin-verify"] {
            assert_eq!(
                dispatch(verb).unwrap().unwrap_err(),
                "FixedTestAdministratorUnavailable"
            );
        }
        assert!(dispatch("prepare").is_none());
    }
}
