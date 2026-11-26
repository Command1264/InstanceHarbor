//! Explicit, stopped-instance JAR replacement with recoverable evidence.
use super::*;
use std::os::windows::{
    ffi::OsStrExt,
    fs::{MetadataExt, OpenOptionsExt},
};
use std::path::{Component, Prefix};

const STORE: &str = ".instanceharbor-core-maintenance";
const PENDING: &str = "pending.json";
const REPARSE_POINT: u32 = 0x400;
const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const BACKUP_SEMANTICS: u32 = 0x0200_0000;
const MAX_JAR_BYTES: u64 = 512 * 1024 * 1024;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn MoveFileExW(source: *const u16, destination: *const u16, flags: u32) -> i32;
    fn GetDriveTypeW(root: *const u16) -> u32;
}

/// The step a core update or restore is in, reported to the GUI (#60).
/// `Downloading` carries the official size when the source states one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "camelCase")]
pub enum CoreUpdatePhase {
    Checking,
    Downloading { received: u64, total: Option<u64> },
    /// The candidate with this SHA-256 was already downloaded, for example by
    /// an earlier item of the same batch or by the prefetch setting.
    Reusing,
    Verifying,
    BackingUp,
    Replacing,
}

/// A progress sink that ignores every update.
pub(super) fn no_progress(_: CoreUpdatePhase) {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    deny_unknown_fields
)]
enum Action {
    Install { candidate_sha256: String },
    Restore { backup_id: String },
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Intent {
    request: String,
    action: Action,
    name: String,
    directory: PathBuf,
    port: u16,
    registration_revision: String,
    intended_record_revision: String,
    before: CoreProfile,
    after: CoreProfile,
    created_at: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Outcome {
    Applied,
    NotApplied,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Completed {
    intent: Intent,
    outcome: Outcome,
}

#[derive(Deserialize)]
struct RunRelease {
    run: Run,
    exit: i32,
}

fn checked_entry(path: &Path, directory: bool) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_attributes() & REPARSE_POINT != 0
                || !(if directory {
                    metadata.is_dir()
                } else {
                    metadata.is_file()
                })
            {
                return Err("CoreMaintenanceUnsafePath".into());
            }
            no_redirect(path)?;
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(file_error("inspect-core-maintenance", path, error)),
    }
}

fn existing(path: &Path, directory: bool) -> Result<()> {
    if checked_entry(path, directory)? {
        Ok(())
    } else {
        Err("CoreMaintenanceFileMissing".into())
    }
}

fn hold_directory(path: &Path) -> Result<File> {
    existing(path, true)?;
    let handle = OpenOptions::new()
        .read(true)
        .share_mode(3)
        .custom_flags(BACKUP_SEMANTICS | OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|error| file_error("pin-core-directory", path, error))?;
    if handle.metadata().map_err(fail)?.file_attributes() & REPARSE_POINT != 0 {
        return Err("CoreMaintenanceUnsafePath".into());
    }
    Ok(handle)
}

pub(super) fn read_optional<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    if !checked_entry(path, false)? {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|error| file_error("open-core-metadata", path, error))?
        .take(64 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| file_error("read-core-metadata", path, error))?;
    if bytes.len() > 64 * 1024 {
        return Err("CoreMaintenanceMetadataTooLarge".into());
    }
    serde_json::from_slice(&bytes).map(Some).map_err(fail)
}

pub(super) fn move_durable(source: &Path, destination: &Path, replace: bool) -> Result<()> {
    move_durable_with_stage(source, destination, replace, "publish-core-file")
}

pub(super) fn move_durable_with_stage(source: &Path, destination: &Path, replace: bool, stage: &str) -> Result<()> {
    let from: Vec<_> = source.as_os_str().encode_wide().chain([0]).collect();
    let to: Vec<_> = destination.as_os_str().encode_wide().chain([0]).collect();
    // WRITE_THROUGH and, only for an already-verified destination, REPLACE_EXISTING.
    let flags = 0x8 | u32::from(replace);
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), flags) } == 0 {
        return Err(file_error(
            stage,
            destination,
            io::Error::last_os_error(),
        ));
    }
    Ok(())
}

pub(super) fn write_durable(path: &Path, value: &impl Serialize, replace: bool) -> Result<()> {
    if checked_entry(path, false)? != replace {
        return Err("CoreMaintenanceDestinationChanged".into());
    }
    let bytes = serde_json::to_vec_pretty(value).map_err(fail)?;
    let temporary = path.with_extension(format!("{}.{}.tmp", std::process::id(), clock()));
    let mut output = File::create_new(&temporary)
        .map_err(|error| file_error("create-core-metadata", &temporary, error))?;
    output.write_all(&bytes).map_err(fail)?;
    output.sync_all().map_err(fail)?;
    drop(output);
    move_durable(&temporary, path, replace)
}

