//! A rebuild never overwrites an existing directory. The receipt and archive
//! make each non-replacing rename recoverable after process interruption.
use super::*;

const STAGED_ID: &str = "agent-rebuild-request.json";

#[derive(Deserialize, Serialize)]
struct Intent {
    request: String,
    original_revision: String,
    original_metadata: String,
    new_metadata: String,
    next: Instance,
}

fn journal(root: &Path, name: &str) -> PathBuf {
    root.join("staging")
        .join(format!("agent-rebuild-{name}.json"))
}

fn present(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            no_redirect(path)?;
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(fail(error)),
    }
}

pub(super) fn pending(root: &Path, name: &str) -> Result<bool> {
    present(&journal(root, name))
}

pub(super) fn assert_idle(root: &Path, name: &str) -> Result<()> {
    if pending(root, name)? {
        return Err("AgentRebuildPending".into());
    }
    agent_profiles::assert_idle(root, name)
}

pub(super) fn preflight(root: &Path, name: &str, spec: &prepare::Request) -> Result<Instance> {
    if !present(&record_path(root, name))? {
        return Err("AgentRebuildRequiresPreparedInstance".into());
    }
    let previous = instance(root, name)?;
    let target = spec.target(root);
    if previous.removed
        || !previous.core_local_create
        || registered_velocity(&previous).is_some()
        || previous.port != spec.port
        || instance_path(root, name)? != target
    {
        return Err("AgentRebuildIdentityChanged".into());
    }
    let metadata = target.join("prepare.json");
    no_redirect(&metadata)?;
    if !spec.matches_agent(name, &read(&metadata)?) {
        return Err("AgentRebuildIdentityChanged".into());
    }
    if deferred_settings::has_pending(root, name)?
        || world_live::queued::has_unsettled_rebuild_work(root, name)?
    {
        return Err("AgentRebuildPendingSettings".into());
    }
    core_maintenance::require_stopped(root, name)?;
    scan_policy::verify_import_idle(&target)?;
    Ok(previous)
}

pub(super) fn stage(
    root: &Path,
    spec: &prepare::Request,
    request: &str,
    staged: &Path,
    next: Instance,
) -> Result<()> {
    core_maintenance::write_durable(&staged.join(STAGED_ID), &request, false)?;
    let intent = Intent {
        request: request.into(),
        original_revision: registration_revision(root, &next.name)?,
        original_metadata: content_revision::file_revision(
            &spec.target(root).join("prepare.json"),
        )?,
        new_metadata: content_revision::file_revision(&staged.join("prepare.json"))?,
        next,
    };
    core_maintenance::write_durable(&journal(root, &intent.next.name), &intent, false)
}

fn directory(root: &Path, name: &str) -> Result<PathBuf> {
    match fs::create_dir(root.join(name)) {
        Ok(()) => (),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(fail(error)),
    }
    child_directory(root, name)
}

fn verify_generation(path: &Path, request: &str, intent: &Intent) -> Result<()> {
    let marker = path.join(STAGED_ID);
    no_redirect(&marker)?;
    if read::<String>(&marker)? != request
        || content_revision::file_revision(&path.join("prepare.json"))? != intent.new_metadata
        || Some(content_revision::file_revision(&path.join("server.jar"))?.as_str())
            != intent
                .next
                .core_profile
                .as_ref()
                .map(|profile| profile.sha256.as_str())
        || property_port(path)? != intent.next.port
    {
        return Err("AgentRebuildGenerationUnconfirmed".into());
    }
    scan_policy::verify_import_idle(path)
}

