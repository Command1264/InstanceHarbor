//! Batch registration for existing stopped servers, with read-only discovery.
use super::*;
use std::{
    collections::BTreeSet,
    os::windows::fs::{MetadataExt, OpenOptionsExt},
};

const MAX_CANDIDATES: usize = 64;
const MAX_SELECTED: usize = 24;
const MAX_ENTRIES: usize = 1024;
const MAX_DEPTH: usize = 5;
const MAX_REQUEST: usize = 128 * 1024;
const MAX_JOURNAL: u64 = 512 * 1024;
const CONFIG_BYTES: u64 = 1024 * 1024;
const REPARSE_POINT: u32 = 0x400;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Preview {
    directory: String,
    #[serde(default)]
    java: String,
    #[serde(default)]
    overrides: Vec<Override>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Override {
    directory: String,
    name: Option<String>,
    java: Option<String>,
    listen_address: Option<String>,
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Selection {
    directory: String,
    rows: Vec<Selected>,
    #[serde(default, skip_serializing_if = "is_false")]
    automatic: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FolderPreview {
    directory: String,
    revision: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FolderSave {
    directory: String,
    revision: String,
    preview_revision: String,
    excluded_directories: Vec<String>,
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Selected {
    directory: String,
    name: String,
    java: String,
    listen_address: Option<String>,
    revision: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewRow {
    directory: String,
    name: String,
    java: String,
    kind: &'static str,
    core: Option<String>,
    port: Option<u16>,
    listen_address: Option<String>,
    state: &'static str,
    reason: Option<String>,
    revision: Option<String>,
    conflicts: Vec<String>,
}

#[derive(Deserialize, Serialize)]
struct Journal {
    version: u8,
    created_at: u64,
    selection: Selection,
    rows: Vec<Outcome>,
}

#[derive(Deserialize, Serialize)]
enum Outcome {
    Waiting,
    Publishing(Value),
    Imported,
    Rejected(String),
}

struct Checked {
    source: ImportSource,
    java: PathBuf,
    detected_core: Option<CoreProfile>,
    revision: String,
    _held_sources: Vec<File>,
}

fn parse<T: serde::de::DeserializeOwned>(raw: &str) -> Result<T> {
    if raw.len() > MAX_REQUEST {
        return Err("InvalidBatchImportArguments".into());
    }
    serde_json::from_str(raw).map_err(|_| "InvalidBatchImportArguments".into())
}

fn relative(root: &Path, path: &Path) -> Result<String> {
    let value = path
        .strip_prefix(root)
        .map_err(|_| "BatchImportDirectoryOutsideScope")?;
    value
        .to_str()
        .map(|value| value.replace('\\', "/"))
        .ok_or_else(|| "BatchImportEvidenceUnavailable".into())
}

fn scope(root: &Path, directory: &str) -> Result<PathBuf> {
    if directory.len() > 1024 {
        return Err("InvalidBatchImportArguments".into());
    }
    let base = root.join("instances");
    let path = if directory.eq_ignore_ascii_case("instances") {
        base.clone()
    } else {
        scan_policy::rule_path(root, directory)?
    };
    if !scan_policy::contains(&base, &path) {
        return Err("BatchImportDirectoryOutsideScope".into());
    }
    Ok(path)
}

fn raw_entry(path: &Path) -> Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(fail(error)),
    }
}

// Deny replacement of all checked directories until disk reads or publication
// finish. OPEN_REPARSE_POINT makes the final path component inspectable in place.
pub(super) fn hold_directories(root: &Path, path: &Path) -> Result<Vec<File>> {
    let suffix = path
        .strip_prefix(root)
        .map_err(|_| "BatchImportDirectoryOutsideScope")?;
    let mut current = root.to_path_buf();
    let mut held = Vec::new();
    for part in suffix.components() {
        current.push(part);
        let item = raw_entry(&current)?.ok_or("BatchImportEvidenceUnavailable")?;
        if !item.is_dir() || item.file_attributes() & REPARSE_POINT != 0 {
            return Err("DirectoryRedirected".into());
        }
        let folder = OpenOptions::new()
            .read(true)
            .share_mode(3)
            .custom_flags(0x02000000 | 0x00200000)
            .open(&current)
            .map_err(fail)?;
        let metadata = folder.metadata().map_err(fail)?;
        if !metadata.is_dir() || metadata.file_attributes() & REPARSE_POINT != 0 {
            return Err("DirectoryRedirected".into());
        }
        no_redirect(&current)?;
        held.push(folder);
    }
    Ok(held)
}

fn hold_file(path: &Path) -> Result<File> {
    let metadata = raw_entry(path)?.ok_or("BatchImportEvidenceUnavailable")?;
    if !metadata.is_file() || metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err("PathRedirected".into());
    }
    let file = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(0x00200000)
        .open(path)
        .map_err(fail)?;
    let metadata = file.metadata().map_err(fail)?;
    if !metadata.is_file() || metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err("PathRedirected".into());
    }
    no_redirect(path)?;
    Ok(file)
}

fn protected_path(path: &Path) -> bool {
    matches!(path.components().next(), Some(std::path::Component::Prefix(prefix))
        if matches!(prefix.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)))
        && path.is_absolute()
}

fn verified_java(requested: &str) -> Result<PathBuf> {
    if requested.is_empty() {
        return Err("BatchImportJavaRequired".into());
    }
    if requested.len() > 1024 || !protected_path(Path::new(requested)) {
        return Err("InvalidJavaExecutable".into());
    }
    let path = java_file(requested)?;
    if !protected_path(&path) {
        return Err("InvalidJavaExecutable".into());
    }
    Ok(path)
}

fn config_info(target: &Path) -> Result<(&'static str, PathBuf, u16, Option<Ipv4Addr>)> {
    let proxy = target.join("velocity.toml");
    let minecraft = target.join("server.properties");
    if raw_entry(&proxy)?.is_some() {
        if raw_entry(&minecraft)?.is_some() {
            return Err("BatchImportAmbiguousCore".into());
        }
        let raw = instance_diagnostics::bounded(&proxy, CONFIG_BYTES)?;
        let (ip, port) =
            velocity_endpoint(std::str::from_utf8(&raw).map_err(|_| "InvalidVelocityBind")?)?;
        Ok(("Velocity", proxy, port, Some(ip)))
    } else {
        let raw = instance_diagnostics::bounded(&minecraft, CONFIG_BYTES)
            .map_err(|_| "BatchImportConfigUnavailable")?;
        let endpoint = instance_diagnostics::backend_endpoint(&raw).ok_or("InvalidServerPort")?;
        if *endpoint.ip() != Ipv4Addr::LOCALHOST {
            return Err("ExpectedLoopbackServerAddress".into());
        }
        Ok(("Minecraft", minecraft, endpoint.port(), None))
    }
}

fn file_token(path: &Path, file: &File) -> Result<Value> {
    let metadata = file.metadata().map_err(fail)?;
    Ok(json!([
        content_revision::file_revision(path)?,
        metadata.creation_time(),
        metadata.last_write_time(),
        metadata.len()
    ]))
}

fn inspect_candidate(
    root: &Path,
    path: &Path,
    name: &str,
    java_path: &str,
    confirmed: Option<&str>,
    row: &mut PreviewRow,
    management: Option<&scan_policy::Policy>,
) -> Result<Checked> {
    if !token(name) {
        return Err("InvalidInstanceName".into());
    }
    if management.is_some() && raw_entry(&record_path(root, name))?.is_some() {
        return Err(if instance(root, name)?.removed {
            "ManagementRegistrationRemoved"
        } else {
            "AlreadyManaged"
        }
        .into());
    }
    let mut held = hold_directories(root, path)?;
    let policy_file = root.join("scan-policy.json");
    held.push(hold_file(&policy_file)?);
    let (_, policy_revision) = scan_policy::load(root)?;
    let admitted = match management {
        Some(policy) => policy.matches(root, name, path)?.1,
        None => scan_policy::admitted_path(root, name, path)?,
    };
    if !admitted {
        return Err("InstanceExcludedOrInvalid".into());
    }
    let (kind, config, port, ip) = config_info(path)?;
    row.kind = kind;
    row.port = Some(port);
    row.listen_address = ip.map(|ip| ip.to_string());
    let core = path.join("server.jar");
    if raw_entry(&core)?.is_none() {
        return Err("MissingCoreArtifact".into());
    }
    let core_file = hold_file(&core)?;
    let core_token = file_token(&core, &core_file)?;
    held.push(core_file);
    let detected_core = core_token[0]
        .as_str()
        .and_then(|hash| pinned_core_profile(hash.to_owned()));
    if let Some(profile) = &detected_core {
        row.core = Some(format!(
            "{} {} · {}",
            profile.family, profile.minecraft, profile.build
        ));
    }
    let config_file = hold_file(&config)?;
    let config_token = file_token(&config, &config_file)?;
    held.push(config_file);

    let (java, java_token) = if management.is_some() {
        if !detected_core
            .as_ref()
            .is_some_and(|profile| (profile.family == "VELOCITY") == (kind == "Velocity"))
        {
            return Err("UnverifiedManagementCore".into());
        }
        (PathBuf::new(), Value::Null)
    } else {
        let java = verified_java(java_path)?;
        let java_file = hold_file(&java)?;
        let java_token = file_token(&java, &java_file)?;
        held.push(java_file);
        (java, java_token)
    };
    let source = match management {
        Some(policy) => checked_import_with_policy(root, name, Some(&row.directory), confirmed, policy),
        None => checked_import(root, name, Some(&row.directory), confirmed),
    }?;
    if source.port != port || source.address != ip || source.target != path {
        return Err("BatchImportPreviewChanged".into());
    }
    let directory_id = held[path.strip_prefix(root).map_err(fail)?.components().count() - 1]
        .metadata()
        .map_err(fail)?
        .creation_time();
    let revision = content_revision::revision(&serde_json::to_vec(&json!({
        "name":name, "directory":row.directory, "directoryCreated":directory_id,
        "port":port, "address":ip, "policy":if management.is_some() { None } else { Some(policy_revision) },
        "core":core_token, "config":config_token,
        "java":java.to_str().ok_or("InvalidJavaExecutable")?, "javaToken":java_token,
        "retained":source.retained
    })).map_err(fail)?)?;
    Ok(Checked {
        source,
        java,
        detected_core,
        revision,
        _held_sources: held,
    })
}

fn bounded_reason(error: String) -> String {
    if error.starts_with("PortOccupied PID=") {
        "PortOccupiedOrUnavailable".to_owned()
    } else if error.len() <= 64
        && !error.is_empty()
        && error.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        error
    } else {
        "BatchImportEvidenceUnavailable".to_owned()
    }
}