pub(super) fn copy_verified(source: &Path, destination: &Path, sha256: &str) -> Result<()> {
    existing(source, false)?;
    let mut input = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(OPEN_REPARSE_POINT)
        .open(source)
        .map_err(|error| file_error("open-core-source", source, error))?;
    let metadata = input.metadata().map_err(fail)?;
    if metadata.file_attributes() & REPARSE_POINT != 0 || !metadata.is_file() {
        return Err("CoreMaintenanceUnsafePath".into());
    }
    if metadata.len() > MAX_JAR_BYTES {
        return Err("CoreArtifactTooLarge".into());
    }
    let mut output = File::create_new(destination)
        .map_err(|error| file_error("create-core-copy", destination, error))?;
    let copied = io::copy(&mut (&mut input).take(MAX_JAR_BYTES + 1), &mut output)
        .map_err(|error| file_error("copy-core-file", destination, error))?;
    if copied != metadata.len() {
        return Err("CoreArtifactChanged".into());
    }
    output.sync_all().map_err(fail)?;
    drop(output);
    require_hash(destination, sha256)
}

fn hash(path: &Path) -> Result<String> {
    existing(path, false)?;
    content_revision::file_revision(path)
}

fn require_hash(path: &Path, expected: &str) -> Result<()> {
    if hash(path)? == expected {
        Ok(())
    } else {
        Err("CoreArtifactHashMismatch".into())
    }
}

fn candidate_path(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err("CoreCandidatePathMustBeAbsolute".into());
    }
    let Some(Component::Prefix(prefix)) = path.components().next() else {
        return Err("CoreCandidateMustBeLocal".into());
    };
    let drive = match prefix.kind() {
        Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => drive,
        _ => return Err("CoreCandidateMustBeLocal".into()),
    };
    let drive_root = [u16::from(drive), 58, 92, 0];
    if !matches!(unsafe { GetDriveTypeW(drive_root.as_ptr()) }, 2 | 3 | 5 | 6) {
        return Err("CoreCandidateMustBeLocal".into());
    }
    let mut prefix = PathBuf::new();
    for component in path.components() {
        prefix.push(component.as_os_str());
        match component {
            Component::Prefix(_) | Component::RootDir => continue,
            Component::ParentDir => return Err("CoreMaintenanceUnsafePath".into()),
            _ => {}
        }
        if fs::symlink_metadata(&prefix)
            .map_err(fail)?
            .file_attributes()
            & REPARSE_POINT
            != 0
        {
            return Err("CoreMaintenanceUnsafePath".into());
        }
    }
    let resolved = path.canonicalize().map_err(fail)?;
    existing(&resolved, false)?;
    Ok(resolved)
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(super) fn trusted_profile(config: &Instance, sha256: String) -> Option<CoreProfile> {
    pinned_core_profile(sha256.clone()).or_else(|| {
        config
            .core_profile
            .as_ref()
            .filter(|profile| profile.sha256 == sha256 && !profile.unverified)
            .cloned()
    })
}

fn verify_pair(before: &CoreProfile, after: &CoreProfile, action: &Action) -> Result<()> {
    if !valid_hash(&before.sha256)
        || !valid_hash(&after.sha256)
        || before.family != after.family
        || before.minecraft != after.minecraft
        || before.sha256 == after.sha256
        || before.build == after.build
    {
        return Err("CoreCandidateIncompatible".into());
    }
    if let Action::Install { candidate_sha256 } = action {
        if candidate_sha256 != &after.sha256 {
            return Err("CoreArtifactHashMismatch".into());
        }
        if let (Ok(current), Ok(candidate)) =
            (before.build.parse::<u64>(), after.build.parse::<u64>())
        {
            if candidate <= current {
                return Err("CoreCandidateIsNotNewer".into());
            }
        }
    }
    Ok(())
}

struct Target {
    config: Instance,
    directory: PathBuf,
}

impl Target {
    fn resolve(root: &Path, name: &str) -> Result<Self> {
        if !instance_name(name) || !admitted(root, name)? {
            return Err("InstanceExcludedOrInvalid".into());
        }
        let config = instance(root, name)?;
        if config.removed {
            return Err("InstanceRemovedFromManagement".into());
        }
        let directory = managed_directory(root, name, config.directory.as_deref())?;
        Ok(Self { config, directory })
    }

    fn store(&self) -> PathBuf {
        self.directory.join(STORE)
    }

    fn jar(&self) -> PathBuf {
        core_artifact::jar(&self.directory, &self.config)
    }

    fn pending(&self) -> Result<Option<Intent>> {
        read_pending(&self.directory)
    }

