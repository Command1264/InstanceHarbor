use super::*;

const MAX_RECORD_BYTES: u64 = 64 * 1024;

pub(super) fn query(root: &Path, include_metadata: bool) -> Result<Value> {
    match resident_manager::existing_snapshot(root, include_metadata) {
        Err(error) if matches!(
            error.as_str(),
            "ManagerUnavailable" | "ManagerPipeUnavailable" | "ManagerFrameTooLarge" | "InvalidManagerRequest"
        ) => read(root, include_metadata),
        result => result,
    }
}

// Compare committed records and the scope before publishing the projection.
// No writer lock or staged files are needed for an offline read.
pub(super) fn read(root: &Path, include_metadata: bool) -> Result<Value> {
    for attempt in 0..3 {
        let (policy, revision) = scan_policy::load(root)?;
        let records = capture(root)?;
        let snapshot = project(root, &policy, &records, include_metadata);
        if records == capture(root)? && revision == scan_policy::load(root)?.1 {
            return Ok(snapshot);
        }
        if attempt < 2 {
            thread::sleep(Duration::from_millis(25));
        }
    }
    Err("InstanceSnapshotChanged".into())
}

type Records = Vec<(PathBuf, Result<Vec<u8>>)>;

fn capture(root: &Path) -> Result<Records> {
    let directory = child_directory(root, "registry")?;
    let mut records = Vec::new();
    for entry in fs::read_dir(&directory).map_err(|error| file_error("read", &directory, error))? {
        let path = entry.map_err(|error| file_error("read", &directory, error))?.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            let bytes = (|| {
                let resolved = fs::canonicalize(&path)
                    .map_err(|error| file_error("resolve", &path, error))?;
                if resolved != path {
                    return Err("PathRedirected".into());
                }
                let mut bytes = Vec::new();
                File::open(&path)
                    .and_then(|file| file.take(MAX_RECORD_BYTES + 1).read_to_end(&mut bytes))
                    .map_err(|error| file_error("read", &path, error))?;
                if bytes.len() as u64 > MAX_RECORD_BYTES {
                    return Err("InvalidInstanceRecord".into());
                }
                Ok(bytes)
            })();
            records.push((path, bytes));
        }
    }
    records.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    Ok(records)
}

fn project(
    root: &Path,
    policy: &scan_policy::Policy,
    records: &Records,
    include_metadata: bool,
) -> Value {
    let mut rows = Vec::new();
    let mut instances = Vec::new();
    let mut issues = Vec::new();
    for (path, bytes) in records {
        let name = path
            .file_stem()
            .and_then(|entry| entry.to_str())
            .filter(|name| token(name));
        let record: Result<Instance> = bytes
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|bytes| decode_instance(path, name.ok_or("InvalidInstanceRecord")?, bytes));
        let config = match record {
            Ok(config) => config,
            Err(error) => {
                let detail = crate::diagnostics::safe_error(&error);
                let os_error = detail["osCode"].as_u64();
                let reason = match os_error {
                    Some(32 | 33) => "FileInUse",
                    Some(5) => "AccessDenied",
                    _ if error.starts_with("InvalidInstanceRecord") => "InvalidRecord",
                    _ => "Unconfirmed",
                };
                let file = path.file_name().unwrap_or_default().to_string_lossy();
                issues.push(json!({"file":file,"stage":"registration","reason":reason,"osError":os_error}));
                continue;
            }
        };
        if config.removed {
            continue;
        }
        let observed = (|| -> Result<Option<Value>> {
            let included = scan_policy::admits_record(root, policy, &config)?;
            if !included && !stopping_outside_scope(root, &config)? {
                return Ok(None);
            }
            let mut row = status(root, &config)?;
            if !included {
                row["outsideManagement"] = json!(true);
            }
            Ok(Some(row))
        })();
        let file = path.file_name().unwrap_or_default().to_string_lossy();
        let mut report = |error: &str| {
            let issue = observation_issue(error);
            let mut entry = issue.clone();
            entry["file"] = json!(file);
            entry["name"] = json!(config.name);
            entry["stage"] = json!("observation");
            issues.push(entry);
            issue
        };
        let row = match observed {
            Ok(None) => continue,
            Ok(Some(row)) => row,
            // Keep the instance listed, so it does not vanish while its state is unreadable.
            // Without a confirmed state it cannot be started, stopped or removed.
            Err(error) => json!({"name":config.name,"port":config.port,"java":config.java,
                "state":"StateUnconfirmed","ownerPid":null,"stopToken":null,"run":null,
                "stateIssue":report(&error)}),
        };
        if include_metadata {
            // The observed state stays usable when only the directory cannot be resolved.
            match managed_directory(root, &config.name, config.directory.as_deref()) {
                Ok(directory) => instances.push(presentation::instance_metadata(&config, &directory)),
                Err(error) => {
                    report(&error);
                }
            }
        }
        rows.push(row);
    }
    let mut result = json!({"rows":rows,"issues":issues});
    if include_metadata {
        result["instances"] = json!(instances);
        result["capabilities"] = presentation::capabilities(root);
    }
    result
}