fn new_row(root: &Path, path: &Path, name: Option<&str>, java: &str) -> Result<PreviewRow> {
    Ok(PreviewRow {
        directory: relative(root, path)?,
        name: name
            .unwrap_or_else(|| {
                path.file_name()
                    .and_then(|part| part.to_str())
                    .unwrap_or("")
            })
            .to_owned(),
        java: java.to_owned(),
        kind: "Unknown",
        core: None,
        port: None,
        listen_address: None,
        state: "Blocked",
        reason: None,
        revision: None,
        conflicts: Vec::new(),
    })
}

fn similar_path(a: &Path, b: &Path) -> bool {
    scan_policy::contains(a, b) || scan_policy::contains(b, a)
}

fn discovered_name(root: &Path, path: &Path, name: &str) -> Result<String> {
    if token(name) {
        return Ok(name.to_owned());
    }
    let slug = name
        .split(|part: char| !part.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
        .to_ascii_lowercase();
    let prefix = slug
        .get(..slug.len().min(48))
        .filter(|s| !s.is_empty())
        .unwrap_or("server");
    let id = content_revision::revision(relative(root, path)?.to_lowercase().as_bytes())?;
    Ok(format!("{}-{}", prefix.trim_end_matches('-'), &id[..12]))
}

fn preview(root: &Path, request: Preview) -> Result<Value> {
    discover(root, request, None)
}

fn discover(root: &Path, request: Preview, management: Option<&scan_policy::Policy>) -> Result<Value> {
    if request.overrides.len() > MAX_CANDIDATES || request.java.len() > 1024 {
        return Err("InvalidBatchImportArguments".into());
    }
    let start = scope(root, &request.directory)?;
    let instances = root.join("instances");
    let _bound = hold_directories(root, &start)?;
    let (current_policy, policy_revision) = scan_policy::load(root)?;
    let policy = management.unwrap_or(&current_policy);
    let mut parameters = std::collections::BTreeMap::new();
    for item in request.overrides {
        let dir = scope(root, &item.directory)?;
        let key = relative(root, &dir)?.to_ascii_lowercase();
        if !scan_policy::contains(&start, &dir)
            || item.name.as_ref().is_some_and(|s| s.len() > 64)
            || item.java.as_ref().is_some_and(|s| s.len() > 1024)
            || item.listen_address.as_ref().is_some_and(|s| s.len() > 64)
            || parameters.insert(key, item).is_some()
        {
            return Err("InvalidBatchImportArguments".into());
        }
    }
    let mut queue = vec![(start.clone(), 0)];
    let mut candidates: Vec<(PathBuf, PreviewRow)> = Vec::new();
    let mut entries = 0;
    let mut complete = true;
    while let Some((path, depth)) = queue.pop() {
        let hint = parameters.get(&relative(root, &path)?.to_ascii_lowercase());
        let java = hint
            .and_then(|v| v.java.as_deref())
            .unwrap_or(&request.java);
        let mut row = new_row(root, &path, hint.and_then(|v| v.name.as_deref()), java)?;
        if management.is_some() {
            row.name = discovered_name(root, &path, &row.name)?;
        }
        let precheck = (|| -> Result<_> {
            if policy.excludes_path(root, &path)? {
                return Err("InstanceExcludedOrInvalid".into());
            }
            let held = hold_directories(root, &path)?;
            Ok(held)
        })();
        match precheck {
            Err(code) => {
                if !matches!(
                    code.as_str(),
                    "DirectoryRedirected" | "InstanceExcludedOrInvalid"
                ) {
                    complete = false;
                }
                row.reason = Some(bounded_reason(code));
                candidates.push((path, row));
            }
            Ok(_held) => {
                let flags: Result<_> = ["server.jar", "server.properties", "velocity.toml"]
                    .into_iter()
                    .map(|name| raw_entry(&path.join(name)).map(|entry| entry.is_some()))
                    .collect::<Result<Vec<_>>>();
                match flags {
                    Ok(flags) if path != instances && flags.iter().any(|present| *present) => {
                        match inspect_candidate(
                            root,
                            &path,
                            &row.name.clone(),
                            java,
                            hint.and_then(|v| v.listen_address.as_deref()),
                            &mut row,
                            management,
                        ) {
                            Ok(checked) => {
                                row.java = path_format::display(&checked.java);
                                row.state = "Ready";
                                row.revision = Some(checked.revision);
                            }
                            Err(code) => row.reason = Some(bounded_reason(code)),
                        }
                        candidates.push((path, row));
                    }
                    Err(code) => {
                        row.reason = Some(bounded_reason(code));
                        candidates.push((path, row));
                        complete = false;
                    }
                    Ok(flags) => {
                        if flags.iter().any(|present| *present) {
                            row.reason = Some("ImportOutsideInstanceRoot".to_owned());
                            candidates.push((path.clone(), row));
                        }
                        if depth >= MAX_DEPTH {
                            complete = false;
                        } else if let Ok(children) = fs::read_dir(&path) {
                            for entry in children {
                                entries += 1;
                                if entries > MAX_ENTRIES {
                                    break;
                                }
                                match entry.and_then(|entry| Ok((entry.path(), entry.file_type()?)))
                                {
                                    Ok((path, kind)) if kind.is_dir() || kind.is_symlink() => {
                                        queue.push((path, depth + 1))
                                    }
                                    Ok(_) => {}
                                    Err(_) => complete = false,
                                }
                            }
                        } else {
                            complete = false;
                        }
                    }
                }
            }
        }
        if entries > MAX_ENTRIES || candidates.len() == MAX_CANDIDATES && !queue.is_empty() {
            complete = false;
            break;
        }
    }
    if scan_policy::load(root)?.1 != policy_revision {
        return Err("BatchImportPreviewChanged".into());
    }
    candidates.sort_by(|a, b| a.1.directory.cmp(&b.1.directory));
    for index in 0..candidates.len() {
        let (earlier, remaining) = candidates.split_at_mut(index);
        let (path, row) = &mut remaining[0];
        if row.state != "Ready" {
            continue;
        }
        for (other_path, other) in earlier.iter_mut().filter(|(_, row)| row.state == "Ready") {
            if row.name.eq_ignore_ascii_case(&other.name)
                || row.port == other.port
                || similar_path(path, other_path)
            {
                row.conflicts.push(other.directory.clone());
                other.conflicts.push(row.directory.clone());
            }
        }
    }
    if !complete {
        for (_, row) in &mut candidates {
            if row.state == "Ready" {
                row.state = "Blocked";
                row.reason = Some("BatchImportPreviewIncomplete".to_owned());
                row.revision = None;
            }
        }
    }
    if management.is_some() {
        for (_, row) in candidates.iter_mut().filter(|(_, row)| !row.conflicts.is_empty()) {
            row.state = "Blocked";
            row.reason = Some("BatchImportSelectionConflict".to_owned());
            row.revision = None;
        }
    }
    Ok(
        json!({"directory":relative(root, &start)?, "complete":complete,
        "rows":candidates.into_iter().map(|(_, row)| row).collect::<Vec<_>>() }),
    )
}

fn folder_preview(
    root: &Path,
    directory: &str,
    revision: &str,
) -> Result<(Value, PathBuf, scan_policy::Policy)> {
    let (current, current_revision) = scan_policy::load(root)?;
    if current_revision != revision {
        return Err("SettingsConflict".into());
    }
    let directory = scan_policy::managed_path(root, directory)?;
    let proposed = current.clone().with_folder(root, &directory, &[])?;
    let mut view = discover_folder(root, &directory, &proposed)?;
    view["policyRevision"] = json!(current_revision);
    view["previewRevision"] =
        json!(content_revision::revision(&serde_json::to_vec(&view).map_err(fail)?)?);
    Ok((view, directory, current))
}

pub(super) fn discover_folder(root: &Path, directory: &Path, policy: &scan_policy::Policy) -> Result<Value> {
    discover(
        root,
        Preview {
            directory: relative(root, directory)?,
            java: String::new(),
            overrides: Vec::new(),
        },
        Some(policy),
    )
}

fn selected_folders(view: &Value) -> Result<Vec<Selected>> {
    view["rows"]
        .as_array()
        .ok_or("BatchImportEvidenceUnavailable")?
        .iter()
        .filter(|row| row["state"] == "Ready")
        .map(|row| {
            let field = |key: &str| -> Result<String> {
                row[key]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or("BatchImportEvidenceUnavailable".into())
            };
            Ok(Selected {
                directory: field("directory")?,
                name: field("name")?,
                java: String::new(),
                listen_address: None,
                revision: field("revision")?,
            })
        })
        .collect()
}

fn register_folders(root: &Path, directory: &Path, rows: Vec<Selected>) -> Result<Value> {
    if rows.is_empty() {
        return Ok(Value::Null);
    }
    let selection = Selection {
        directory: relative(root, directory)?,
        rows,
        automatic: true,
    };
    let work_id = format!("folder-{}-{}", std::process::id(), clock());
    apply_locked(root, &work_id, selection)
}

fn folder_save(root: &Path, request: FolderSave) -> Result<Value> {
    let _registration = command_lock(root, "port-registration.lock", false)?;
    let (view, directory, current) = folder_preview(root, &request.directory, &request.revision)?;
    if view["previewRevision"] != request.preview_revision || view["complete"] != true {
        return Err("BatchImportPreviewChanged".into());
    }
    let selected = selected_folders(&view)?;
    let previewed: BTreeSet<_> = selected.iter().map(|row| row.directory.as_str()).collect();
    let excluded: BTreeSet<_> = request.excluded_directories.iter().map(String::as_str).collect();
    if excluded.len() != request.excluded_directories.len()
        || !excluded.is_subset(&previewed)
        || excluded.contains(relative(root, &directory)?.as_str())
    {
        return Err("InvalidManagementFolderSelection".into());
    }
    let policy = current.clone().with_folder(root, &directory, &request.excluded_directories)?;
    let impacts = scan_policy::preview(root, &current, &policy)?;
    let revision = scan_policy::persist(root, &policy, &request.revision)?;
    let registration = register_folders(
        root,
        &directory,
        selected
            .into_iter()
            .filter(|row| !excluded.contains(row.directory.as_str()))
            .collect(),
    );
    let (registration, registration_error) = match registration {
        Ok(registration) => (registration, None),
        Err(error) => (Value::Null, Some(bounded_reason(error))),
    };
    Ok(json!({"policy":policy,"revision":revision,"rows":impacts,
        "registration":registration,"registrationError":registration_error}))
}

pub(super) fn sync_management_folders(root: &Path, cursor: usize) -> Result<()> {
    let (policy, revision) = scan_policy::load(root)?;
    let roots = policy.management_roots();
    if roots.is_empty() {
        return Ok(());
    }
    let directory = scan_policy::managed_path(root, &roots[cursor % roots.len()])?;
    let view = discover_folder(root, &directory, &policy)?;
    if view["complete"] != true {
        return Ok(());
    }
    let selected = selected_folders(&view)?;
    if selected.is_empty() {
        return Ok(());
    }
    let _registration = command_lock(root, "port-registration.lock", false)?;
    // Reload after acquiring the registry lock. A policy edit may have removed
    // the selected root since the cursor was chosen.
    if scan_policy::load(root)?.1 != revision {
        return Ok(());
    }
    register_folders(root, &directory, selected)?;
    Ok(())
}

fn journal_folder(root: &Path, create: bool) -> Result<Option<PathBuf>> {
    let jobs = child_directory(root, "jobs")?;
    let folder = jobs.join("batch-import");
    if raw_entry(&folder)?.is_none() {
        if !create {
            return Ok(None);
        }
        fs::create_dir(&folder).map_err(fail)?;
    }
    child_directory(&jobs, "batch-import").map(Some)
}

fn read_journal(path: &Path) -> Result<Journal> {
    let journal: Journal =
        serde_json::from_slice(&instance_diagnostics::bounded(path, MAX_JOURNAL)?)
            .map_err(|_| "BatchImportResultUnconfirmed")?;
    if journal.version != 1
        || journal.rows.len() != journal.selection.rows.len()
        || journal.rows.is_empty()
        || journal.rows.len() > selection_limit(&journal.selection)
        || journal.selection.rows.iter().any(|row| !token(&row.name))
    {
        return Err("BatchImportResultUnconfirmed".into());
    }
    Ok(journal)
}

fn published(root: &Path, name: &str, record: &Value) -> bool {
    let path = record_path(root, name);
    instance_diagnostics::bounded(&path, 128 * 1024)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .is_some_and(|actual| actual == *record)
}

fn result(root: &Path, work_id: &str, journal: &Journal) -> Value {
    let mut interrupted = false;
    let rows = journal
        .selection
        .rows
        .iter()
        .zip(&journal.rows)
        .map(|(row, outcome)| {
            let (state, reason) = match outcome {
                Outcome::Imported => ("Imported", None),
                Outcome::Rejected(reason) => ("Rejected", Some(reason.as_str())),
                Outcome::Publishing(record) if published(root, &row.name, record) => {
                    ("Imported", None)
                }
                Outcome::Waiting | Outcome::Publishing(_) => {
                    interrupted = true;
                    ("NeedsConfirmation", Some("BatchImportResultUnconfirmed"))
                }
            };
            json!({"directory":row.directory,"name":row.name,"state":state,"reason":reason})
        })
        .collect::<Vec<_>>();
    json!({"workId":work_id,"state":if interrupted {"Interrupted"} else {"Complete"},"rows":rows})
}

fn imported_record(
    row: &Selected,
    checked: &Checked,
    work_id: &str,
    automatic: bool,
) -> Result<Value> {
    let retained = checked.source.retained.as_ref();
    let record = Instance {
        name: row.name.clone(),
        java: checked.java.clone(),
        port: checked.source.port,
        velocity_listen_address: checked
            .source
            .address
            .filter(|address| *address != Ipv4Addr::LOCALHOST),
        removed: false,
        directory: Some(path_format::display(&checked.source.target)),
        jvm: retained.and_then(|item| item.jvm.clone()),
        timing: retained.and_then(|item| item.timing.clone()),
        auto_restart: retained.and_then(|item| item.auto_restart.clone()),
        core_profile: if automatic {
            checked.detected_core.clone()
        } else {
            checked.source.velocity.clone()
        },
        core_local_create: false,
        management_discovered: automatic,
    };
    let mut record = serde_json::to_value(record).map_err(fail)?;
    // The durable marker ties a partially published record to this batch.
    // Older/single-instance readers tolerate this additional JSON field.
    record["batchImportWorkId"] = json!(work_id);
    Ok(record)
}

fn selection_limit(selection: &Selection) -> usize {
    if selection.automatic {
        MAX_CANDIDATES
    } else {
        MAX_SELECTED
    }
}

fn validate_selection(root: &Path, selection: &Selection) -> Result<Vec<PathBuf>> {
    if selection.rows.is_empty() || selection.rows.len() > selection_limit(selection) {
        return Err("InvalidBatchImportArguments".into());
    }
    let boundary = scope(root, &selection.directory)?;
    let _bound = hold_directories(root, &boundary)?;
    let mut names = BTreeSet::new();
    let mut paths: Vec<PathBuf> = Vec::new();
    for row in &selection.rows {
        if !token(&row.name)
            || row.java.len() > 1024
            || row.listen_address.as_ref().is_some_and(|ip| ip.len() > 64)
            || row.revision.len() != 64
            || !row.revision.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("InvalidBatchImportArguments".into());
        }
        let path = scope(root, &row.directory)?;
        if path == root.join("instances") || !scan_policy::contains(&boundary, &path) {
            return Err("BatchImportDirectoryOutsideScope".into());
        }
        if !names.insert(&row.name) || paths.iter().any(|other| similar_path(other, &path)) {
            return Err("BatchImportSelectionConflict".into());
        }
        paths.push(path);
    }
    Ok(paths)
}