    fn stopped(&self, root: &Path) -> Result<()> {
        // A Velocity proxy states its port in velocity.toml, not server.properties (#41).
        let configured = if registered_velocity(&self.config).is_some() {
            velocity_binding(&self.directory)?.0 .1
        } else {
            property_port(&self.directory)?
        };
        if configured != self.config.port {
            return Err("PortChanged".into());
        }
        port_unassigned(root, self.config.port, &self.config.name)?;
        let observed = status(root, &self.config)?;
        if observed["state"] != "Stopped" {
            return Err("InstanceNotConfirmedStopped".into());
        }
        instance_port_free(&self.config)?;
        if observed["run"].is_null() {
            return self
                .config
                .core_local_create
                .then_some(())
                .ok_or_else(|| "CoreStopEvidenceMissing".to_owned());
        }
        let run: Run = serde_json::from_value(observed["run"].clone()).map_err(fail)?;
        let folder = root.join("runs").join(&self.config.name);
        let release: RunRelease =
            read_optional(&folder.join("run-release.json"))?.ok_or("CoreStopEvidenceMissing")?;
        let exit: Option<Option<i32>> = read_optional(&folder.join("exit.json"))?;
        if release.run != run || release.exit != 0 || exit != Some(Some(0)) {
            return Err("CoreStopEvidenceUnconfirmed".into());
        }
        Ok(())
    }

    fn validate_intent(&self, intent: &Intent, request: &str) -> Result<()> {
        if intent.request != request
            || !token(request)
            || intent.name != self.config.name
            || intent.directory != self.directory
            || intent.port == 0
            || !valid_hash(&intent.registration_revision)
            || !valid_hash(&intent.intended_record_revision)
            || matches!(&intent.action, Action::Restore { backup_id } if !token(backup_id))
        {
            return Err("CoreMaintenanceIdentityChanged".into());
        }
        verify_pair(&intent.before, &intent.after, &intent.action)
    }

    fn bind(&self, intent: &Intent, request: &str) -> Result<()> {
        self.validate_intent(intent, request)?;
        if intent.port != self.config.port {
            return Err("CoreMaintenanceIdentityChanged".into());
        }
        Ok(())
    }
}

fn read_pending(directory: &Path) -> Result<Option<Intent>> {
    let store = directory.join(STORE);
    if !checked_entry(&store, true)? {
        return Ok(None);
    }
    read_optional(&store.join(PENDING))
}

pub(super) fn assert_start_allowed(directory: &Path) -> Result<()> {
    if read_pending(directory)?.is_some() {
        return Err(
            "CoreMaintenancePending: reconcile the recorded request before starting".into(),
        );
    }
    Ok(())
}

fn completed(target: &Target, request: &str) -> Result<Option<Completed>> {
    let folder = target.store().join(request);
    if !checked_entry(&folder, true)? {
        return Ok(None);
    }
    let result: Option<Completed> = read_optional(&folder.join("result.json"))?;
    if let Some(result) = &result {
        target.validate_intent(&result.intent, request)?;
    }
    Ok(result)
}

fn report(result: &Completed, replayed: bool) -> Value {
    let id = &result.intent.request;
    json!({"workId": id, "request": id, "outcome": result.outcome,
        "installed": if result.outcome == Outcome::Applied { &result.intent.after } else { &result.intent.before },
        "backupId": (result.outcome == Outcome::Applied).then_some(id), "replayed": replayed})
}

pub(super) fn activity(root: &Path, name: &str) -> Result<Vec<Value>> {
    let target = Target::resolve(root, name)?;
    let store = target.store();
    if !checked_entry(&store, true)? {
        return Ok(Vec::new());
    }
    let pending = target.pending()?;
    let mut rows = Vec::new();
    if let Some(intent) = &pending {
        target.bind(intent, &intent.request)?;
        rows.push(
            json!({"id":intent.request,"action":format!("core-{}",kind(&intent.action)),
            "instance":name,"state":"Unconfirmed","createdAt":intent.created_at}),
        );
    }
    for entry in fs::read_dir(&store).map_err(fail)? {
        let entry = entry.map_err(fail)?;
        let filename = entry.file_name();
        let Some(id) = filename.to_str().filter(|id| token(id)) else {
            continue;
        };
        if pending.as_ref().is_some_and(|intent| intent.request == id) {
            continue;
        }
        if let Some(result) = completed(&target, id)? {
            rows.push(json!({"id":id,"action":format!("core-{}",kind(&result.intent.action)),"instance":name,
                "state":if result.outcome == Outcome::Applied {"Completed"} else {"Failed"},
                "createdAt":result.intent.created_at}));
        }
    }
    Ok(rows)
}

