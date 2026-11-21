use super::*;

pub(super) fn java_major(version: &str) -> Option<u16> {
    let version = version.split('"').nth(1)?;
    version
        .strip_prefix("1.")
        .unwrap_or(version)
        .split(|character: char| !character.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

fn definition(id: &str) -> Result<(u16, &'static str)> {
    Ok(match id {
        "paper-26.3" => (
            25,
            "a0a58f532f08054ea92eb916d3edb6ed3c86429d77175317304f6e995cce9968",
        ),
        "spigot-1.8.8" => (
            8,
            "6ed6046687ce9255228fe7bd31e097da49beff2a0c577e7be56d14f09753239c",
        ),
        _ => return Err("UnknownAgentProfile".into()),
    })
}

pub(super) fn validate(id: &str, core: &CoreProfile) -> Result<()> {
    if core.sha256 != definition(id)?.1 {
        return Err("AgentProfileCoreMismatch".into());
    }
    Ok(())
}

pub(super) fn resolve(root: &Path, name: &str, request: &mut prepare::Request) -> Result<()> {
    let id = request.profile.as_deref().ok_or("UnknownAgentProfile")?;
    let (major, hash) = definition(id)?;
    let assets = child_directory(root, "profile-assets")
        .and_then(|directory| child_directory(&directory, id))
        .map_err(|error| format!("AgentProfileAssetsUnavailable: {error}"))?;
    let artifact = |file: &str| -> Result<String> {
        prepare::local_jar(&path_format::display(&assets.join(file)))
            .map(|path| path_format::display(&path))
            .map_err(|error| format!("AgentProfileArtifactUnavailable: {file}: {error}"))
    };
    request.core_jar = artifact("server.jar")?;
    let core = prepare::local_jar(&request.core_jar)?;
    if content_revision::file_revision(&core)? != hash {
        return Err("AgentProfileCoreMismatch".into());
    }
    let core_profile = pinned_core_profile(hash.to_owned()).ok_or("AgentProfileCoreMismatch")?;
    request.mojang_jar = core_profile
        .mojang_sha256()
        .map(|_| artifact("mojang.jar"))
        .transpose()?;
    request.permission_jar = artifact("luckperms.jar")?;
    let found = presentation::discover_java(root)?;
    request.java = found
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|row| {
            let version = row["version"].as_str()?;
            (row["valid"] == true && row["stable"] == true && java_major(version) == Some(major))
                .then(|| row["path"].as_str())
                .flatten()
        })
        .ok_or_else(|| format!("AgentProfileJavaUnavailable: Java {major}"))?
        .to_owned();
    if request.port == 0 {
        for _ in 0..32 {
            let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
                .and_then(|listener| listener.local_addr())
                .map_err(fail)?
                .port();
            match port_unassigned(root, port, name) {
                Ok(()) => {
                    request.port = port;
                    break;
                }
                Err(error) if error == "PortAlreadyRegistered" => continue,
                Err(error) => return Err(error),
            }
        }
        if request.port == 0 {
            return Err("AgentProfilePortUnavailable".into());
        }
    }
    Ok(())
}

fn pending_path(root: &Path, name: &str) -> PathBuf {
    root.join("staging")
        .join(format!("agent-profile-{name}.json"))
}

pub(super) fn assert_idle(root: &Path, name: &str) -> Result<()> {
    match fs::symlink_metadata(pending_path(root, name)) {
        Ok(_) => Err("AgentProfilePending".into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(fail(error)),
    }
}

fn read_pending(root: &Path, name: &str) -> Result<Option<String>> {
    let marker = pending_path(root, name);
    match fs::symlink_metadata(&marker) {
        Ok(_) => {
            no_redirect(&marker)?;
            Ok(Some(read(&marker)?))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(fail(error)),
    }
}

pub(super) fn publish(
    root: &Path,
    name: &str,
    id: &str,
    spec: &prepare::Request,
    work: &Path,
    stage: &Path,
    profile_id: &str,
    java_path: PathBuf,
    core: CoreProfile,
) -> Result<Value> {
    let config = Instance {
        name: name.to_owned(),
        java: java_path,
        port: spec.port,
        velocity_listen_address: None,
        removed: false,
        jvm: None,
        timing: None,
        auto_restart: None,
        core_local_create: true,
        management_discovered: false,
        core_profile: Some(core),
        directory: Some(path_format::display(&spec.target(root))),
    };
    let metadata = stage.join("prepare.json");
    let mut manifest: Value = read(&metadata)?;
    manifest["profile"] = json!(profile_id);
    manifest["requestId"] = json!(id);
    save(&metadata, &manifest)?;
    let mut receipt: Receipt = read(work)?;
    let properties = content_revision::file_revision(&stage.join("server.properties"))?;
    receipt.output =
        Some(json!({"registration":config,"manifest":manifest,"properties":properties}));
    save(work, &receipt)?;
    save(&pending_path(root, name), &id)?;
    finish(root, name, id, spec, work, receipt, false)
}

pub(super) fn replay(
    root: &Path,
    name: &str,
    id: &str,
    spec: &prepare::Request,
    work: &Path,
    receipt: Receipt,
) -> Result<Value> {
    if receipt.complete {
        if read_pending(root, name)?.as_deref() == Some(id) {
            fs::remove_file(pending_path(root, name)).map_err(fail)?;
        }
        let mut output = receipt.output.ok_or("ResultUnconfirmed")?;
        output["replayed"] = json!(true);
        return Ok(output);
    }
    finish(root, name, id, spec, work, receipt, true)
}

fn finish(
    root: &Path,
    name: &str,
    id: &str,
    spec: &prepare::Request,
    work: &Path,
    mut receipt: Receipt,
    replayed: bool,
) -> Result<Value> {
    let snapshot = receipt.output.as_ref().ok_or("ResultUnconfirmed")?;
    let config: Instance =
        serde_json::from_value(snapshot["registration"].clone()).map_err(fail)?;
    let manifest = &snapshot["manifest"];
    let target = spec.target(root);
    let core = config
        .core_profile
        .as_ref()
        .ok_or("AgentProfileRecoveryMismatch")?;
    if config.name != name
        || config.directory.as_deref() != Some(path_format::display(&target).as_str())
        || !spec.matches_agent(name, manifest)
        || manifest["requestId"] != id
        || manifest["profile"].as_str() != spec.profile.as_deref()
        || manifest["port"] != config.port
        || serde_json::to_value(core).map_err(fail)? != manifest["core"]
    {
        return Err("AgentProfileRecoveryMismatch".into());
    }
    validate(spec.profile.as_deref().ok_or("UnknownAgentProfile")?, core)?;
    let stage = root.join("staging").join(id);
    let record = record_path(root, name);
    let _parents = spec.hold_parent(root)?;
    let location = match (stage.exists(), target.exists()) {
        (true, false) => &stage,
        (false, true) => &target,
        _ => return Err("AgentProfileRecoveryMismatch".into()),
    };
    no_redirect(location)?;
    let metadata = location.join("prepare.json");
    no_redirect(&metadata)?;
    let jar = location.join("server.jar");
    no_redirect(&jar)?;
    if read::<Value>(&metadata)? != *manifest
        || content_revision::file_revision(&jar)? != core.sha256
    {
        return Err("AgentProfileRecoveryMismatch".into());
    }
    let properties = location.join("server.properties");
    no_redirect(&properties)?;
    if snapshot["properties"] != content_revision::file_revision(&properties)? {
        return Err("AgentProfileRecoveryMismatch".into());
    }
    if let Some(expected) = core.mojang_sha256() {
        let cache = child_directory(location, "cache")?;
        let mojang = cache.join(format!("mojang_{}.jar", core.minecraft));
        no_redirect(&mojang)?;
        if content_revision::file_revision(&mojang)? != expected {
            return Err("AgentProfileRecoveryMismatch".into());
        }
    }
    test_permissions::verify_staged(location, core, manifest)?;
    match read_pending(root, name)?.as_deref() {
        Some(owner) if owner == id => (),
        None if location == &stage && !record.exists() => save(&pending_path(root, name), &id)?,
        _ => return Err("AgentProfileRecoveryMismatch".into()),
    }
    port_unassigned(root, config.port, name)?;
    port_free(config.port)?;
    if !scan_policy::admitted_path(root, name, &target)? {
        return Err("InstanceExcludedOrInvalid".into());
    }
    registration_path_available(root, name, &target)?;
    if record.exists() {
        if location == &stage
            || serde_json::to_value(instance(root, name)?).map_err(fail)?
                != snapshot["registration"]
        {
            return Err("AgentProfileRecoveryMismatch".into());
        }
    } else {
        if location == &stage {
            core_maintenance::move_durable(&stage, &target, false)?;
        }
        save(&record, &config)?;
    }
    let result = json!({"workId": id, "replayed": replayed, "state":"Stopped",
        "port": config.port, "profile": manifest["profile"], "core": core,
        "java": path_format::display(&config.java)});
    receipt.complete = true;
    receipt.output = Some(result.clone());
    save(work, &receipt)?;
    fs::remove_file(pending_path(root, name)).map_err(fail)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_major_handles_legacy_and_modern_versions() {
        assert_eq!(java_major("openjdk version \"1.8.0_504\""), Some(8));
        assert_eq!(java_major("openjdk version \"25.0.5\""), Some(25));
        assert_eq!(java_major("unknown"), None);
    }
}