fn apply(root: &Path, work_id: &str, selection: Selection) -> Result<Value> {
    if selection.automatic {
        return Err("InvalidBatchImportArguments".into());
    }
    let _registration = command_lock(root, "port-registration.lock", false)?;
    apply_locked(root, work_id, selection)
}

fn apply_locked(root: &Path, work_id: &str, selection: Selection) -> Result<Value> {
    if let Some(folder) = journal_folder(root, false)? {
        let path = folder.join(format!("{work_id}.json"));
        if raw_entry(&path)?.is_some() {
            let journal = read_journal(&path)?;
            if journal.selection != selection {
                return Err("RequestConflict".into());
            }
            return Ok(result(root, work_id, &journal));
        }
    }
    let paths = validate_selection(root, &selection)?;
    let management = selection
        .automatic
        .then(|| scan_policy::load(root).map(|(policy, _)| policy))
        .transpose()?;
    let mut ports = BTreeSet::new();
    let mut checked = Vec::new();
    for (row, path) in selection.rows.iter().zip(&paths) {
        let mut draft = new_row(root, path, Some(&row.name), &row.java)?;
        let observation = inspect_candidate(
            root,
            path,
            &row.name,
            &row.java,
            row.listen_address.as_deref(),
            &mut draft,
            management.as_ref(),
        );
        let observation = observation.and_then(|value| {
            if value.revision == row.revision {
                Ok(value)
            } else {
                Err("BatchImportPreviewChanged".into())
            }
        });
        if let Ok(value) = &observation {
            if !ports.insert(value.source.port) {
                return Err("BatchImportSelectionConflict".into());
            }
        }
        checked.push(observation);
    }
    let path = journal_folder(root, true)?
        .ok_or("BatchImportResultUnconfirmed")?
        .join(format!("{work_id}.json"));
    let mut journal = Journal {
        version: 1,
        created_at: (clock() / 1_000_000) as u64,
        rows: selection.rows.iter().map(|_| Outcome::Waiting).collect(),
        selection,
    };
    core_maintenance::write_durable(&path, &journal, false)?;
    for (index, checked) in checked.into_iter().enumerate() {
        let row = &journal.selection.rows[index];
        let checked = match checked {
            Ok(checked) => checked,
            Err(error) => {
                journal.rows[index] = Outcome::Rejected(bounded_reason(error));
                core_maintenance::write_durable(&path, &journal, true)?;
                continue;
            }
        };
        let _writer = match instance_writer_lock(root, &row.name) {
            Ok(lock) => lock,
            Err(error) if error == "ManagerBusy" => {
                journal.rows[index] = Outcome::Rejected(error);
                core_maintenance::write_durable(&path, &journal, true)?;
                continue;
            }
            Err(error) => return Err(error),
        };
        // The shared registration lock excludes another registration. The name
        // lock also excludes changes made through other per-instance entry points.
        let record = record_path(root, &row.name);
        let retained = match raw_entry(&record)? {
            Some(_) => Some(instance(root, &row.name)?),
            None => None,
        };
        if serde_json::to_value(&retained).map_err(fail)?
            != serde_json::to_value(&checked.source.retained).map_err(fail)?
        {
            journal.rows[index] = Outcome::Rejected("BatchImportPreviewChanged".into());
            core_maintenance::write_durable(&path, &journal, true)?;
            continue;
        }
        // An external server can acquire a world lock after the initial
        // validation. Recheck immediately before publishing the registration.
        let idle = scan_policy::verify_import_idle(&checked.source.target).and_then(|()| {
            port_free_at(
                checked.source.address.unwrap_or(Ipv4Addr::LOCALHOST),
                checked.source.port,
            )
        });
        if let Err(error) = idle {
            journal.rows[index] = Outcome::Rejected(bounded_reason(error));
            core_maintenance::write_durable(&path, &journal, true)?;
            continue;
        }
        let payload = imported_record(row, &checked, work_id, journal.selection.automatic)?;
        journal.rows[index] = Outcome::Publishing(payload.clone());
        core_maintenance::write_durable(&path, &journal, true)?;
        core_maintenance::write_durable(&record, &payload, retained.is_some())?;
        journal.rows[index] = Outcome::Imported;
        core_maintenance::write_durable(&path, &journal, true)?;
    }
    Ok(result(root, work_id, &journal))
}