fn state(root: &Path, name: &str) -> Result<Value> {
    let target = Target::resolve(root, name)?;
    let pending = target.pending()?;
    if let Some(pending) = &pending {
        target.bind(pending, &pending.request)?;
    }
    let current = hash(&target.jar());
    let installed = current
        .as_ref()
        .ok()
        .and_then(|sha| trusted_profile(&target.config, sha.clone()))
        .or_else(|| {
            pending.as_ref().and_then(|pending| {
                let sha = current.as_ref().ok()?;
                (sha == &pending.after.sha256).then(|| pending.after.clone())
            })
        });
    let reason = if pending.is_some() {
        Some("CoreMaintenancePending".to_owned())
    } else if installed.is_none() {
        Some(
            current
                .as_ref()
                .err()
                .cloned()
                .unwrap_or_else(|| "CoreProfileUnverified".to_owned()),
        )
    } else {
        target.stopped(root).err()
    };
    let mut backups = Vec::new();
    let store = target.store();
    if checked_entry(&store, true)? {
        for entry in fs::read_dir(&store).map_err(fail)? {
            let entry = entry.map_err(fail)?;
            let filename = entry.file_name();
            let Some(id) = filename.to_str().filter(|name| token(name)) else {
                continue;
            };
            if pending.as_ref().is_some_and(|intent| intent.request == id) {
                continue;
            }
            if let Some(result) = completed(&target, id)? {
                if result.outcome == Outcome::Applied
                    && installed.as_ref().is_some_and(|profile| {
                        profile.sha256 != result.intent.before.sha256
                            && profile.family == result.intent.before.family
                            && profile.minecraft == result.intent.before.minecraft
                    })
                {
                    backups.push((
                        result.intent.created_at,
                        id.to_owned(),
                        result.intent.before,
                    ));
                }
            }
        }
    }
    backups.sort_by(|left, right| (right.0, &right.1).cmp(&(left.0, &left.1)));
    let backups: Vec<Value> = backups.into_iter().map(|(created_at, id, profile)| {
        json!({"createdAt": created_at, "id": id, "profile": profile})
    }).collect();
    let pending = pending.map(|intent| {
        json!({"request":intent.request, "kind":kind(&intent.action), "action":intent.action,
        "before":intent.before, "after":intent.after})
    });
    let prepared = current.as_ref().ok().and_then(|hash| {
        core_prefetch::status(root, name, hash).ok().flatten()
    });
    // What an unverified JAR states about itself, shown as an offline reading
    // (#48). It does not make the core writable: `reason` still applies.
    let identified = installed
        .is_none()
        .then(|| core_identity::read_identity(&target.jar()).ok().flatten())
        .flatten()
        .map(|identity| {
            json!({"family":identity.family,"minecraft":identity.minecraft_version,"build":identity.build})
        });
    Ok(
        json!({"name":name,"installed":installed,"identified":identified,"currentSha256":current.ok(),"pending":pending,
        "backups":backups,"canWrite":reason.is_none(),"reason":reason,"prepared":prepared}),
    )
}

pub(super) fn require_stopped(root: &Path, name: &str) -> Result<()> {
    let target = Target::resolve(root, name)?;
    assert_start_allowed(&target.directory)?;
    target.stopped(root)
}

fn kind(action: &Action) -> &'static str {
    match action {
        Action::Install { .. } => "install",
        Action::Restore { .. } => "restore",
    }
}

fn compute_outcome(installed: &str, intent: &Intent) -> Result<Outcome> {
    if installed == intent.before.sha256 {
        Ok(Outcome::NotApplied)
    } else if installed == intent.after.sha256 {
        Ok(Outcome::Applied)
    } else {
        Err(
            "CoreMaintenanceResultUnconfirmed: the installed JAR differs from both recorded hashes"
                .into(),
        )
    }
}

fn config_revision(config: &Instance) -> Result<String> {
    content_revision::revision(&serde_json::to_vec(config).map_err(fail)?)
}

fn update_config(root: &Path, target: &Target, intent: &Intent, outcome: Outcome) -> Result<()> {
    let config = instance(root, &target.config.name)?;
    let revision = registration_revision(root, &target.config.name)?;
    if outcome == Outcome::Applied && config_revision(&config)? == intent.intended_record_revision {
        return Ok(());
    }
    if revision != intent.registration_revision {
        return Err("CoreMaintenanceRegistrationChanged".into());
    }
    if outcome == Outcome::Applied {
        let mut next = config;
        next.core_profile = Some(intent.after.clone());
        if config_revision(&next)? != intent.intended_record_revision {
            return Err("CoreMaintenanceRegistrationChanged".into());
        }
        write_durable(&record_path(root, &target.config.name), &next, true)?;
    }
    Ok(())
}

fn finish(root: &Path, target: &Target, intent: Intent, replayed: bool) -> Result<Value> {
    target.bind(&intent, &intent.request)?;
    let store = target.store();
    let folder = store.join(&intent.request);
    let _folder = hold_directory(&folder)?;
    let installed = hash(&target.jar())?;
    let outcome = compute_outcome(&installed, &intent)?;
    if outcome == Outcome::Applied {
        require_hash(&folder.join("before.jar"), &intent.before.sha256)?;
    }
    update_config(root, target, &intent, outcome)?;
    let result_path = folder.join("result.json");
    let result = Completed { intent, outcome };
    if let Some(saved) = completed(target, &result.intent.request)? {
        let expected = serde_json::to_vec(&result).map_err(fail)?;
        if serde_json::to_vec(&saved).map_err(fail)? != expected {
            return Err("CoreMaintenanceRecordedOutcomeChanged".into());
        }
    } else {
        write_durable(&result_path, &result, false)?;
    }
    if result.outcome == Outcome::NotApplied {
        for name in ["next.jar", "before.jar"] {
            let unused = folder.join(name);
            if checked_entry(&unused, false)? {
                fs::remove_file(&unused)
                    .map_err(|error| file_error("discard-unapplied-core", &unused, error))?;
            }
        }
    }
    // Publish the completed receipt before removing the start guard. A lost
    // acknowledgement can be reconciled or replayed with the original ID.
    let pending = store.join(PENDING);
    let settled = folder.join("settled.json");
    if !checked_entry(&pending, false)? || checked_entry(&settled, false)? {
        return Err("CoreMaintenanceJournalChanged".into());
    }
    move_durable(&pending, &settled, false)?;
    Ok(report(&result, replayed))
}

