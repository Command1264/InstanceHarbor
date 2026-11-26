//! Persist the confirmed batch before contacting an update source.
use super::*;
use std::{cell::Cell, collections::{BTreeSet, HashSet}};

const DIRECTORY: &str = "core-batches";
const MAX_TARGETS: usize = 64;
const MAX_ACTIVE_BATCHES: usize = 6;
const MAX_RECENT_BATCHES: usize = 6;
const MAX_JOURNAL_BYTES: u64 = 128 * 1024;
/// The fastest rate at which byte counts reach the progress file.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(400);

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Target {
    name: String,
    current_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    candidate_sha256: Option<String>,
    /// Purpur (#113): the official MD5 of the candidate, in place of a SHA-256
    /// that is known only after download.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    candidate_md5: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    registration_revision: Option<String>,
}

impl Target {
    fn valid(&self) -> bool {
        let candidate = match (&self.candidate_sha256, &self.candidate_md5) {
            (Some(hash), None) => digest(hash) && &self.current_sha256 != hash,
            (None, Some(md5)) => core_prefetch::md5_key(md5),
            _ => false,
        };
        instance_name(&self.name)
            && digest(&self.current_sha256)
            && candidate
            && self.registration_revision.as_deref().is_none_or(digest)
    }

    /// The candidate as the source names it: its SHA-256, or its MD5.
    fn candidate(&self) -> &str {
        self.candidate_sha256.as_deref().or(self.candidate_md5.as_deref()).unwrap_or_default()
    }
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
enum Phase {
    Queued,
    Preparing,
    Sending,
    Cancelled,
    Completed,
    Failed,
    Unconfirmed,
}

impl Phase {
    fn cancelable(self) -> bool {
        matches!(self, Self::Queued | Self::Preparing)
    }