pub(super) fn activity(root: &Path) -> Result<Vec<Value>> {
    let Some(folder) = journal_folder(root, false)? else {
        return Ok(Vec::new());
    };
    let mut activity = Vec::new();
    for entry in fs::read_dir(folder).map_err(fail)? {
        let path = entry.map_err(fail)?.path();
        if path.extension().is_none_or(|suffix| suffix != "json") {
            continue;
        }
        let work_id = path
            .file_stem()
            .and_then(|name| name.to_str())
            .filter(|id| token(id))
            .ok_or("BatchImportResultUnconfirmed")?;
        let journal = read_journal(&path)?;
        let view = result(root, work_id, &journal);
        for (index, (row, item)) in journal
            .selection
            .rows
            .iter()
            .zip(view["rows"].as_array().into_iter().flatten())
            .enumerate()
        {
            let target = scope(root, &row.directory)?;
            if !scan_policy::admitted_path(root, &row.name, &target)? {
                continue;
            }
            let state = match item["state"].as_str() {
                Some("Imported") => "Completed",
                Some("Rejected") => "Rejected",
                _ => "Unconfirmed",
            };
            activity.push(json!({"id":format!("{work_id}:{index}"),"action":"import-batch","instance":row.name,
                "createdAt":journal.created_at,"state":state}));
        }
    }
    Ok(activity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::BTreeMap,
        net::TcpListener,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        root: PathBuf,
        java: String,
    }

    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "ih-batch-import-{}-{}-{}",
                std::process::id(),
                clock(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
            ));
            super::super::dispatch(&[path.to_str().unwrap().into(), "init".into()]).unwrap();
            let root = safe_directory(&path).unwrap();
            let java = root.join("java.exe");
            // An inert file ensures preview and commit never execute Java.
            fs::write(&java, b"synthetic-java").unwrap();
            Self {
                root,
                java: path_format::display(&java),
            }
        }

        fn server(&self, directory: &str, port: u16) {
            let dir = self.root.join(directory);
            fs::create_dir_all(dir.join("world")).unwrap();
            fs::write(dir.join("server.jar"), b"fixture").unwrap();
            fs::write(
                dir.join("server.properties"),
                format!("server-ip=127.0.0.1\nserver-port={port}\n"),
            )
            .unwrap();
            fs::write(dir.join("world").join("level.dat"), b"unchanged world").unwrap();
        }

        fn call(&self, action: &str, args: &[String]) -> Result<Value> {
            let mut invocation = vec![path_format::display(&self.root), action.to_owned()];
            invocation.extend_from_slice(args);
            super::super::dispatch(&invocation)
        }

        fn preview(&self) -> Value {
            self.call(
                "import-batch-preview",
                &[json!({"directory":"instances","java":self.java}).to_string()],
            )
            .unwrap()
        }

        fn select(&self, names: &[&str]) -> Selection {
            let report = self.preview();
            assert_eq!(report["complete"], true);
            let rows = report["rows"].as_array().unwrap();
            Selection {
                directory: "instances".into(),
                automatic: false,
                rows: names
                    .iter()
                    .map(|name| {
                        let item = rows
                            .iter()
                            .find(|item| item["name"].as_str() == Some(*name))
                            .unwrap();
                        assert_eq!(item["state"], "Ready", "{item}");
                        Selected {
                            directory: item["directory"].as_str().unwrap().into(),
                            name: (*name).into(),
                            java: self.java.clone(),
                            listen_address: item["listenAddress"].as_str().map(str::to_owned),
                            revision: item["revision"].as_str().unwrap().into(),
                        }
                    })
                    .collect(),
            }
        }

        fn apply(&self, id: &str, selection: &Selection) -> Result<Value> {
            self.call(
                "import-batch-apply",
                &[id.into(), serde_json::to_string(selection).unwrap()],
            )
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }

    fn ports(count: usize) -> Vec<u16> {
        let reserved: Vec<_> = (0..count)
            .map(|_| TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap())
            .collect();
        reserved
            .iter()
            .map(|listener| listener.local_addr().unwrap().port())
            .collect()
    }

    fn snapshot(path: &Path) -> BTreeMap<PathBuf, Option<(String, u64)>> {
        fn entries(path: &Path, output: &mut BTreeMap<PathBuf, Option<(String, u64)>>) {
            for entry in fs::read_dir(path).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                let meta = fs::symlink_metadata(&path).unwrap();
                if meta.is_dir() {
                    output.insert(path.clone(), None);
                    entries(&path, output);
                } else {
                    output.insert(
                        path.clone(),
                        Some((
                            content_revision::file_revision(&path).unwrap(),
                            meta.last_write_time(),
                        )),
                    );
                }
            }
        }
        let mut result = BTreeMap::new();
        entries(path, &mut result);
        result
    }

    #[test]
    fn management_preview_reads_sources_without_java_and_does_not_register_unknown_core() {
        let f = Fixture::new();
        f.server("instances/scope/unknown", ports(1)[0]);
        let before = snapshot(&f.root);
        let policy = f.call("scan-policy", &[]).unwrap();
        let request = json!({"directory":"instances/scope","revision":policy["revision"]});
        let view = f.call("management-folder-preview", &[request.to_string()]).unwrap();
        assert_eq!(view["complete"], true);
        assert_eq!(view["rows"][0]["state"], "Blocked");
        assert_eq!(view["rows"][0]["reason"], "UnverifiedManagementCore");
        assert_eq!(view["rows"][0]["java"], "");
        assert_eq!(snapshot(&f.root), before);

        let request = json!({
            "directory":"instances/scope","revision":policy["revision"],
            "previewRevision":view["previewRevision"],"excludedDirectories":[]
        });
        let saved = f.call("management-folder-save", &[request.to_string()]).unwrap();
        assert_eq!(saved["policy"]["managedFolders"], true);
        assert!(saved["registration"].is_null());
        let sources = f.root.join("instances");
        assert_eq!(
            snapshot(&sources),
            before.into_iter().filter(|(p, _)| p != &sources && p.starts_with(&sources)).collect()
        );
        assert!(!record_path(&f.root, "unknown").exists());
        assert_eq!(
            f.call("management-folder-save", &[request.to_string()]).unwrap_err(),
            "SettingsConflict"
        );
    }

    #[test]
    fn workspace_instances_container_is_not_an_import_candidate() {
        let f = Fixture::new();
        let available = ports(2);
        f.server("instances", available[0]);
        f.server("instances/scope/unknown", available[1]);
        let policy = f.call("scan-policy", &[]).unwrap();
        let request = json!({"directory":"instances","revision":policy["revision"]});
        let view = f.call("management-folder-preview", &[request.to_string()]).unwrap();
        assert_eq!(view["complete"], true);
        assert_eq!(view["rows"][0]["directory"], "instances");
        assert_eq!(view["rows"][0]["reason"], "ImportOutsideInstanceRoot");
        assert_eq!(view["rows"][1]["directory"], "instances/scope/unknown");
        assert_eq!(view["rows"][1]["reason"], "UnverifiedManagementCore");
    }

    #[test]
    fn management_policy_rejects_invalid_exclusions_without_changing_the_saved_policy() {
        let f = Fixture::new();
        fs::create_dir_all(f.root.join("instances/include/exclude")).unwrap();
        fs::create_dir(f.root.join("instances/neighbor")).unwrap();
        for (path, reason) in [
            ("instances/include", "ScanExclusionOutsideWhitelist"),
            ("instances/neighbor", "ScanExclusionOutsideWhitelist"),
            ("instances/include/missing", "ScanDirectoryUnavailable"),
        ] {
            let policy = f.call("scan-policy", &[]).unwrap();
            let request = json!({
                "revision":policy["revision"], "allowPaths":["instances/include"], "excludePaths":[path],
            });
            assert_eq!(
                f.call("save-scan-policy", &[request.to_string()]).unwrap_err(),
                format!("InvalidManagementPath exclude 0 {reason}")
            );
            assert_eq!(f.call("scan-policy", &[]).unwrap(), policy);
        }
        let policy = f.call("scan-policy", &[]).unwrap();
        let request = json!({
            "revision":policy["revision"], "allowPaths":["instances/include"], "excludePaths":["instances/include/exclude"],
        });
        let saved = f.call("save-scan-policy", &[request.to_string()]).unwrap();
        assert_eq!(saved["policy"]["managedFolders"], true);
        let (policy, _) = scan_policy::load(&f.root).unwrap();
        assert!(policy.matches(&f.root, "include", &f.root.join("instances/include")).unwrap().1);
        assert!(!policy.matches(&f.root, "exclude", &f.root.join("instances/include/exclude")).unwrap().1);
        assert!(!policy.matches(&f.root, "neighbor", &f.root.join("instances/neighbor")).unwrap().1);
    }

    #[test]
    fn preview_is_read_only_and_lists_occupied_legacy_excluded_and_malformed_rows() {
        let f = Fixture::new();
        let available = ports(3);
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        f.server(
            "instances/scope/occupied",
            listener.local_addr().unwrap().port(),
        );
        f.server("instances/scope/legacy", available[0]);
        fs::write(f.root.join("instances/scope/legacy/workspace.json"), b"{}").unwrap();
        f.server("instances/scope/excluded", available[1]);
        fs::write(
            f.root.join("scan-policy.json"),
            br#"{"excludePaths":["instances/scope/excluded"]}"#,
        )
        .unwrap();
        f.server("instances/scope/ready", available[2]);
        fs::create_dir(f.root.join("instances/scope/broken")).unwrap();
        fs::write(
            f.root.join("instances/scope/broken/server.properties"),
            b"server-port=?",
        )
        .unwrap();
        let before = snapshot(&f.root);
        let result = f.preview();
        let rows = result["rows"].as_array().unwrap();
        let row = |name: &str| {
            rows.iter()
                .find(|item| item["name"].as_str() == Some(name))
                .unwrap()
        };
        assert_eq!(row("ready")["state"], "Ready");
        assert_eq!(row("legacy")["reason"], "ImportManagementUnconfirmed");
        assert_eq!(row("occupied")["reason"], "PortOccupiedOrUnavailable");
        assert_eq!(row("excluded")["reason"], "InstanceExcludedOrInvalid");
        assert_eq!(row("broken")["reason"], "InvalidServerPort");
        assert_eq!(
            f.call(
                "import-batch-preview",
                &[json!({"directory":"instances/../outside","java":f.java}).to_string()]
            )
            .unwrap_err(),
            "InvalidScanPath"
        );
        assert_eq!(
            f.call("import-batch-status", &["never-submitted".into()])
                .unwrap_err(),
            "BatchImportNotFound"
        );
        assert_eq!(snapshot(&f.root), before);
    }

    #[test]
    fn incomplete_scan_does_not_issue_confirmation_tokens() {
        let f = Fixture::new();
        f.server("instances/ready", ports(1)[0]);
        fs::create_dir_all(f.root.join("instances/one/two/three/four/five/six")).unwrap();
        let report = f.preview();
        assert_eq!(report["complete"], false);
        assert_eq!(report["rows"][0]["name"], "ready");
        assert_eq!(report["rows"][0]["state"], "Blocked");
        assert!(report["rows"][0]["revision"].is_null());
    }

    #[test]
    fn world_lock_taken_after_preview_refuses_registration() {
        let f = Fixture::new();
        f.server("instances/locked", ports(1)[0]);
        let selection = f.select(&["locked"]);
        let path = f.root.join("instances/locked/world/session.lock");
        let active = OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(0)
            .open(path)
            .unwrap();
        let result = f.apply("world-busy", &selection).unwrap();
        assert_eq!(result["rows"][0]["reason"], "ImportManagementUnconfirmed");
        assert!(!record_path(&f.root, "locked").exists());
        drop(active);
    }

    #[test]
    fn conflicting_selection_writes_no_registration_and_only_one_candidate_can_be_chosen() {
        let f = Fixture::new();
        let available = ports(1)[0];
        f.server("instances/group-a/candidate-a", available);
        f.server("instances/group-b/candidate-b", available);
        let report = f.preview();
        assert_eq!(
            report["rows"][0]["conflicts"][0],
            "instances/group-b/candidate-b"
        );
        let both = f.select(&["candidate-a", "candidate-b"]);
        assert_eq!(
            f.apply("duplicate-port", &both).unwrap_err(),
            "BatchImportSelectionConflict"
        );
        assert!(!record_path(&f.root, "candidate-a").exists());
        assert!(!journal_folder(&f.root, false).unwrap().is_some());
        let first = f.select(&["candidate-a"]);
        let imported = f.apply("only-first", &first).unwrap();
        assert_eq!(imported["rows"][0]["state"], "Imported");
        assert_eq!(f.preview()["rows"][1]["reason"], "PortAlreadyRegistered");
    }

    #[test]
    fn partial_failure_keeps_success_and_retries_never_change_the_sources() {
        let f = Fixture::new();
        let available = ports(3);
        f.server("instances/stable", available[0]);
        f.server("instances/changed", available[1]);
        f.server("instances/unselected", available[2]);
        let selection = f.select(&["stable", "changed"]);
        fs::write(
            f.root.join("instances/changed/server.jar"),
            b"altered fixture",
        )
        .unwrap();
        let before = snapshot(&f.root.join("instances"));
        let applied = f.apply("partial", &selection).unwrap();
        assert_eq!(applied["state"], "Complete");
        assert_eq!(applied["rows"][0]["state"], "Imported");
        assert_eq!(applied["rows"][1]["reason"], "BatchImportPreviewChanged");
        assert!(!record_path(&f.root, "unselected").exists());
        assert!(!record_path(&f.root, "changed").exists());
        assert_eq!(snapshot(&f.root.join("instances")), before);
        let after_commit = snapshot(&f.root);
        assert_eq!(f.apply("partial", &selection).unwrap(), applied);
        assert_eq!(
            f.call("import-batch-status", &["partial".into()]).unwrap(),
            applied
        );
        assert_eq!(snapshot(&f.root), after_commit);
        let other = f.select(&["unselected"]);
        assert_eq!(f.apply("partial", &other).unwrap_err(), "RequestConflict");
    }

    #[test]
    fn restart_reconciles_only_a_record_published_by_the_interrupted_batch() {
        let f = Fixture::new();
        let available = ports(2);
        f.server("instances/published", available[0]);
        f.server("instances/waiting", available[1]);
        let selection = f.select(&["published", "waiting"]);
        let pub_row = &selection.rows[0];
        let target = scope(&f.root, &pub_row.directory).unwrap();
        let mut row = new_row(&f.root, &target, Some(&pub_row.name), &pub_row.java).unwrap();
        let checked = inspect_candidate(
            &f.root,
            &target,
            &pub_row.name,
            &pub_row.java,
            None,
            &mut row,
            None,
        )
        .unwrap();
        let record = imported_record(pub_row, &checked, "interrupted", false).unwrap();
        drop(checked);
        let path = journal_folder(&f.root, true)
            .unwrap()
            .unwrap()
            .join("interrupted.json");
        let journal = Journal {
            version: 1,
            created_at: 0,
            selection: selection.clone(),
            rows: vec![Outcome::Publishing(record.clone()), Outcome::Waiting],
        };
        core_maintenance::write_durable(&path, &journal, false).unwrap();
        let status = || {
            f.call("import-batch-status", &["interrupted".into()])
                .unwrap()
        };
        assert_eq!(status()["rows"][0]["state"], "NeedsConfirmation");
        let mut foreign = record.clone();
        foreign["batchImportWorkId"] = json!("foreign");
        let registry = record_path(&f.root, "published");
        core_maintenance::write_durable(&registry, &foreign, false).unwrap();
        assert_eq!(status()["rows"][0]["state"], "NeedsConfirmation");
        core_maintenance::write_durable(&registry, &record, true).unwrap();
        let committed = status();
        assert_eq!(committed["state"], "Interrupted");
        assert_eq!(committed["rows"][0]["state"], "Imported");
        assert_eq!(committed["rows"][1]["state"], "NeedsConfirmation");
        assert_eq!(f.apply("interrupted", &selection).unwrap(), committed);
        assert!(!record_path(&f.root, "waiting").exists());
        let _lock = command_lock(&f.root, "port-registration.lock", false).unwrap();
        assert_eq!(f.apply("competing", &selection).unwrap_err(), "ManagerBusy");
    }
}