fn run_action(
    root: &Path,
    target: Target,
    request: &str,
    expected_current_sha256: &str,
    action: Action,
    candidate: Option<(&Path, &CoreProfile)>,
    progress: &dyn Fn(CoreUpdatePhase),
    begin_commit: impl FnOnce() -> Result<()>,
) -> Result<Value> {
    if !token(request) || !valid_hash(expected_current_sha256) {
        return Err("InvalidCoreMaintenanceRequest".into());
    }
    agent_rebuild::assert_idle(root, &target.config.name)?;
    let _lease = claim_runtime_directory(&target.directory)?;
    let _directory = hold_directory(&target.directory)?;
    assert_start_allowed(&target.directory)?;
    if let Some(result) = completed(&target, request)? {
        if result.intent.action != action || result.intent.before.sha256 != expected_current_sha256
        {
            return Err("RequestConflict".into());
        }
        return Ok(report(&result, true));
    }
    target.stopped(root)?;
    scan_policy::verify_import_idle(&target.directory)?;
    let before_sha = hash(&target.jar())?;
    if before_sha != expected_current_sha256 {
        return Err("CoreCurrentChanged".into());
    }
    let before = trusted_profile(&target.config, before_sha).ok_or("CoreProfileUnverified")?;
    let store = target.store();
    let (source, after) = match (&action, candidate) {
        (Action::Install { .. }, Some((source, profile))) => {
            (candidate_path(source)?, profile.clone())
        }
        (Action::Restore { backup_id }, None) => {
            if !token(backup_id) || request == backup_id {
                return Err("InvalidCoreBackupId".into());
            }
            let backup = completed(&target, backup_id)?
                .filter(|saved| saved.outcome == Outcome::Applied)
                .ok_or("CoreBackupUnavailable")?;
            (
                store.join(backup_id).join("before.jar"),
                backup.intent.before,
            )
        }
        _ => return Err("InvalidCoreMaintenanceRequest".into()),
    };
    verify_pair(&before, &after, &action)?;
    progress(CoreUpdatePhase::Verifying);
    require_hash(&source, &after.sha256)?;
    begin_commit()?;
    if !checked_entry(&store, true)? {
        fs::create_dir(&store)
            .map_err(|error| file_error("create-core-backup-store", &store, error))?;
    }
    let _store = hold_directory(&store)?;
    let folder = store.join(request);
    // A crash just before publishing the first intent can leave an empty
    // directory. It is safe to reuse only if no preparation has started.
    if !checked_entry(&folder, true)? {
        fs::create_dir(&folder)
            .map_err(|error| file_error("create-core-request", &folder, error))?;
    }
    let _folder = hold_directory(&folder)?;
    if fs::read_dir(&folder).map_err(fail)?.next().is_some() {
        return Err("CoreMaintenanceRequestDirectoryNotEmpty".into());
    }
    let original_registration_revision = registration_revision(root, &target.config.name)?;
    let mut next = target.config.clone();
    next.core_profile = Some(after.clone());
    let intent = Intent {
        request: request.to_owned(),
        action,
        name: target.config.name.clone(),
        directory: target.directory.clone(),
        port: target.config.port,
        registration_revision: original_registration_revision,
        intended_record_revision: config_revision(&next)?,
        before,
        after,
        created_at: epoch_ms(),
    };
    write_durable(&store.join(PENDING), &intent, false)?;
    let prepare_and_replace = || -> Result<()> {
        let stage = folder.join("next.jar");
        copy_verified(&source, &stage, &intent.after.sha256)?;
        progress(CoreUpdatePhase::BackingUp);
        copy_verified(
            &target.jar(),
            &folder.join("before.jar"),
            &intent.before.sha256,
        )?;
        target.stopped(root)?;
        if registration_revision(root, &target.config.name)? != intent.registration_revision {
            return Err("CoreMaintenanceRegistrationChanged".into());
        }
        require_hash(&stage, &intent.after.sha256)?;
        require_hash(&target.jar(), &intent.before.sha256)?;
        progress(CoreUpdatePhase::Replacing);
        move_durable(&stage, &target.jar(), true)
    };
    prepare_and_replace()
        .map_err(|cause| format!("CoreMaintenancePending: reconcile request {request}: {cause}"))?;
    finish(root, &target, intent, false)
        .map_err(|cause| format!("CoreMaintenancePending: reconcile request {request}: {cause}"))
}