pub(super) fn finish(
    root: &Path,
    name: &str,
    request: &str,
    spec: &prepare::Request,
    mut receipt: Receipt,
) -> Result<Value> {
    let record = journal(root, name);
    no_redirect(&record)?;
    let intent: Intent = read(&record)?;
    if intent.request != request {
        return Err("AgentRebuildPending".into());
    }
    let target = spec.target(root);
    if spec.human()
        || !spec.replace
        || intent.next.name != name
        || intent.next.port != spec.port
        || intent.next.directory.as_deref() != Some(path_format::display(&target).as_str())
        || !scan_policy::admitted_path(root, name, &target)?
    {
        return Err("AgentRebuildIdentityChanged".into());
    }
    let _parent = spec.hold_parent(root)?;
    let _lease = claim_runtime_directory(&target)?;
    let current = instance(root, name)?;
    if current.removed
        || current.port != spec.port
        || current.directory != intent.next.directory
        || status(root, &current)?["state"] != "Stopped"
    {
        return Err("AgentRebuildIdentityChanged".into());
    }
    port_unassigned(root, spec.port, name)?;
    port_free(spec.port)?;
    let revision = registration_revision(root, name)?;
    let registry_committed = json!(current) == json!(intent.next);
    if revision != intent.original_revision && !registry_committed {
        return Err("RegistrationChanged".into());
    }
    let archives = directory(root, "agent-rebuild-archives")?;
    let archive = directory(&archives, request)?;
    let original = archive.join("instance");
    let prior_registration = archive.join("registration.json");
    let staged = root.join("staging").join(request);
    let moved = present(&original)?;
    let target_exists = present(&target)?;
    if !moved {
        if !target_exists
            || revision != intent.original_revision
            || receipt.complete
            || content_revision::file_revision(&target.join("prepare.json"))?
                != intent.original_metadata
        {
            return Err("AgentRebuildOriginalUnconfirmed".into());
        }
        verify_generation(&staged, request, &intent)?;
        preflight(root, name, spec)?;
        if !present(&prior_registration)? {
            core_maintenance::write_durable(&prior_registration, &current, false)?;
        } else if json!(read::<Instance>(&prior_registration)?) != json!(current) {
            return Err("AgentRebuildOriginalUnconfirmed".into());
        }
        core_maintenance::move_durable(&target, &original, false)?;
    } else {
        if !present(&prior_registration)?
            || content_revision::file_revision(&original.join("prepare.json"))?
                != intent.original_metadata
        {
            return Err("AgentRebuildOriginalUnconfirmed".into());
        }
    }

    if moved && target_exists {
        verify_generation(&target, request, &intent)?;
        if present(&staged)? {
            return Err("AgentRebuildGenerationUnconfirmed".into());
        }
    } else {
        // If publishing fails, try to return the original tree to its original
        // path. The intent remains durable for a retry in either case.
        verify_generation(&staged, request, &intent)?;
        if let Err(error) = core_maintenance::move_durable(&staged, &target, false) {
            if !present(&target)? {
                core_maintenance::move_durable(&original, &target, false)
                    .map_err(|rollback| format!("AgentRebuildPending:{error}:{rollback}"))?;
            }
            return Err(format!("AgentRebuildPending:{error}"));
        }
    }
    // Do not carry a previous run or old world-rule journal into a new world.
    let run = root.join("runs").join(name);
    if present(&run)? {
        core_maintenance::move_durable(&run, &archive.join("run"), false)?;
    }
    let world_rules = root
        .join("history")
        .join(format!("queued-world-{name}.json"));
    if present(&world_rules)? {
        core_maintenance::move_durable(&world_rules, &archive.join("world-rules.json"), false)?;
    }
    // Even matching settings belong to a new generation. Bump the registry
    // revision so a previously confirmed lifecycle request cannot reuse it.
    if revision == intent.original_revision || !registry_committed {
        core_maintenance::write_durable(&record_path(root, name), &intent.next, true)?;
    }
    let replayed = receipt.complete;
    let mut output = json!({"workId":request,"replayed":false,"state":"Stopped","archive":path_format::display(&archive)});
    receipt.complete = true;
    receipt.output = Some(output.clone());
    core_maintenance::write_durable(
        &root.join("jobs").join(format!("{request}.json")),
        &receipt,
        true,
    )?;
    fs::remove_file(&record).map_err(fail)?;
    output["replayed"] = json!(replayed);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resumes_after_old_directory_was_archived_and_refuses_unexpected_target() {
        let socket = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = socket.local_addr().unwrap().port();
        drop(socket);
        let root =
            std::env::temp_dir().join(format!("ih-rebuild-{}-{}", std::process::id(), clock()));
        dispatch(&[root.to_string_lossy().into_owned(), "init".into()]).unwrap();
        struct Fixture(PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                fs::remove_dir_all(&self.0).unwrap();
            }
        }
        let _fixture = Fixture(root.clone());
        let root = safe_directory(&root).unwrap();
        let raw = json!({
            "audience":"Agent", "project":"Lobby", "instance":"rebuild",
            "java":"java.exe", "port":port, "coreJar":"paper.jar",
            "permissionJar":"luckperms.jar", "replace":true
        })
        .to_string();
        let spec = prepare::Request::parse(&raw).unwrap();
        let target = spec.target(&root);
        let _guard = spec.hold_parent(&root).unwrap();
        fs::create_dir(&target).unwrap();
        let name = "rebuild";
        let request = "replace-one";
        let mut profile = CoreProfile {
            family: "PAPER".into(),
            minecraft: "26.3".into(),
            build: "205".into(),
            sha256: content_revision::revision(b"original").unwrap(),
        };
        let previous = Instance {
            name: name.into(),
            java: PathBuf::from("java.exe"),
            port: spec.port,
            removed: false,
            directory: Some(path_format::display(&target)),
            jvm: None,
            timing: None,
            auto_restart: None,
            velocity_listen_address: None,
            core_profile: Some(profile.clone()),
            core_local_create: true,
            management_discovered: false,
        };
        let write_fixture =
            |folder: &Path, bytes: &[u8]| {
                fs::write(folder.join("server.jar"), bytes).unwrap();
                fs::write(
                    folder.join("server.properties"),
                    format!("server-port={port}\nserver-ip=127.0.0.1\n"),
                )
                .unwrap();
                save(&folder.join("prepare.json"), &json!({
                "schemaVersion":1, "audience":"Agent","project":"Lobby","instance":"rebuild",
                "managedId":name,"port":port
            })).unwrap();
            };
        write_fixture(&target, b"original");
        fs::write(target.join("world-marker"), b"world data").unwrap();
        save(&record_path(&root, name), &previous).unwrap();
        profile.build = "222".into();
        profile.sha256 = content_revision::revision(b"replacement").unwrap();
        let mut next = previous.clone();
        next.core_profile = Some(profile);
        let staged = root.join("staging").join(request);
        fs::create_dir(&staged).unwrap();
        write_fixture(&staged, b"replacement");
        stage(&root, &spec, request, &staged, next).unwrap();
        let work = root.join("jobs/replace-one.json");
        save(
            &work,
            &Receipt {
                arguments: vec!["prepare".into(), name.into(), request.into(), raw.clone()],
                complete: false,
                output: None,
                created_at: Some(1),
            },
        )
        .unwrap();
        let archives = directory(&root, "agent-rebuild-archives").unwrap();
        let archive = directory(&archives, request).unwrap();
        save(&archive.join("registration.json"), &previous).unwrap();
        let original = archive.join("instance");
        core_maintenance::move_durable(&target, &original, false).unwrap();
        assert_eq!(assert_idle(&root, name).unwrap_err(), "AgentRebuildPending");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("unexpected"), b"leave it alone").unwrap();
        assert!(finish(&root, name, request, &spec, read(&work).unwrap()).is_err());
        assert_eq!(
            fs::read(target.join("unexpected")).unwrap(),
            b"leave it alone"
        );
        core_maintenance::move_durable(&target, &root.join("unrelated-sample"), false).unwrap();
        let argv = [
            root.to_string_lossy().into_owned(),
            "prepare".into(),
            name.into(),
            request.into(),
            raw,
        ];
        let completed = dispatch(&argv).unwrap();
        assert_eq!(completed["state"], "Stopped");
        assert_eq!(dispatch(&argv).unwrap()["replayed"], true);
        assert_eq!(
            fs::read(original.join("world-marker")).unwrap(),
            b"world data"
        );
        assert_eq!(fs::read(target.join("server.jar")).unwrap(), b"replacement");
    }
}