/// Why an instance's state could not be read, as a stable reason and code. Raw
/// errors can hold user paths, so only the leading code, a trailing OS error and
/// the typed file-access stage are kept for "Copy diagnostics".
fn observation_issue(error: &str) -> Value {
    let detail = crate::diagnostics::safe_error(error);
    let os_error = error
        .strip_suffix(')')
        .and_then(|rest| rest.rsplit_once("(os error "))
        .and_then(|(_, code)| code.parse::<u32>().ok());
    let lead = error.split([':', '[', '(', ' ']).next().unwrap_or("");
    let rest = &error[lead.len()..];
    let identifier = !lead.is_empty()
        && lead.len() <= 64
        && lead.starts_with(|c: char| c.is_ascii_uppercase())
        && lead.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !rest.starts_with(' ');
    // serde_json reports malformed state files as plain sentences.
    let malformed = !identifier && os_error.is_none() && [
        "expected ", "EOF while parsing", "invalid ", "missing field", "unknown field", "duplicate field",
        "trailing ", "key must be", "control character", "lone leading surrogate",
    ].iter().any(|prefix| error.starts_with(prefix));
    let code = if identifier {
        lead
    } else if malformed {
        "InvalidStateFile"
    } else if os_error.is_some() {
        "OsError"
    } else {
        "UnclassifiedError"
    };
    let reason = match (os_error, code) {
        (Some(32 | 33), _) => "FileInUse",
        (Some(5), _) => "AccessDenied",
        (_, "PathRedirected") => "PathRedirected",
        (_, "InvalidStateFile") => "InvalidStateFile",
        _ => "ObservationUnconfirmed",
    };
    json!({"reason":reason,"code":code,"osError":os_error,"access":detail["detail"]})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(label: &str) -> (PathBuf, PathBuf) {
        let parent = std::env::temp_dir().join(format!("ih-snapshot-{label}-{}-{}", std::process::id(), clock()));
        fs::create_dir(&parent).unwrap();
        let workspace = parent.join("workspace");
        dispatch(&[workspace.to_string_lossy().into_owned(), "init".into()]).unwrap();
        let root = workspace.canonicalize().unwrap();
        let directory = root.join("instances").join("agent");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("server.jar"), b"not a real jar").unwrap();
        fs::write(directory.join("server.properties"), "server-ip=127.0.0.1\nserver-port=1\n").unwrap();
        let config = Instance { core_file: None,
            name: "agent".into(),
            java: parent.join("jdk").join("bin").join("java.exe"),
            port: 1,
            velocity_listen_address: None,
            removed: false,
            directory: None,
            jvm: None,
            timing: None,
            auto_restart: None,
            core_profile: None,
            core_local_create: false,
            management_discovered: false,
        };
        save(&record_path(&root, "agent"), &config).unwrap();
        (parent, root)
    }

    #[test]
    fn an_instance_whose_state_cannot_be_read_keeps_its_row_and_reports_why() {
        let (parent, root) = workspace("unreadable");
        let run = root.join("runs").join("agent");
        fs::create_dir_all(&run).unwrap();
        fs::write(run.join("identity.json"), b"{ not json").unwrap();

        let snapshot = read(&root, true).unwrap();
        let rows = snapshot["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{snapshot}");
        assert_eq!(rows[0]["name"], "agent");
        assert_eq!(rows[0]["state"], "StateUnconfirmed");
        assert_eq!(rows[0]["port"], 1);
        assert!(rows[0]["run"].is_null());
        assert_eq!(rows[0]["stateIssue"]["reason"], "InvalidStateFile");
        let issue = &snapshot["issues"][0];
        assert_eq!(issue["file"], "agent.json");
        assert_eq!(issue["name"], "agent");
        assert_eq!(issue["stage"], "observation");
        assert_eq!(issue["reason"], "InvalidStateFile");
        assert_eq!(snapshot["instances"].as_array().unwrap().len(), 1);
        assert!(!snapshot["issues"].to_string().contains("ih-snapshot"), "{snapshot}");
        fs::remove_dir_all(&parent).unwrap();
    }

    #[test]
    fn observation_failures_are_classified_without_paths() {
        let cases = [
            ("Access is denied. (os error 5)", "AccessDenied", "OsError", Some(5u64)),
            (r"FileAccess[read] path=C:\Users\someone\ws\runs\a\identity.json: The process cannot access the file because it is being used by another process. (os error 32)", "FileInUse", "FileAccess", Some(32)),
            ("PathRedirected", "PathRedirected", "PathRedirected", None),
            ("expected value at line 1 column 3", "InvalidStateFile", "InvalidStateFile", None),
            ("ScanDirectoryUnavailable", "ObservationUnconfirmed", "ScanDirectoryUnavailable", None),
            (r"in C:\secret\folder (os error 5) more", "ObservationUnconfirmed", "UnclassifiedError", None),
        ];
        for (error, reason, code, os) in cases {
            let issue = observation_issue(error);
            assert_eq!(issue["reason"], reason, "{error}");
            assert_eq!(issue["code"], code, "{error}");
            assert_eq!(issue["osError"].as_u64(), os, "{error}");
            assert!(!issue.to_string().contains("someone") && !issue.to_string().contains("secret"), "{issue}");
        }
        let access = observation_issue(cases[1].0);
        assert_eq!(access["access"], json!({"stage":"read","file":"managed-file"}));
    }
}
