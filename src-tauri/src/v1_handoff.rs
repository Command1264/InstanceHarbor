//! Hand a server folder that V1 managed over to this workspace (#14).
//!
//! The folder stays where it is. V1's markers move into a backup folder inside
//! it, and the folder is then registered through the ordinary import checks,
//! which record the core under its existing file name. A manifest records every
//! change so `v1-handoff-rollback` can give the folder back to V1.
use super::*;

pub(super) const BACKUP: &str = ".instanceharbor-v1-handoff";
const MANIFEST: &str = "manifest.json";
const MARKERS: [&str; 3] = ["workspace.json", ".testlab-manager.lock", "testlab-console"];

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema: u32,
    name: String,
    markers: Vec<String>,
    /// Only manifests written by early test builds rename a Velocity core.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    renamed_core: Option<String>,
}

struct Plan {
    directory: PathBuf,
    markers: Vec<String>,
    core: String,
    port: u16,
    listen: Option<Ipv4Addr>,
    revision: String,
}

fn entry(path: &Path) -> Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            no_redirect(path)?;
            Ok(Some(metadata))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(fail(error)),
    }
}

fn plan(root: &Path, name: &str, directory: &str) -> Result<Plan> {
    if !instance_name(name) {
        return Err("InvalidInstanceName".into());
    }
    if entry(&record_path(root, name))?.is_some() {
        return Err("AlreadyManaged".into());
    }
    let requested = scan_policy::rule_path(root, directory)?;
    if !scan_policy::manageable(root, &requested) || scan_policy::contains(&requested, &root.join("instances")) {
        return Err("ImportOutsideInstanceRoot".into());
    }
    let target = safe_directory(&requested)?;
    if target != requested {
        return Err("DirectoryRedirected".into());
    }
    // An excluded folder is refused before anything moves.
    if !scan_policy::admitted_path(root, name, &target)? {
        return Err("InstanceExcludedOrInvalid".into());
    }
    if entry(&target.join(BACKUP))?.is_some() {
        return Err("V1HandoffAlreadyStarted".into());
    }
    let mut markers = Vec::new();
    let mut evidence = Vec::new();
    for marker in MARKERS {
        if let Some(metadata) = entry(&target.join(marker))? {
            use std::os::windows::fs::MetadataExt;
            evidence.push(json!([marker, metadata.len(), metadata.last_write_time()]));
            markers.push(marker.to_owned());
        }
    }
    if markers.is_empty() {
        return Err("V1MarkersNotFound".into());
    }
    let velocity = entry(&target.join("velocity.toml"))?.is_some();
    let core_name = core_artifact::locate(&target, velocity)?.ok_or("MissingCoreArtifact")?;
    let core = target.join(&core_name);
    if entry(&core)?.is_none() {
        return Err("MissingCoreArtifact".into());
    }
    let (port, listen, config) = if velocity {
        let ((address, port), config) = velocity_binding(&target)?;
        (port, (address != Ipv4Addr::LOCALHOST).then_some(address), config)
    } else {
        let config = content_revision::file_revision(&target.join("server.properties"))?;
        (property_port(&target)?, None, config)
    };
    legacy_preview::v1_stopped(&target, port)?;
    core_artifact::assert_unused(&core)?;
    let revision = content_revision::revision(&serde_json::to_vec(&json!({
        "name": name, "directory": path_format::display(&target), "markers": evidence,
        "core": [&core_name, content_revision::file_revision(&core)?], "config": config, "port": port,
    })).map_err(fail)?)?;
    Ok(Plan { directory: target, markers, core: core_name, port, listen, revision })
}

fn view(name: &str, plan: &Plan) -> Value {
    json!({
        "name": name,
        "directory": path_format::display(&plan.directory),
        "markers": plan.markers,
        "backup": BACKUP,
        "core": plan.core,
        "port": plan.port,
        "listenAddress": plan.listen.map(|ip| ip.to_string()),
        "allInterfaces": plan.listen.is_some_and(|ip| ip.is_unspecified()),
        "revision": plan.revision,
    })
}

fn move_item(from: &Path, to: &Path) -> Result<()> {
    if entry(to)?.is_some() {
        return Err("V1HandoffTargetExists".into());
    }
    fs::rename(from, to).map_err(fail)
}

/// Put the folder back the way V1 left it. Every step is attempted; the first
/// failure is reported so the folder can be checked by hand.
fn restore(directory: &Path, manifest: &Manifest) -> Result<()> {
    let backup = directory.join(BACKUP);
    let mut first = None;
    if let Some(original) = &manifest.renamed_core {
        if let Err(error) = move_item(&directory.join(core_artifact::DEFAULT), &directory.join(original)) {
            first.get_or_insert(error);
        }
    }
    for marker in &manifest.markers {
        if let Err(error) = move_item(&backup.join(marker), &directory.join(marker)) {
            first.get_or_insert(error);
        }
    }
    if first.is_none() {
        if let Err(error) = fs::remove_file(backup.join(MANIFEST)).and_then(|()| fs::remove_dir(&backup)) {
            first.get_or_insert(fail(error));
        }
    }
    first.map_or(Ok(()), Err)
}