#[cfg(test)]
pub(super) fn install_verified(
    root: &Path,
    name: &str,
    request: &str,
    expected_current_sha256: &str,
    candidate_path: &Path,
    profile: &CoreProfile,
) -> Result<Value> {
    install_verified_guarded(root, name, request, expected_current_sha256, candidate_path, profile, &no_progress, || Ok(()))
}

pub(super) fn install_verified_guarded(
    root: &Path,
    name: &str,
    request: &str,
    expected_current_sha256: &str,
    candidate_path: &Path,
    profile: &CoreProfile,
    progress: &dyn Fn(CoreUpdatePhase),
    begin_commit: impl FnOnce() -> Result<()>,
) -> Result<Value> {
    let target = Target::resolve(root, name)?;
    run_action(
        root,
        target,
        request,
        expected_current_sha256,
        Action::Install {
            candidate_sha256: profile.sha256.clone(),
        },
        Some((candidate_path, profile)),
        progress,
        begin_commit,
    )
}

fn install_pinned(
    root: &Path,
    name: &str,
    request: &str,
    current: &str,
    candidate: &str,
    expected: &str,
    progress: &dyn Fn(CoreUpdatePhase),
) -> Result<Value> {
    let profile = pinned_core_profile(expected.to_owned()).ok_or("CoreCandidateUnverified")?;
    install_verified_guarded(root, name, request, current, Path::new(candidate), &profile, progress, || Ok(()))
}

fn reconcile(root: &Path, name: &str, request: &str) -> Result<Value> {
    if !token(request) {
        return Err("InvalidCoreMaintenanceRequest".into());
    }
    let target = Target::resolve(root, name)?;
    let _lease = claim_runtime_directory(&target.directory)?;
    let _directory = hold_directory(&target.directory)?;
    let Some(intent) = target.pending()? else {
        let result = completed(&target, request)?.ok_or("CoreMaintenanceRequestNotFound")?;
        return Ok(report(&result, true));
    };
    target.bind(&intent, request)?;
    let _store = hold_directory(&target.store())?;
    target.stopped(root)?;
    scan_policy::verify_import_idle(&target.directory)?;
    finish(root, &target, intent, true)
}

pub(super) fn dispatch(root: &Path, verb: &str, args: &[String]) -> Option<Result<Value>> {
    dispatch_with_progress(root, verb, args, &no_progress)
}

