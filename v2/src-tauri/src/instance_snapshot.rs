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
        let record = bytes
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|bytes| decode_instance(path, name.ok_or("InvalidInstanceRecord")?, bytes));
        let is_registration_error = record.is_err();
        let result = record.and_then(|config| {
            if config.removed {
                return Ok(());
            }
            let included = scan_policy::admits_record(root, policy, &config)?;
            if !included && !stopping_outside_scope(root, &config)? {
                return Ok(());
            }
            let mut row = status(root, &config)?;
            if !included {
                row["outsideManagement"] = json!(true);
            }
            if include_metadata {
                let directory = managed_directory(root, &config.name, config.directory.as_deref())?;
                instances.push(presentation::instance_metadata(&config, &directory));
            }
            rows.push(row);
            Ok(())
        });
        if let Err(error) = result {
            let detail = crate::diagnostics::safe_error(&error);
            let os_error = detail["osCode"].as_u64();
            let reason = match os_error {
                _ if !is_registration_error => "ObservationUnconfirmed",
                Some(32 | 33) => "FileInUse",
                Some(5) => "AccessDenied",
                _ if error.starts_with("InvalidInstanceRecord") => "InvalidRecord",
                _ => "Unconfirmed",
            };
            let file = path.file_name().unwrap_or_default().to_string_lossy();
            issues.push(json!({"file":file,"reason":reason,"osError":os_error}));
        }
    }
    let mut result = json!({"rows":rows,"issues":issues});
    if include_metadata {
        result["instances"] = json!(instances);
        result["capabilities"] = presentation::capabilities(root);
    }
    result
}