    fn active(self) -> bool {
        self.cancelable() || self == Self::Sending
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Item {
    target: Target,
    registration: String,
    request: String,
    phase: Phase,
    reason: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Batch {
    id: String,
    created_at: u64,
    items: Vec<Item>,
}

impl Batch {
    fn active(&self) -> bool {
        self.items
            .iter()
            .any(|item| item.phase.active())
    }

    fn view(&self) -> Value {
        self.view_with(None)
    }

    /// The batch, with the reported step of the item in progress (#60).
    fn view_with(&self, progress: Option<&ItemProgress>) -> Value {
        let items: Vec<_> = self
            .items
            .iter()
            .map(|item| {
                let mut row = json!({
                    "name": item.target.name, "request": item.request, "phase": item.phase,
                    "reason": item.reason
                });
                if let Some(progress) = progress.filter(|progress| {
                    progress.request == item.request && matches!(item.phase, Phase::Preparing | Phase::Sending)
                }) {
                    row["progress"] = json!(progress.progress);
                    if progress.reused {
                        row["reused"] = json!(true);
                    }
                }
                row
            })
            .collect();
        let state = if self.active() {
            "Running"
        } else if self.items.iter().all(|item| item.phase == Phase::Completed) {
            "Completed"
        } else {
            "CompletedWithIssues"
        };
        json!({"id":self.id,"action":"core-batch-install","createdAt":self.created_at,
            "instance":"core maintenance","state":state,"items":items})
    }
}

/// The step of one batch item, kept apart from the durable journal. It is only
/// a display hint: a missing or stale file never changes what the batch does.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ItemProgress {
    request: String,
    progress: core_maintenance::CoreUpdatePhase,
    /// Stays true after `Reusing`, so the list still says the item used the
    /// downloaded core while it verifies and replaces it.
    #[serde(default)]
    reused: bool,
}

fn progress_path(directory: &Path, id: &str) -> PathBuf {
    directory.join(format!("{id}.progress"))
}

fn read_progress(directory: &Path, id: &str) -> Option<ItemProgress> {
    let path = progress_path(directory, id);
    no_redirect(&path).ok()?;
    let mut bytes = Vec::new();
    File::open(&path).ok()?.take(4096).read_to_end(&mut bytes).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_progress(root: &Path, id: &str, progress: &ItemProgress) -> Result<()> {
    let directory = journal_directory(root)?.ok_or("CoreBatchNotFound")?;
    let path = progress_path(&directory, id);
    let temporary = directory.join(format!("{id}.progress.tmp"));
    fs::write(&temporary, serde_json::to_vec(progress).map_err(fail)?).map_err(fail)?;
    fs::rename(&temporary, &path).map_err(fail)
}

fn remove_progress(root: &Path, id: &str) {
    let Ok(Some(directory)) = journal_directory(root) else { return };
    let path = progress_path(&directory, id);
    if let Err(error) = fs::remove_file(&path) {
        if error.kind() != io::ErrorKind::NotFound {
            eprintln!("Core batch progress cleanup failed: {error}");
        }
    }
}

type PhaseKind = std::mem::Discriminant<core_maintenance::CoreUpdatePhase>;

/// Publishes the steps of `work` for `core-batch-list`. A new step is written
/// at once; byte counts of the same step at most every `PROGRESS_INTERVAL`.
pub(super) fn progress_reporter<'a>(
    root: &'a Path,
    work: &'a Work,
) -> impl Fn(core_maintenance::CoreUpdatePhase) + 'a {
    let last: Cell<Option<(Instant, PhaseKind)>> = Cell::new(None);
    let reused = Cell::new(false);
    move |progress| {
        if progress == core_maintenance::CoreUpdatePhase::Reusing {
            reused.set(true);
        }
        let kind = std::mem::discriminant(&progress);
        let finished = matches!(progress,
            core_maintenance::CoreUpdatePhase::Downloading { received, total: Some(total) } if received == total);
        if last.get().is_some_and(|(written, previous)| {
            previous == kind && !finished && written.elapsed() < PROGRESS_INTERVAL
        }) {
            return;
        }
        last.set(Some((Instant::now(), kind)));
        let item = ItemProgress { request: work.item.request.clone(), progress, reused: reused.get() };
        if let Err(error) = write_progress(root, &work.batch, &item) {
            eprintln!("Core batch progress could not be written: {error}");
        }
    }
}

pub(super) struct Work {
    batch: String,
    index: usize,
    item: Item,
    recovering: bool,
}

fn request_id(batch: &str, target: &Target, registration: &str) -> Result<String> {
    content_revision::revision(
        &serde_json::to_vec(&("core-batch-v1", batch, target, registration)).map_err(fail)?,
    )
}

fn journal_directory(root: &Path) -> Result<Option<PathBuf>> {
    match fs::symlink_metadata(root.join(DIRECTORY)) {
        Ok(_) => child_directory(root, DIRECTORY).map(Some),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(fail(error)),
    }
}

fn journal_path(directory: &Path, id: &str) -> PathBuf {
    directory.join(format!("{id}.json"))
}

fn read_batch(directory: &Path, id: &str) -> Result<Batch> {
    if !token(id) {
        return Err("InvalidCoreBatchId".into());
    }
    let path = journal_path(directory, id);
    no_redirect(&path)?;
    let mut bytes = Vec::new();
    File::open(&path)
        .map_err(fail)?
        .take(MAX_JOURNAL_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(fail)?;
    if bytes.len() as u64 > MAX_JOURNAL_BYTES {
        return Err("InvalidCoreBatchJournal".into());
    }
    let batch: Batch = serde_json::from_slice(&bytes).map_err(|_| "InvalidCoreBatchJournal")?;
    let mut names = HashSet::new();
    if batch.id != id || batch.items.is_empty() || batch.items.len() > MAX_TARGETS {
        return Err("InvalidCoreBatchJournal".into());
    }
    for item in &batch.items {
        if !item.target.valid()
            || !names.insert(&item.target.name)
            || !digest(&item.registration)
            || item.target.registration_revision.as_ref().is_some_and(|expected| expected != &item.registration)
            || item.request != request_id(id, &item.target, &item.registration)?
            || item
                .reason
                .as_deref()
                .is_some_and(|reason| reason != reason_code(reason))
        {
            return Err("InvalidCoreBatchJournal".into());
        }
    }
    Ok(batch)
}

fn load_batches(root: &Path) -> Result<Vec<Batch>> {
    let Some(directory) = journal_directory(root)? else {
        return Ok(Vec::new());
    };
    let mut batches = Vec::new();
    for entry in fs::read_dir(&directory).map_err(fail)? {
        let path = entry.map_err(fail)?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let id = path
            .file_stem()
            .and_then(|id| id.to_str())
            .ok_or("InvalidCoreBatchJournal")?;
        batches.push(read_batch(&directory, id)?);
    }
    batches
        .sort_by(|left, right| (&left.created_at, &left.id).cmp(&(&right.created_at, &right.id)));
    Ok(batches)
}

fn persist(root: &Path, batch: &Batch, replace: bool) -> Result<()> {
    let directory = match journal_directory(root)? {
        Some(directory) => directory,
        None => {
            fs::create_dir(root.join(DIRECTORY)).map_err(fail)?;
            child_directory(root, DIRECTORY)?
        }
    };
    let path = journal_path(&directory, &batch.id);
    match fs::symlink_metadata(&path) {
        Ok(_) if replace => no_redirect(&path)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound && !replace => (),
        _ => return Err("CoreBatchJournalChanged".into()),
    }
    core_maintenance::write_durable(&path, batch, replace)
}

pub(super) struct Journal(Vec<Batch>);

impl Journal {
    pub(super) fn open(root: &Path) -> Result<Self> {
        load_batches(root).map(Self)
    }

    pub(super) fn active(&self) -> bool {
        self.0.iter().any(Batch::active)
    }

    pub(super) fn submit(&mut self, root: &Path, id: &str, raw: &str) -> Result<Value> {
        if !token(id) {
            return Err("InvalidCoreBatchId".into());
        }
        let targets: Vec<Target> =
            serde_json::from_str(raw).map_err(|_| "InvalidCoreBatchTargets")?;
        let mut names = HashSet::new();
        if targets.is_empty()
            || targets.len() > MAX_TARGETS
            || targets
                .iter()
                .any(|target| !target.valid() || !names.insert(&target.name))
        {
            return Err("InvalidCoreBatchTargets".into());
        }
        if let Some(prior) = self.0.iter().find(|batch| batch.id == id) {
            return if prior.items.iter().map(|item| &item.target).eq(&targets) {
                Ok(prior.view())
            } else {
                Err("RequestConflict".into())
            };
        }
        if self.0.iter().filter(|batch| batch.active()).count() >= MAX_ACTIVE_BATCHES
            || self.0.iter().flat_map(|batch| &batch.items).any(|item| {
                item.phase.active() && names.contains(&item.target.name)
            })
        {
            return Err("CoreBatchTargetPending".into());
        }
        let mut items = Vec::with_capacity(targets.len());
        for target in targets {
            if !admitted(root, &target.name)? || instance(root, &target.name)?.removed {
                return Err("CoreTargetUnavailable".into());
            }
            let registration = registration_revision(root, &target.name)?;
            if target.registration_revision.as_ref().is_some_and(|expected| expected != &registration) {
                return Err("RegistrationChanged".into());
            }
            let request = request_id(id, &target, &registration)?;
            items.push(Item {
                target,
                registration,
                request,
                phase: Phase::Queued,
                reason: None,
            });
        }
        let batch = Batch {
            id: id.into(),
            created_at: epoch_ms(),
            items,
        };
        persist(root, &batch, false)?;
        let result = batch.view();
        self.0.push(batch);
        Ok(result)
    }

    pub(super) fn cancel(&mut self, root: &Path, id: &str) -> Result<Value> {
        let batch = self.0.iter_mut().find(|batch| batch.id == id).ok_or("CoreBatchNotFound")?;
        if !batch.items.iter().any(|item| item.phase.cancelable()) {
            return Ok(batch.view());
        }
        let mut changed = batch.clone();
        for item in &mut changed.items {
            if item.phase.cancelable() {
                item.phase = Phase::Cancelled;
            }
        }
        persist(root, &changed, true)?;
        *batch = changed;
        Ok(batch.view())
    }

    pub(super) fn next(&mut self, root: &Path) -> Result<Option<Work>> {
        // Preparing may survive a Manager crash, but that phase cannot have
        // opened an install transaction. Only Sending needs receipt reconciliation.
        for phase in [Phase::Sending, Phase::Preparing, Phase::Queued] {
            for batch in &mut self.0 {
                let Some(index) = batch.items.iter().position(|item| item.phase == phase) else {
                    continue;
                };
                let recovering = phase == Phase::Sending;
                if phase == Phase::Queued {
                    let mut changed = batch.clone();
                    changed.items[index].phase = Phase::Preparing;
                    persist(root, &changed, true)?;
                    *batch = changed;
                }
                return Ok(Some(Work {
                    batch: batch.id.clone(),
                    index,
                    item: batch.items[index].clone(),
                    recovering,
                }));
            }
        }
        Ok(None)
    }

    fn work_batch(&mut self, work: &Work) -> Result<&mut Batch> {
        let batch = self.0.iter_mut().find(|batch| batch.id == work.batch)
            .ok_or("CoreBatchChanged")?;
        if batch.items.get(work.index).is_none_or(|item| item.request != work.item.request) {
            return Err("CoreBatchChanged".into());
        }
        Ok(batch)
    }

    pub(super) fn begin_commit(&mut self, root: &Path, work: &Work) -> Result<()> {
        let batch = self.work_batch(work)?;
        match batch.items[work.index].phase {
            Phase::Cancelled => return Err("CoreBatchCancelled".into()),
            Phase::Preparing => (),
            _ => return Err("CoreBatchChanged".into()),
        }
        let mut changed = batch.clone();
        changed.items[work.index].phase = Phase::Sending;
        persist(root, &changed, true)?;
        *batch = changed;
        Ok(())
    }

    pub(super) fn finish(&mut self, root: &Path, work: &Work, outcome: Outcome) -> Result<()> {
        let batch = self.work_batch(work)?;
        match batch.items[work.index].phase {
            Phase::Cancelled => return Ok(()),
            Phase::Sending => (),
            Phase::Preparing if matches!(outcome, Outcome::Failed(_) | Outcome::Unconfirmed(_)) => (),
            _ => return Err("CoreBatchChanged".into()),
        }
        let mut changed = batch.clone();
        let entry = &mut changed.items[work.index];
        (entry.phase, entry.reason) = match outcome {
            Outcome::Completed => (Phase::Completed, None),
            Outcome::Failed(error) => (Phase::Failed, Some(error)),
            Outcome::Unconfirmed(error) => (Phase::Unconfirmed, Some(error)),
        };
        persist(root, &changed, true)?;
        let done = !changed.active();
        *batch = changed;
        if done {
            remove_progress(root, &work.batch);
        }
        Ok(())
    }

    /// Removes batch candidates that no active item still needs (#60).
    pub(super) fn collect_candidates(&self, root: &Path) -> Result<()> {
        let needed: BTreeSet<&str> = self.0.iter().flat_map(|batch| &batch.items)
            .filter(|item| item.phase.active())
            .map(|item| item.target.candidate())
            .collect();
        core_prefetch::collect_batch(root, &needed)
    }
}

pub(super) enum Outcome {
    Completed,
    Failed(String),
    Unconfirmed(String),
}

fn reason_code(error: &str) -> &str {
    let code = error.split(':').next().unwrap_or(error);
    if code.len() <= 60 && !code.is_empty() && code.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        code
    } else {
        "CoreBatchUpdateFailed"
    }
}

fn reconcile(root: &Path, item: &Item) -> Result<Value> {
    dispatch(&[
        root.to_string_lossy().into_owned(),
        "core-reconcile".into(),
        item.target.name.clone(),
        item.request.clone(),
    ])
}

fn classify_receipt(item: &Item, receipt: Result<Value>) -> Outcome {
    match receipt {
        Ok(receipt) if receipt["workId"] == item.request => {
            let installed = &receipt["installed"]["sha256"];
            // A Purpur candidate has no SHA-256 before download (#113). This
            // request installs only bytes that matched its official MD5, so a
            // Purpur receipt of this request that changed the core completed it.
            let candidate = match &item.target.candidate_sha256 {
                Some(hash) => installed == hash,
                None => receipt["installed"]["family"] == "PURPUR" && installed != &item.target.current_sha256,
            };
            match receipt["outcome"].as_str() {
                Some("applied") if candidate => Outcome::Completed,
                Some("notApplied") if installed == &item.target.current_sha256 => {
                    Outcome::Failed("CoreBatchNotApplied".into())
                }
                _ => Outcome::Unconfirmed("CoreBatchReceiptMismatch".into()),
            }
        }
        Ok(_) => Outcome::Unconfirmed("CoreBatchReceiptMismatch".into()),
        Err(error) => Outcome::Unconfirmed(reason_code(&error).into()),
    }
}

fn after_install_error(root: &Path, item: &Item, error: &str) -> Outcome {
    match core_maintenance::activity(root, &item.target.name) {
        Ok(rows) if rows.iter().any(|row| row["id"] == item.request) => {
            classify_receipt(item, reconcile(root, item))
        }
        Ok(_) => Outcome::Failed(reason_code(error).into()),
        Err(error) => Outcome::Unconfirmed(reason_code(&error).into()),
    }
}

pub(super) fn perform(
    root: &Path,
    work: &Work,
    progress: &dyn Fn(core_maintenance::CoreUpdatePhase),
    begin_commit: impl FnOnce() -> Result<()>,
) -> Outcome {
    let item = &work.item;
    if work.recovering {
        // A previous Manager may already have replaced the artifact. Only the durable
        // transaction can tell, and no recovered item is sent to the source again.
        return classify_receipt(item, reconcile(root, item));
    }
    let result = core_source::install_latest_guarded(
        root,
        &item.target.name,
        &item.request,
        &item.target.current_sha256,
        item.target.candidate(),
        &item.registration,
        core_source::CandidateCache::Batch,
        progress,
        begin_commit,
    );
    match result {
        Err(error) => after_install_error(root, item, &error),
        receipt => classify_receipt(item, receipt),
    }
}

fn live_view(directory: &Path, batch: &Batch) -> Value {
    let progress = batch.active().then(|| read_progress(directory, &batch.id)).flatten();
    batch.view_with(progress.as_ref())
}

pub(super) fn client(root: &Path, verb: &str, args: &[String]) -> Result<Value> {
    match (verb, args) {
        ("core-batch-submit", [id, _]) if token(id) => resident_manager::client(root, verb, args),
        ("core-batch-cancel", [id]) if token(id) => resident_manager::client(root, verb, args),
        ("core-batch-status", [id]) if token(id) => {
            let directory = journal_directory(root)?.ok_or("CoreBatchNotFound")?;
            if !journal_path(&directory, id).exists() {
                return Err("CoreBatchNotFound".into());
            }
            read_batch(&directory, id).map(|batch| live_view(&directory, &batch))
        }
        ("core-batch-list", []) => {
            let batches = load_batches(root)?;
            let directory = journal_directory(root)?;
            let recent: Vec<_> = batches
                .iter()
                .rev()
                .filter(|batch| batch.active())
                .chain(batches.iter().rev().filter(|batch| !batch.active()))
                .take(MAX_RECENT_BATCHES)
                .map(|batch| match &directory {
                    Some(directory) => live_view(directory, batch),
                    None => batch.view(),
                })
                .collect();
            Ok(json!(recent))
        }
        _ => Err("InvalidCoreBatchArguments".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture(PathBuf);
    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().canonicalize().unwrap().join(format!(
                "ih-core-batch-test-{}-{}-{}",
                std::process::id(),
                clock(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
            ));
            dispatch(&[path.to_string_lossy().into_owned(), "init".into()]).unwrap();
            Self(path)
        }

        fn add(&self, name: &str) {
            self.add_with(name, profile(b"current artifact", "100"));
        }

        fn add_with(&self, name: &str, core: CoreProfile) {
            let directory = child_directory(&self.0, "instances").unwrap().join(name);
            fs::create_dir(&directory).unwrap();
            fs::write(directory.join("server.jar"), b"current artifact").unwrap();
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let port = listener.local_addr().unwrap().port();
            fs::write(
                directory.join("server.properties"),
                format!("server-ip=127.0.0.1\nserver-port={port}\n"),
            )
            .unwrap();
            save(
                &record_path(&self.0, name),
                &Instance {
                    core_file: None,
                    name: name.into(),
                    java: directory.join("unused-java.exe"),
                    port,
                    velocity_listen_address: None,
                    removed: false,
                    directory: None,
                    jvm: None,
                    timing: None,
                    auto_restart: None,
                    core_profile: Some(core),
                    core_local_create: true,
                    management_discovered: false,
                },
            )
            .unwrap();
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
            minecraft: "26.3".into(),
            build: build.into(),
            sha256: content_revision::revision(bytes).unwrap(),
        }
    }

    fn target(name: &str) -> Target {
        Target {
            name: name.into(),
            current_sha256: profile(b"current artifact", "100").sha256,
            candidate_sha256: Some(profile(b"candidate artifact", "101").sha256),
            candidate_md5: None,
            registration_revision: None,
        }
    }

    fn arguments(targets: &[Target]) -> String {
        serde_json::to_string(targets).unwrap()
    }

    #[test]
    fn resubmission_is_idempotent_and_concurrent_target_is_rejected() {
        let fixture = Fixture::new();
        fixture.add("first");
        fixture.add("second");
        let mut journal = Journal::open(&fixture.0).unwrap();
        let raw = arguments(&[target("first"), target("second")]);
        let first = journal.submit(&fixture.0, "batch-one", &raw).unwrap();
        assert_eq!(first["items"][0]["phase"], "Queued");
        assert_eq!(
            journal.submit(&fixture.0, "batch-one", &raw).unwrap(),
            first
        );
        let partial = arguments(&[target("first")]);
        assert_eq!(
            journal
                .submit(&fixture.0, "batch-one", &partial)
                .unwrap_err(),
            "RequestConflict"
        );
        assert_eq!(
            journal
                .submit(&fixture.0, "batch-two", &partial)
                .unwrap_err(),
            "CoreBatchTargetPending"
        );
        let duplicate = arguments(&[target("second"), target("second")]);
        assert_eq!(
            journal
                .submit(&fixture.0, "duplicate", &duplicate)
                .unwrap_err(),
            "InvalidCoreBatchTargets"
        );
        assert_eq!(
            client(&fixture.0, "core-batch-list", &[]).unwrap(),
            json!([first])
        );
    }

    #[test]
    fn restart_reconciles_only_the_original_request_and_keeps_untouched_items_queued() {
        let fixture = Fixture::new();
        fixture.add("first");
        fixture.add("second");
        let mut current = Journal::open(&fixture.0).unwrap();
        current
            .submit(
                &fixture.0,
                "batch-recovery",
                &arguments(&[target("first"), target("second")]),
            )
            .unwrap();
        let sent = current.next(&fixture.0).unwrap().unwrap();
        assert!(!sent.recovering);
        current.begin_commit(&fixture.0, &sent).unwrap();
        let request = client(&fixture.0, "core-batch-status", &["batch-recovery".into()]).unwrap();
        assert_eq!(request["items"][0]["phase"], "Sending");
        assert_eq!(request["items"][1]["phase"], "Queued");
        assert!(matches!(
            after_install_error(&fixture.0, &sent.item, "CoreArtifactChanged"),
            Outcome::Failed(_)
        ));
        let broken = fixture
            .0
            .join("instances/first/.instanceharbor-core-maintenance");
        fs::create_dir(&broken).unwrap();
        fs::write(broken.join("pending.json"), b"malformed").unwrap();
        assert!(matches!(
            after_install_error(&fixture.0, &sent.item, "CoreArtifactChanged"),
            Outcome::Unconfirmed(_)
        ));
        fs::remove_file(broken.join("pending.json")).unwrap();
        let mut restarted = Journal::open(&fixture.0).unwrap();
        let received = restarted.next(&fixture.0).unwrap().unwrap();
        assert!(received.recovering);
        assert_eq!(received.item.request, sent.item.request);
        let outcome = perform(&fixture.0, &received, &core_maintenance::no_progress, || panic!("recovery cannot enter commit"));
        assert!(matches!(outcome, Outcome::Unconfirmed(_)));
        restarted.finish(&fixture.0, &received, outcome).unwrap();
        let next = restarted.next(&fixture.0).unwrap().unwrap();
        assert_eq!(next.item.target.name, "second");
        assert!(!next.recovering);
        assert_eq!(
            fs::read(fixture.0.join("instances/first/server.jar")).unwrap(),
            b"current artifact"
        );
    }

    #[test]
    fn cancel_prepared_candidate_never_creates_install_intent_and_survives_restart() {
        let fixture = Fixture::new();
        for name in ["first", "second", "not-selected"] {
            fixture.add(name);
        }
        let mut journal = Journal::open(&fixture.0).unwrap();
        let raw = arguments(&[target("first"), target("second")]);
        let submitted = journal.submit(&fixture.0, "batch-cancel", &raw).unwrap();
        let prepared = journal.next(&fixture.0).unwrap().unwrap();
        let mut journal = Journal::open(&fixture.0).unwrap();
        let resumed = journal.next(&fixture.0).unwrap().unwrap();
        assert!(!resumed.recovering);
        assert_eq!(prepared.item.request, resumed.item.request);
        let candidate = fixture.0.join("candidate.jar");
        fs::write(&candidate, b"candidate artifact").unwrap();
        let error = core_maintenance::install_verified_guarded(
            &fixture.0, "first", &resumed.item.request, &resumed.item.target.current_sha256,
            &candidate, &profile(b"candidate artifact", "101"), &core_maintenance::no_progress, || {
                let canceled = journal.cancel(&fixture.0, "batch-cancel").unwrap();
                assert_eq!(canceled["items"][0]["phase"], "Cancelled");
                assert_eq!(canceled["items"][1]["phase"], "Cancelled");
                journal.begin_commit(&fixture.0, &resumed)
            },
        ).unwrap_err();
        assert_eq!(error, "CoreBatchCancelled");
        journal.finish(&fixture.0, &resumed, Outcome::Failed(error)).unwrap();
        let mut journal = Journal::open(&fixture.0).unwrap();
        let canceled = journal.cancel(&fixture.0, "batch-cancel").unwrap();
        assert_eq!(canceled, journal.submit(&fixture.0, "batch-cancel", &raw).unwrap());
        assert_eq!(canceled["items"][0]["request"], submitted["items"][0]["request"]);
        assert!(journal.next(&fixture.0).unwrap().is_none());
        for name in ["first", "second", "not-selected"] {
            let directory = fixture.0.join("instances").join(name);
            assert_eq!(fs::read(directory.join("server.jar")).unwrap(), b"current artifact");
            assert!(!directory.join(".instanceharbor-core-maintenance").exists());
        }
    }

    #[test]
    fn cancel_leaves_committing_work_and_its_backup_uninterrupted() {
        let fixture = Fixture::new();
        fixture.add("first");
        fixture.add("second");
        let mut journal = Journal::open(&fixture.0).unwrap();
        let mut first = target("first");
        first.registration_revision = Some("a".repeat(64));
        assert_eq!(journal.submit(&fixture.0, "stale", &arguments(&[first])).unwrap_err(),
            "RegistrationChanged");
        journal.submit(&fixture.0, "batch-boundary", &arguments(&[target("first"), target("second")])).unwrap();
        let work = journal.next(&fixture.0).unwrap().unwrap();
        let candidate = fixture.0.join("candidate.jar");
        fs::write(&candidate, b"candidate artifact").unwrap();
        let result = core_maintenance::install_verified_guarded(
            &fixture.0, "first", &work.item.request, &work.item.target.current_sha256,
            &candidate, &profile(b"candidate artifact", "101"), &core_maintenance::no_progress, || {
                journal.begin_commit(&fixture.0, &work)?;
                let canceled = journal.cancel(&fixture.0, "batch-boundary")?;
                assert_eq!(canceled["items"][0]["phase"], "Sending");
                assert_eq!(canceled["items"][1]["phase"], "Cancelled");
                Ok(())
            },
        );
        let mut journal = Journal::open(&fixture.0).unwrap();
        let recovered = journal.next(&fixture.0).unwrap().unwrap();
        assert!(recovered.recovering);
        assert_eq!(recovered.item.request, work.item.request);
        assert!(matches!(classify_receipt(&work.item, result), Outcome::Completed));
        let outcome = perform(&fixture.0, &recovered, &core_maintenance::no_progress, || panic!("cannot commit again"));
        journal.finish(&fixture.0, &recovered, outcome).unwrap();
        let result = journal.cancel(&fixture.0, "batch-boundary").unwrap();
        assert_eq!(result["items"][0]["phase"], "Completed");
        assert_eq!(result["items"][1]["phase"], "Cancelled");
        let first = fixture.0.join("instances/first");
        assert_eq!(fs::read(first.join("server.jar")).unwrap(), b"candidate artifact");
        assert_eq!(fs::read(first.join(".instanceharbor-core-maintenance").join(&work.item.request).join("before.jar")).unwrap(), b"current artifact");
        assert_eq!(fs::read(fixture.0.join("instances/second/server.jar")).unwrap(), b"current artifact");
        assert!(journal.next(&fixture.0).unwrap().is_none());
    }

    #[test]
    fn restart_uses_completed_transaction_even_when_registration_revision_changed() {
        let fixture = Fixture::new();
        fixture.add("only");
        let mut journal = Journal::open(&fixture.0).unwrap();
        journal
            .submit(&fixture.0, "batch-receipt", &arguments(&[target("only")]))
            .unwrap();
        let sent = journal.next(&fixture.0).unwrap().unwrap();
        journal.begin_commit(&fixture.0, &sent).unwrap();
        let jar = fixture.0.join("candidate.jar");
        fs::write(&jar, b"candidate artifact").unwrap();
        let result = core_maintenance::install_verified(
            &fixture.0,
            "only",
            &sent.item.request,
            &sent.item.target.current_sha256,
            &jar,
            &profile(b"candidate artifact", "101"),
        )
        .unwrap();
        assert_eq!(result["outcome"], "applied");
        assert_ne!(
            registration_revision(&fixture.0, "only").unwrap(),
            sent.item.registration
        );
        assert!(matches!(
            after_install_error(&fixture.0, &sent.item, "ReadbackFailed"),
            Outcome::Completed
        ));
        let mut restarted = Journal::open(&fixture.0).unwrap();
        let recovered = restarted.next(&fixture.0).unwrap().unwrap();
        let outcome = perform(&fixture.0, &recovered, &core_maintenance::no_progress, || panic!("recovery cannot enter commit"));
        assert!(matches!(outcome, Outcome::Completed));
        restarted.finish(&fixture.0, &recovered, outcome).unwrap();
        let result = client(&fixture.0, "core-batch-status", &["batch-receipt".into()]).unwrap();
        assert_eq!(result["state"], "Completed");
        assert_eq!(result["items"][0]["request"], sent.item.request);
        assert!(restarted.next(&fixture.0).unwrap().is_none());
    }

    // #60: the list shows the step of the item in progress, only while it runs.
    #[test]
    fn list_shows_the_reported_step_of_the_running_item_only() {
        use core_maintenance::CoreUpdatePhase::*;
        let fixture = Fixture::new();
        fixture.add("first");
        fixture.add("second");
        let mut journal = Journal::open(&fixture.0).unwrap();
        journal.submit(&fixture.0, "batch-progress", &arguments(&[target("first"), target("second")])).unwrap();
        let work = journal.next(&fixture.0).unwrap().unwrap();
        let report = progress_reporter(&fixture.0, &work);
        let list = || client(&fixture.0, "core-batch-list", &[]).unwrap()[0].clone();
        report(Downloading { received: 0, total: Some(300) });
        assert_eq!(list()["items"][0]["progress"], json!({"phase":"downloading","received":0,"total":300}));
        // Byte counts of the same step are throttled; a finished download and a new step are not.
        report(Downloading { received: 100, total: Some(300) });
        assert_eq!(list()["items"][0]["progress"]["received"], 0);
        report(Downloading { received: 300, total: Some(300) });
        assert_eq!(list()["items"][0]["progress"]["received"], 300);
        report(Verifying);
        assert_eq!(list()["items"][0]["progress"], json!({"phase":"verifying"}));
        assert!(list()["items"][1].get("progress").is_none());
        let status = client(&fixture.0, "core-batch-status", &["batch-progress".into()]).unwrap();
        assert_eq!(status["items"][0]["progress"], json!({"phase":"verifying"}));
        // A download without an official size reports bytes and no total.
        report(Downloading { received: 5, total: None });
        assert_eq!(list()["items"][0]["progress"], json!({"phase":"downloading","received":5,"total":null}));
        assert!(list()["items"][0].get("reused").is_none());
        // After reusing a downloaded candidate, later steps still say so.
        report(Reusing);
        report(Replacing);
        assert_eq!((&list()["items"][0]["progress"], &list()["items"][0]["reused"]), (&json!({"phase":"replacing"}), &json!(true)));
        journal.finish(&fixture.0, &work, Outcome::Failed("CoreDownloadUnverified".into())).unwrap();
        assert!(list()["items"][0].get("progress").is_none());
        let next = journal.next(&fixture.0).unwrap().unwrap();
        assert!(list()["items"][1].get("progress").is_none(), "an old step never moves to the next item");
        journal.finish(&fixture.0, &next, Outcome::Failed("CoreDownloadUnverified".into())).unwrap();
        assert!(!fixture.0.join("core-batches/batch-progress.progress").exists());
    }

    // #60: one candidate per SHA-256 serves every item of the batch, and is
    // removed once no active item needs it.
    #[test]
    fn batch_candidates_stay_while_an_active_item_needs_them() {
        let fixture = Fixture::new();
        fixture.add("first");
        fixture.add("second");
        let mut journal = Journal::open(&fixture.0).unwrap();
        journal.submit(&fixture.0, "batch-shared", &arguments(&[target("first"), target("second")])).unwrap();
        let candidate = target("first").candidate_sha256.unwrap();
        let store = core_prefetch::Store::batch(&fixture.0).unwrap();
        let downloads = std::cell::Cell::new(0);
        for _ in 0..2 {
            let lease = store.lease(&candidate).unwrap();
            lease.ensure(&candidate, |path| {
                downloads.set(downloads.get() + 1);
                fs::write(path, b"candidate artifact").map_err(fail)
            }).unwrap();
        }
        assert_eq!(downloads.get(), 1, "the second item reuses the downloaded candidate");
        let directory = fixture.0.join("staging/core-batch-candidates");
        let unused = directory.join(format!("{}.jar", "c".repeat(64)));
        fs::write(&unused, b"left by an earlier batch").unwrap();
        journal.collect_candidates(&fixture.0).unwrap();
        assert!(directory.join(format!("{candidate}.jar")).exists());
        assert!(!unused.exists());
        journal.cancel(&fixture.0, "batch-shared").unwrap();
        journal.collect_candidates(&fixture.0).unwrap();
        assert!(!directory.join(format!("{candidate}.jar")).exists());
    }

    fn purpur(bytes: &[u8], build: &str) -> CoreProfile {
        CoreProfile { family: "PURPUR".into(), ..profile(bytes, build) }
    }

    fn md5_of(fixture: &Fixture, bytes: &[u8]) -> String {
        let probe = fixture.0.join("md5-probe.jar");
        fs::write(&probe, bytes).unwrap();
        let md5 = content_revision::file_md5(&probe).unwrap();
        fs::remove_file(&probe).unwrap();
        md5
    }

    // #113: a Purpur item enters a batch by the official MD5 of its candidate.
    // Only the first download is compared with the MD5; its SHA-256 is recorded
    // and later items use the stored bytes. A download that does not match the
    // MD5 leaves that instance unchanged, and the other items still complete.
    #[test]
    fn purpur_batch_checks_the_md5_once_and_a_mismatch_fails_only_its_item() {
        let fixture = Fixture::new();
        for name in ["tampered", "first", "second"] {
            fixture.add_with(name, purpur(b"current artifact", "100"));
        }
        let md5 = md5_of(&fixture, b"candidate artifact");
        let purpur_target = |name: &str| Target {
            name: name.into(),
            current_sha256: purpur(b"current artifact", "100").sha256,
            candidate_sha256: None,
            candidate_md5: Some(md5.clone()),
            registration_revision: None,
        };
        // A target names its candidate by exactly one lowercase digest.
        assert!(purpur_target("a").valid());
        assert!(!Target { candidate_md5: Some(md5.to_uppercase()), ..purpur_target("a") }.valid());
        assert!(!Target { candidate_md5: None, ..purpur_target("a") }.valid());
        assert!(!Target { candidate_sha256: target("a").candidate_sha256, ..purpur_target("a") }.valid());
        // A SHA-256 target is written as before, so its request ID is unchanged.
        assert_eq!(serde_json::to_value(target("a")).unwrap().as_object().unwrap().keys().collect::<Vec<_>>(),
            ["candidateSha256", "currentSha256", "name"]);

        let mut journal = Journal::open(&fixture.0).unwrap();
        let targets = arguments(&[purpur_target("tampered"), purpur_target("first"), purpur_target("second")]);
        journal.submit(&fixture.0, "batch-purpur", &targets).unwrap();
        let store = core_prefetch::Store::batch(&fixture.0).unwrap();
        let downloads = std::cell::Cell::new(0);
        let reused = std::cell::Cell::new(0);
        for (name, served) in [("tampered", &b"tampered artifact"[..]), ("first", b"candidate artifact"), ("second", b"never served")] {
            let work = journal.next(&fixture.0).unwrap().unwrap();
            assert_eq!(work.item.target.name, name);
            let after = CoreProfile { sha256: String::new(), ..purpur(b"", "101") };
            let result = core_source::install_purpur_stored(
                &fixture.0, name, &work.item.request, &work.item.target.current_sha256, &work.item.registration,
                &store, &md5, &purpur(b"current artifact", "100"), &after,
                |path| {
                    downloads.set(downloads.get() + 1);
                    fs::write(path, served).map_err(fail)
                },
                &|phase| if phase == core_maintenance::CoreUpdatePhase::Reusing { reused.set(reused.get() + 1) },
                || journal.begin_commit(&fixture.0, &work),
            );
            let outcome = match result {
                Err(error) => after_install_error(&fixture.0, &work.item, &error),
                receipt => classify_receipt(&work.item, receipt),
            };
            journal.finish(&fixture.0, &work, outcome).unwrap();
        }
        assert_eq!((downloads.get(), reused.get()), (2, 1), "the second matching item reuses the stored bytes");
        let status = client(&fixture.0, "core-batch-status", &["batch-purpur".into()]).unwrap();
        let phases: Vec<_> = status["items"].as_array().unwrap().iter()
            .map(|item| (item["phase"].as_str().unwrap(), item["reason"].as_str())).collect();
        assert_eq!(phases, [("Failed", Some("CoreArtifactDigestMismatch")), ("Completed", None), ("Completed", None)]);
        let candidate = content_revision::revision(b"candidate artifact").unwrap();
        assert_eq!(fs::read(fixture.0.join("instances/tampered/server.jar")).unwrap(), b"current artifact");
        assert!(!fixture.0.join("instances/tampered/.instanceharbor-core-maintenance").exists());
        for name in ["first", "second"] {
            assert_eq!(fs::read(fixture.0.join("instances").join(name).join("server.jar")).unwrap(), b"candidate artifact");
            let registered = instance(&fixture.0, name).unwrap().core_profile.unwrap();
            assert_eq!((registered.family.as_str(), registered.build.as_str(), registered.sha256.as_str(), registered.unverified),
                ("PURPUR", "101", candidate.as_str(), false));
        }
        // The batch is done, so its stored bytes and their record are collected.
        let directory = fixture.0.join("staging/core-batch-candidates");
        assert!(directory.join(format!("{candidate}.jar")).exists());
        journal.collect_candidates(&fixture.0).unwrap();
        assert!(!directory.join(format!("{candidate}.jar")).exists());
        assert!(!directory.join(format!("{md5}.sha256")).exists());
    }

    // #113: a recovered Purpur item is judged by its own receipt.
    #[test]
    fn purpur_receipt_completes_only_a_changed_purpur_core() {
        let item = Item {
            target: Target { name: "a".into(), current_sha256: "1".repeat(64), candidate_sha256: None,
                candidate_md5: Some("e".repeat(32)), registration_revision: None },
            registration: "2".repeat(64), request: "req".into(), phase: Phase::Sending, reason: None,
        };
        let receipt = |outcome: &str, family: &str, sha: String| Ok(json!({"workId":"req","outcome":outcome,
            "installed":{"family":family,"sha256":sha}}));
        assert!(matches!(classify_receipt(&item, receipt("applied", "PURPUR", "3".repeat(64))), Outcome::Completed));
        assert!(matches!(classify_receipt(&item, receipt("applied", "PAPER", "3".repeat(64))), Outcome::Unconfirmed(_)));
        assert!(matches!(classify_receipt(&item, receipt("applied", "PURPUR", "1".repeat(64))), Outcome::Unconfirmed(_)));
        assert!(matches!(classify_receipt(&item, receipt("notApplied", "PURPUR", "1".repeat(64))), Outcome::Failed(_)));
    }
}