pub(super) fn dispatch_with_progress(
    root: &Path,
    verb: &str,
    args: &[String],
    progress: &dyn Fn(CoreUpdatePhase),
) -> Option<Result<Value>> {
    Some(match (verb, args) {
        ("core-state", [name]) => state(root, name),
        ("core-install", [name, id, before, path, after]) => {
            install_pinned(root, name, id, before, path, after, progress)
        }
        ("core-restore", [name, id, before, backup]) => {
            Target::resolve(root, name).and_then(|target| {
                run_action(
                    root,
                    target,
                    id,
                    before,
                    Action::Restore {
                        backup_id: backup.clone(),
                    },
                    None,
                    progress,
                    || Ok(()),
                )
            })
        }
        ("core-reconcile", [name, id]) => reconcile(root, name, id),
        ("core-state" | "core-install" | "core-restore" | "core-reconcile", _) => {
            Err("InvalidCoreMaintenanceArguments".into())
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let temporary = std::env::temp_dir().canonicalize().unwrap();
            let path = temporary.join(format!("ih-core-test-{}-{}", std::process::id(), clock()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn workspace(&self) -> Target {
            for name in ["instances", "instances/server", "registry", "runs"] {
                fs::create_dir(self.0.join(name)).unwrap();
            }
            save(&self.0.join("scan-policy.json"), &json!({})).unwrap();
            let directory = self.0.join("instances").join("server");
            fs::write(directory.join("server.jar"), b"first").unwrap();
            fs::write(directory.join("other-data"), b"unmodified data").unwrap();
            let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
                .unwrap()
                .local_addr()
                .unwrap()
                .port();
            fs::write(
                directory.join("server.properties"),
                format!("server-ip=127.0.0.1\nserver-port={port}\n"),
            )
            .unwrap();
            let config = Instance {
                core_file: None,
                name: "server".into(),
                java: self.0.join("unused-java.exe"),
                port,
                velocity_listen_address: None,
                removed: false,
                directory: None,
                jvm: None,
                timing: None,
                auto_restart: None,
                core_profile: Some(profile(b"first", "112")),
                core_local_create: true,
                management_discovered: false,
            };
            save(&record_path(&self.0, "server"), &config).unwrap();
            Target { config, directory }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn profile(bytes: &[u8], build: &str) -> CoreProfile {
        CoreProfile { unverified: false, java: None,
            family: "PAPER".into(),
            minecraft: "26.2".into(),
            build: build.into(),
            sha256: content_revision::revision(bytes).unwrap(),
        }
    }

    #[test]
    fn verifies_local_bytes_before_install_and_restores_explicit_backup() {
        let fixture = Fixture::new();
        let target = fixture.workspace();
        let candidate = fixture.0.join("pinned-test.jar");
        let before = profile(b"first", "112");
        let after = profile(b"second", "134");
        let install =
            |id| install_verified(&fixture.0, "server", id, &before.sha256, &candidate, &after);
        fs::write(&candidate, b"invalid download").unwrap();
        assert_eq!(install("bad").unwrap_err(), "CoreArtifactHashMismatch");
        assert!(!target.store().exists());
        fs::write(&candidate, b"second").unwrap();
        let result = install("update").unwrap();
        assert_eq!(result["outcome"], "applied");
        assert_eq!(fs::read(target.jar()).unwrap(), b"second");
        assert_eq!(
            state(&fixture.0, "server").unwrap()["installed"]["build"],
            "134"
        );
        fs::remove_file(&candidate).unwrap();
        assert_eq!(install("update").unwrap()["replayed"], true);
        let restore = run_action(
            &fixture.0,
            Target::resolve(&fixture.0, "server").unwrap(),
            "restore",
            &after.sha256,
            Action::Restore {
                backup_id: "update".into(),
            },
            None,
            &no_progress,
            || Ok(()),
        )
        .unwrap();
        assert_eq!(restore["outcome"], "applied");
        assert_eq!(fs::read(target.jar()).unwrap(), b"first");
        let history = activity(&fixture.0, "server").unwrap();
        assert_eq!(history.len(), 2);
        assert!(history.iter().any(|row| row["id"] == "update"
            && row["action"] == "core-install"
            && row["state"] == "Completed"));
        assert!(history.iter().any(|row| row["id"] == "restore"
            && row["action"] == "core-restore"
            && row["state"] == "Completed"));
        assert_eq!(
            fs::read(target.directory.join("other-data")).unwrap(),
            b"unmodified data"
        );
        assert!(assert_start_allowed(&target.directory).is_ok());
    }

    // #41: a Velocity proxy is checked against velocity.toml and keeps the
    // registered core file name after a build update and its restore.
    #[test]
    fn velocity_build_update_keeps_the_registered_name_and_restores() {
        let fixture = Fixture::new();
        let mut target = fixture.workspace();
        let directory = target.directory.clone();
        fs::remove_file(directory.join("server.properties")).unwrap();
        fs::rename(directory.join("server.jar"), directory.join("velocity-4.2.1-SNAPSHOT-38.jar")).unwrap();
        let velocity = |bytes: &[u8], build: &str| CoreProfile { family: "VELOCITY".into(), minecraft: "4.2.1-SNAPSHOT".into(), ..profile(bytes, build) };
        target.config.core_file = Some("velocity-4.2.1-SNAPSHOT-38.jar".into());
        target.config.core_profile = Some(velocity(b"first", "38"));
        save(&record_path(&fixture.0, "server"), &target.config).unwrap();
        assert!(state(&fixture.0, "server").unwrap()["reason"].as_str().is_some_and(|reason| reason != "PortChanged"),
            "a missing velocity.toml is not read as server.properties");
        fs::write(directory.join("velocity.toml"), format!("bind = \"127.0.0.1:{}\"\n", target.config.port)).unwrap();
        let candidate = fixture.0.join("velocity-4.2.1-SNAPSHOT-43.jar");
        fs::write(&candidate, b"second").unwrap();
        let (before, after) = (velocity(b"first", "38"), velocity(b"second", "43"));
        assert_eq!(state(&fixture.0, "server").unwrap()["canWrite"], true);
        install_verified(&fixture.0, "server", "proxy-update", &before.sha256, &candidate, &after).unwrap();
        assert_eq!(fs::read(directory.join("velocity-4.2.1-SNAPSHOT-38.jar")).unwrap(), b"second");
        assert!(!directory.join("velocity-4.2.1-SNAPSHOT-43.jar").exists());
        assert_eq!(state(&fixture.0, "server").unwrap()["installed"]["build"], "43");
        let args = ["server".into(), "proxy-restore".into(), after.sha256.clone(), "proxy-update".into()];
        dispatch(&fixture.0, "core-restore", &args).unwrap().unwrap();
        assert_eq!(fs::read(directory.join("velocity-4.2.1-SNAPSHOT-38.jar")).unwrap(), b"first");
        fs::write(directory.join("velocity.toml"), "bind = \"127.0.0.1:1\"\n").unwrap();
        assert_eq!(state(&fixture.0, "server").unwrap()["reason"], "PortChanged");
    }

    // #60: the dialog names each step after the download.
    #[test]
    fn install_and_restore_report_verify_backup_and_replace_in_order() {
        let fixture = Fixture::new();
        fixture.workspace();
        let candidate = fixture.0.join("candidate.jar");
        fs::write(&candidate, b"second").unwrap();
        let (before, after) = (profile(b"first", "112"), profile(b"second", "134"));
        let steps = std::cell::RefCell::new(Vec::new());
        let record = |step| steps.borrow_mut().push(step);
        install_verified_guarded(&fixture.0, "server", "update", &before.sha256, &candidate, &after, &record, || Ok(()))
            .unwrap();
        use CoreUpdatePhase::*;
        assert_eq!(steps.take(), [Verifying, BackingUp, Replacing]);
        let args = ["server".into(), "restore".into(), after.sha256.clone(), "update".into()];
        dispatch_with_progress(&fixture.0, "core-restore", &args, &record).unwrap().unwrap();
        assert_eq!(steps.take(), [Verifying, BackingUp, Replacing]);
        // A rejected candidate stops at verification and never backs up.
        fs::write(&candidate, b"tampered").unwrap();
        let third = profile(b"third", "140");
        assert!(install_verified_guarded(&fixture.0, "server", "bad", &before.sha256, &candidate, &third, &record, || Ok(()))
            .is_err());
        assert_eq!(steps.take(), [Verifying]);
    }

    #[test]
    fn reconciliation_observes_either_side_of_the_atomic_replacement() {
        for outcome in [Outcome::NotApplied, Outcome::Applied] {
            let fixture = Fixture::new();
            let target = fixture.workspace();
            let before = profile(b"first", "112");
            let after = profile(b"second", "134");
            let folder = target.store().join("interrupted");
            fs::create_dir(target.store()).unwrap();
            fs::create_dir(&folder).unwrap();
            let mut next = target.config.clone();
            next.core_profile = Some(after.clone());
            let intent = Intent {
                request: "interrupted".into(),
                action: Action::Install {
                    candidate_sha256: after.sha256.clone(),
                },
                name: "server".into(),
                directory: target.directory.clone(),
                port: target.config.port,
                registration_revision: registration_revision(&fixture.0, "server").unwrap(),
                intended_record_revision: config_revision(&next).unwrap(),
                before,
                after,
                created_at: epoch_ms(),
            };
            write_durable(&target.store().join(PENDING), &intent, false).unwrap();
            assert!(assert_start_allowed(&target.directory).is_err());
            fs::write(target.jar(), b"unknown contents").unwrap();
            assert!(reconcile(&fixture.0, "server", "interrupted")
                .unwrap_err()
                .contains("CoreMaintenanceResultUnconfirmed"));
            assert!(assert_start_allowed(&target.directory).is_err());
            fs::write(folder.join("before.jar"), b"first").unwrap();
            fs::write(
                target.jar(),
                if outcome == Outcome::Applied {
                    b"second".as_slice()
                } else {
                    b"first".as_slice()
                },
            )
            .unwrap();
            let result = reconcile(&fixture.0, "server", "interrupted").unwrap();
            assert_eq!(result["outcome"], json!(outcome));
            assert_eq!(
                reconcile(&fixture.0, "server", "interrupted").unwrap(),
                result
            );
            assert!(assert_start_allowed(&target.directory).is_ok());
        }
    }

    #[test]
    fn enforces_same_family_version_and_newer_build_for_updates() {
        let before = profile(b"first", "112");
        let mut after = profile(b"second", "134");
        let action = Action::Install {
            candidate_sha256: after.sha256.clone(),
        };
        assert!(verify_pair(&before, &after, &action).is_ok());
        after.minecraft = "26.3".into();
        assert_eq!(
            verify_pair(&before, &after, &action).unwrap_err(),
            "CoreCandidateIncompatible"
        );
        after.minecraft = before.minecraft.clone();
        after.build = "110".into();
        assert_eq!(
            verify_pair(&before, &after, &action).unwrap_err(),
            "CoreCandidateIsNotNewer"
        );
    }

    #[test]
    fn rejects_occupied_port_and_import_without_clean_release_evidence() {
        let fixture = Fixture::new();
        let mut target = fixture.workspace();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, target.config.port)).unwrap();
        assert!(target.stopped(&fixture.0).is_err());
        drop(listener);
        target.config.core_local_create = false;
        assert_eq!(
            target.stopped(&fixture.0).unwrap_err(),
            "CoreStopEvidenceMissing"
        );
    }

    #[test]
    fn rejects_remote_artifact_paths_without_opening_them() {
        let source = Path::new(r"\\remote\share\server.jar");
        assert_eq!(
            candidate_path(source).unwrap_err(),
            "CoreCandidateMustBeLocal"
        );
    }

    // #48: an unverified Folia shows the version its JAR states, but stays
    // read-only until it is verified.
    #[test]
    fn state_reports_the_offline_identity_of_an_unverified_core() {
        let fixture = Fixture::new();
        let target = fixture.workspace();
        assert!(state(&fixture.0, "server").unwrap()["identified"].is_null());
        let folia = "META-INF/libraries/dev/folia/folia-api/26.3.build.28-stable/folia-api-26.3.build.28-stable.jar";
        fs::write(target.jar(), core_identity::zip(&[folia])).unwrap();
        let state = state(&fixture.0, "server").unwrap();
        assert!(state["installed"].is_null());
        assert_eq!(state["identified"], json!({"family":"Folia","minecraft":"26.3","build":"28"}));
        assert_eq!((&state["canWrite"], &state["reason"]), (&json!(false), &json!("CoreProfileUnverified")));
    }
}