fn handoff(root: &Path, name: &str, request: &str, java: &str, directory: &str, revision: &str, listen: Option<&str>) -> Result<Value> {
    let plan = plan(root, name, directory)?;
    if plan.revision != revision {
        return Err("V1HandoffPreviewChanged".into());
    }
    let manifest = Manifest {
        schema: 1,
        name: name.to_owned(),
        markers: plan.markers.clone(),
        renamed_core: None,
    };
    let backup = plan.directory.join(BACKUP);
    fs::create_dir(&backup).map_err(fail)?;
    // The manifest is written before anything moves, so a rollback always knows
    // what to put back.
    if let Err(error) = save(&backup.join(MANIFEST), &manifest) {
        let _ = fs::remove_dir(&backup);
        return Err(error);
    }
    let moved = (|| -> Result<Value> {
        for marker in &plan.markers {
            move_item(&plan.directory.join(marker), &backup.join(marker))?;
        }
        register(root, name, request, java, &plan, listen)
    })();
    match moved {
        Ok(registered) => Ok(json!({"handedOff": name, "registration": registered, "manifest": manifest})),
        Err(error) => match restore(&plan.directory, &manifest) {
            Ok(()) => Err(error),
            Err(restore_error) => Err(format!("{error}; V1HandoffRestoreFailed {restore_error}")),
        },
    }
}

/// Register through the batch import checks: the same admission, port, core
/// and configuration checks as an ordinary import from a management folder.
fn register(root: &Path, name: &str, request: &str, java: &str, plan: &Plan, listen: Option<&str>) -> Result<Value> {
    let directory = path_format::display(&plan.directory);
    let call = |verb: &str, args: Vec<String>| {
        let mut invocation = vec![path_format::display(root), verb.to_owned()];
        invocation.extend(args);
        dispatch(&invocation)
    };
    let preview = call("import-batch-preview", vec![json!({
        "directory": directory, "java": java,
        "overrides": [{"directory": directory, "name": name, "listenAddress": listen}],
    }).to_string()])?;
    let row = preview["rows"].as_array()
        .and_then(|rows| rows.iter().find(|row| row["directory"] == directory.as_str()))
        .ok_or("V1HandoffImportUnconfirmed")?;
    if row["state"] != "Ready" {
        return Err(row["reason"].as_str().unwrap_or("V1HandoffImportUnconfirmed").to_owned());
    }
    let selection = json!({"directory": directory, "rows": [{
        "directory": directory, "name": name, "java": java,
        "listenAddress": row["listenAddress"], "revision": row["revision"],
    }]});
    let applied = call("import-batch-apply", vec![request.to_owned(), selection.to_string()])?;
    let imported = applied["rows"].as_array()
        .is_some_and(|rows| rows.iter().any(|row| row["name"] == name && row["state"] == "Imported"));
    if !imported {
        return Err(applied["rows"][0]["reason"].as_str().unwrap_or("V1HandoffImportUnconfirmed").to_owned());
    }
    Ok(applied)
}

fn rollback(root: &Path, name: &str) -> Result<Value> {
    if !instance_name(name) {
        return Err("InvalidInstanceName".into());
    }
    let directory = instance_path(root, name)?;
    let manifest: Manifest = read(&directory.join(BACKUP).join(MANIFEST))?;
    if manifest.schema != 1 || manifest.name != name {
        return Err("V1HandoffManifestMismatch".into());
    }
    for marker in &manifest.markers {
        if entry(&directory.join(marker))?.is_some() {
            return Err("V1HandoffTargetExists".into());
        }
    }
    let detached = dispatch(&[path_format::display(root), "detach".into(), name.into()])?;
    restore(&directory, &manifest)?;
    Ok(json!({"rolledBack": name, "detached": detached, "directory": path_format::display(&directory)}))
}