pub(super) fn dispatch(root: &Path, verb: &str, args: &[String]) -> Option<Result<Value>> {
    Some(match (verb, args) {
        ("management-folder-preview", [raw]) => parse(raw).and_then(|request: FolderPreview| {
            folder_preview(root, &request.directory, &request.revision).map(|(view, _, _)| view)
        }),
        ("management-folder-save", [raw]) => parse(raw).and_then(|request| folder_save(root, request)),
        ("import-batch-preview", [raw]) => parse(raw).and_then(|request| preview(root, request)),
        ("import-batch-apply", [id, raw]) if token(id) => {
            parse(raw).and_then(|selection| apply(root, id, selection))
        }
        ("import-batch-status", [id]) if token(id) => (|| {
            let folder = journal_folder(root, false)?.ok_or("BatchImportNotFound")?;
            let path = folder.join(format!("{id}.json"));
            if raw_entry(&path)?.is_none() {
                return Err("BatchImportNotFound".into());
            }
            let lock_path = root.join("port-registration.lock");
            let _read_guard = if raw_entry(&lock_path)?.is_some() {
                no_redirect(&lock_path)?;
                let file = File::open(lock_path).map_err(fail)?;
                file.try_lock_shared().map_err(|_| "ManagerBusy")?;
                Some(file)
            } else {
                None
            };
            Ok(result(root, id, &read_journal(&path)?))
        })(),
        ("management-folder-preview" | "management-folder-save" | "import-batch-preview" | "import-batch-apply" | "import-batch-status", _) => {
            Err("InvalidBatchImportArguments".into())
        }
        _ => return None,
    })
}