pub(super) fn dispatch_verb(root: &Path, verb: &str, args: &[String]) -> Option<Result<Value>> {
    Some(match (verb, args) {
        ("v1-handoff-preview", [name, directory]) => plan(root, name, directory).map(|plan| view(name, &plan)),
        ("v1-handoff", [name, request, java, directory, revision]) => handoff(root, name, request, java, directory, revision, None),
        ("v1-handoff", [name, request, java, directory, revision, listen]) => handoff(root, name, request, java, directory, revision, Some(listen)),
        ("v1-handoff-rollback", [name]) => rollback(root, name),
        ("v1-handoff-preview" | "v1-handoff" | "v1-handoff-rollback", _) => Err("InvalidV1HandoffArguments".into()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct V1 {
        parent: PathBuf,
        root: PathBuf,
        server: PathBuf,
        java: String,
    }

    impl V1 {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let parent = std::env::temp_dir().join(format!(
                "ih-v1-handoff-{}-{}-{}", std::process::id(), clock(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            fs::create_dir(&parent).unwrap();
            let workspace = parent.join("workspace");
            dispatch(&[path_format::display(&workspace), "init".into()]).unwrap();
            let root = safe_directory(&workspace).unwrap();
            let java = parent.join("java.exe");
            fs::write(&java, b"synthetic-java").unwrap();
            // A V1 layout outside the workspace: instances\Human\project\lobby.
            let server = parent.join("v1").join("instances").join("Human").join("project").join("lobby");
            fs::create_dir_all(server.join("world")).unwrap();
            fs::create_dir_all(server.join("testlab-console")).unwrap();
            fs::create_dir_all(server.join("plugins").join("Example")).unwrap();
            let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port();
            fs::write(server.join("server.jar"), b"fixture").unwrap();
            fs::write(server.join("server.properties"), format!("server-ip=127.0.0.1\nserver-port={port}\nonline-mode=false\n")).unwrap();
            fs::write(server.join("workspace.json"), br#"{"schemaVersion":2}"#).unwrap();
            fs::write(server.join(".testlab-manager.lock"), b"").unwrap();
            fs::write(server.join("world").join("level.dat"), b"unchanged world").unwrap();
            fs::write(server.join("plugins").join("Example").join("config.yml"), b"kept: true").unwrap();
            Self { parent, root, server: safe_directory(&server).unwrap(), java: path_format::display(&java) }
        }

        fn call(&self, verb: &str, args: &[&str]) -> Result<Value> {
            let mut invocation = vec![path_format::display(&self.root), verb.to_owned()];
            invocation.extend(args.iter().map(|arg| (*arg).to_owned()));
            dispatch(&invocation)
        }

        fn directory(&self) -> String {
            path_format::display(&self.server)
        }

        fn files(&self) -> Vec<(String, Vec<u8>)> {
            let mut found = Vec::new();
            let mut folders = vec![self.server.clone()];
            while let Some(folder) = folders.pop() {
                for item in fs::read_dir(&folder).unwrap() {
                    let path = item.unwrap().path();
                    if path.is_dir() {
                        folders.push(path);
                    } else {
                        let relative = path.strip_prefix(&self.server).unwrap().display().to_string();
                        found.push((relative, fs::read(&path).unwrap()));
                    }
                }
            }
            found.sort();
            found
        }
    }

    impl Drop for V1 {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.parent);
        }
    }

    #[test]
    fn a_stopped_v1_server_is_handed_over_and_can_be_given_back() {
        let v1 = V1::new();
        let before = v1.files();
        let preview = v1.call("v1-handoff-preview", &["lobby", &v1.directory()]).unwrap();
        assert_eq!(preview["markers"], json!(["workspace.json", ".testlab-manager.lock", "testlab-console"]));
        assert_eq!(preview["core"], "server.jar");
        assert_eq!(v1.files(), before, "preview must not change the folder");

        let revision = preview["revision"].as_str().unwrap();
        v1.call("v1-handoff", &["lobby", "handoff-1", &v1.java, &v1.directory(), revision]).unwrap();
        assert_eq!(instance_path(&v1.root, "lobby").unwrap(), v1.server);
        for marker in MARKERS {
            assert!(!v1.server.join(marker).exists(), "{marker} still in place");
            assert!(v1.server.join(BACKUP).join(marker).exists(), "{marker} not backed up");
        }
        assert_eq!(fs::read(v1.server.join("world").join("level.dat")).unwrap(), b"unchanged world");
        assert_eq!(fs::read(v1.server.join("plugins").join("Example").join("config.yml")).unwrap(), b"kept: true");

        v1.call("v1-handoff-rollback", &["lobby"]).unwrap();
        assert!(!record_path(&v1.root, "lobby").exists());
        assert_eq!(v1.files(), before, "rollback must restore the V1 folder exactly");
    }

    fn ticket(v1: &V1, id: &str, body: Value) {
        fs::write(v1.server.join("testlab-console").join(format!("launch-{id}.json")), body.to_string()).unwrap();
    }

    #[test]
    fn stale_v1_launch_records_do_not_block_a_stopped_server() {
        let v1 = V1::new();
        // V1 leaves one record per launch. Interrupted launches can lack a Host
        // identity, and old ones name Hosts that exited long ago.
        ticket(&v1, "82dd701979b54828bc4a95ecbe983550", json!({"state":"Accepted"}));
        ticket(&v1, "1162baba305d4a518ff73e8af7b101fc",
            json!({"state":"Accepted","hostPid":4_000_000_u32,"hostStartUtc":"2026-10-05T08:46:25.9807852Z"}));
        ticket(&v1, "2b7ba8e5f34e4dd59fd7b4601d82cf7c", json!({"state":"Preparing"}));
        let preview = v1.call("v1-handoff-preview", &["lobby", &v1.directory()]).unwrap();
        assert_eq!(preview["core"], "server.jar");
    }

    #[test]
    fn a_live_v1_host_or_a_listener_still_blocks_the_handoff() {
        let v1 = V1::new();
        let mut child = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", "Start-Sleep 30"])
            .spawn()
            .unwrap();
        let started = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command",
                &format!("(Get-Process -Id {}).StartTime.ToUniversalTime().ToString('o')", child.id())])
            .output()
            .unwrap();
        let started = String::from_utf8(started.stdout).unwrap().trim().to_owned();
        ticket(&v1, "9c224e07d6374deb98f9a7e81013a7d0",
            json!({"state":"Accepted","hostPid":child.id(),"hostStartUtc":started}));
        let running = v1.call("v1-handoff-preview", &["lobby", &v1.directory()]);
        child.kill().unwrap();
        child.wait().unwrap();
        assert_eq!(running.unwrap_err(), "V1HostRunning");

        fs::remove_file(v1.server.join("testlab-console").join("launch-9c224e07d6374deb98f9a7e81013a7d0.json")).unwrap();
        let port = property_port(&v1.server).unwrap();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).unwrap();
        assert_eq!(v1.call("v1-handoff-preview", &["lobby", &v1.directory()]).unwrap_err(), "V1ServerListening");
        drop(listener);
        assert!(v1.call("v1-handoff-preview", &["lobby", &v1.directory()]).is_ok());
    }

    #[test]
    fn a_running_v1_manager_blocks_the_handoff_and_leaves_the_folder_alone() {
        use std::os::windows::fs::OpenOptionsExt;
        let v1 = V1::new();
        let before = v1.files();
        let held = fs::OpenOptions::new().read(true).write(true).share_mode(0)
            .open(v1.server.join(".testlab-manager.lock")).unwrap();
        assert_eq!(v1.call("v1-handoff-preview", &["lobby", &v1.directory()]).unwrap_err(), "V1HostRunning");
        drop(held);
        assert_eq!(v1.files(), before);
        assert!(!record_path(&v1.root, "lobby").exists());
    }

    #[test]
    fn a_failed_registration_puts_the_v1_markers_back() {
        let v1 = V1::new();
        let before = v1.files();
        let preview = v1.call("v1-handoff-preview", &["lobby", &v1.directory()]).unwrap();
        // Another registration that takes the same port makes the import checks refuse it.
        save(&record_path(&v1.root, "other"), &json!({"name":"other","port":preview["port"],"java":""})).unwrap();
        let error = v1.call("v1-handoff", &["lobby", "handoff-2", &v1.java, &v1.directory(), preview["revision"].as_str().unwrap()]).unwrap_err();
        assert!(!error.contains("RestoreFailed"), "{error}");
        assert!(error.contains("PortAlreadyRegistered"), "{error}");
        assert!(!record_path(&v1.root, "lobby").exists());
        assert!(!v1.server.join(BACKUP).exists());
        assert_eq!(v1.files(), before);
    }

    #[test]
    fn an_excluded_folder_is_refused_in_the_preview() {
        let v1 = V1::new();
        let policy = v1.call("scan-policy", &[]).unwrap();
        let only = json!({"revision":policy["revision"],"allowPaths":[path_format::display(v1.server.parent().unwrap())],"excludePaths":[v1.directory()]});
        v1.call("save-scan-policy", &[&only.to_string()]).unwrap();
        assert_eq!(v1.call("v1-handoff-preview", &["lobby", &v1.directory()]).unwrap_err(), "InstanceExcludedOrInvalid");
        assert!(v1.server.join("workspace.json").exists());
    }

    #[test]
    fn a_changed_folder_needs_a_new_preview() {
        let v1 = V1::new();
        let preview = v1.call("v1-handoff-preview", &["lobby", &v1.directory()]).unwrap();
        fs::write(v1.server.join("server.properties"), "server-ip=127.0.0.1\nserver-port=1\n").unwrap();
        assert_eq!(
            v1.call("v1-handoff", &["lobby", "handoff-3", &v1.java, &v1.directory(), preview["revision"].as_str().unwrap()]).unwrap_err(),
            "V1HandoffPreviewChanged"
        );
        assert!(v1.server.join("workspace.json").exists());
    }
}
