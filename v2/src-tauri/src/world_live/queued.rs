//! Durable world-rule requests and readback through a verified Host.
use super::*;
use std::collections::BTreeMap;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Edit {
    request: String,
    target: String,
    sha256: String,
    queued_after: Option<String>,
    attempted_run: Option<String>,
    state: String,
    created_at: u64,
    #[serde(default)]
    readback_attempts: u8,
    #[serde(default)]
    persistence_checked: bool,
}

type Edits = BTreeMap<String, Edit>;
type Targets = BTreeMap<String, String>;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Batch {
    request: String,
    targets: Targets,
    #[serde(default)]
    live_first: bool,
    #[serde(default)]
    states: BTreeMap<String, String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Edit,
    Queue,
    LiveFirst,
}

const READBACK_ATTEMPTS: u8 = 3;

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Queue {
    edits: Edits,
    last_batch: Option<Batch>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum QueueFile {
    Current(Queue),
    Legacy(Edits),
}

fn entry(root: &Path, name: &str) -> Result<PathBuf> {
    if !token(name) {
        return Err("InvalidWorldRequest".into());
    }
    Ok(child_directory(root, "history")?.join(format!("queued-world-{name}.json")))
}

fn read_bounded<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(fail(error)),
        Ok(_) => no_redirect(path)?,
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(fail)?
        .take(16385)
        .read_to_end(&mut bytes)
        .map_err(fail)?;
    if bytes.len() > 16384 {
        return Err("InvalidWorldJournal".into());
    }
    serde_json::from_slice(&bytes).map(Some).map_err(fail)
}

fn load(path: &Path) -> Result<Queue> {
    let queue = match read_bounded(path)? {
        Some(QueueFile::Current(queue)) => queue,
        Some(QueueFile::Legacy(edits)) => Queue {
            edits,
            last_batch: None,
        },
        None => Queue::default(),
    };
    if queue.edits.len() > RULE_KEYS.len() + 1 {
        return Err("InvalidWorldJournal".into());
    }
    Ok(queue)
}

pub(in crate::windows) fn has_unsettled_rebuild_work(root: &Path, name: &str) -> Result<bool> {
    Ok(load(&entry(root, name)?)?.edits.values().any(|edit| {
        unresolved(edit) || (edit.state == "AwaitingNormalStop" && !edit.persistence_checked)
    }))
}

fn lock(root: &Path, name: &str) -> Result<File> {
    command_lock(root, &format!("queued-world-{name}.lock"), false)
}

pub(super) fn verified(root: &Path, config: &Instance) -> Result<(PathBuf, CoreProfile)> {
    agent_rebuild::assert_idle(root, &config.name)?;
    if config.removed || !admitted(root, &config.name)? {
        return Err("WorldRunUnavailable".into());
    }
    let directory = instance_path(root, &config.name)?;
    let jar = directory.join("server.jar");
    no_redirect(&jar)?;
    let hash = content_revision::file_revision(&jar)?;
    if config
        .core_profile
        .as_ref()
        .is_some_and(|saved| saved.sha256 != hash)
    {
        return Err("CoreIdentityChanged".into());
    }
    let core = core_maintenance::trusted_profile(config, hash).ok_or("WorldProfileUnverified")?;
    if !matches!(core.family.as_str(), "PAPER" | "FOLIA" | "SPIGOT") {
        return Err("WorldProfileUnverified".into());
    }
    Ok((directory, core))
}

pub(super) fn submit_edit(
    root: &Path,
    name: &str,
    request: &str,
    key: &str,
    value: &str,
    confirmation: &str,
) -> Result<Value> {
    if !token(name) || !token(request) || confirmation != if key == TIME { "time" } else { "rules" }
    {
        return Err("InvalidWorldRequest".into());
    }
    submit(
        root,
        name,
        request,
        Targets::from([(key.into(), value.into())]),
        Mode::Edit,
    )
}

pub(super) fn submit_rules(root: &Path, name: &str, request: &str, raw: &str) -> Result<Value> {
    submit_rule_batch(root, name, request, raw, Mode::Queue)
}

pub(super) fn save_rules(root: &Path, name: &str, request: &str, raw: &str) -> Result<Value> {
    submit_rule_batch(root, name, request, raw, Mode::LiveFirst)
}

fn submit_rule_batch(
    root: &Path,
    name: &str,
    request: &str,
    raw: &str,
    mode: Mode,
) -> Result<Value> {
    if !token(name) || !token(request) || raw.len() > 4096 {
        return Err("InvalidWorldRequest".into());
    }
    let targets: Targets = serde_json::from_str(raw).map_err(|_| "InvalidWorldRequest")?;
    if targets.is_empty() || targets.len() > RULE_KEYS.len() || targets.contains_key(TIME) {
        return Err("InvalidWorldRequest".into());
    }
    submit(root, name, request, targets, mode)
}

fn batch_directory(root: &Path, name: &str) -> Result<PathBuf> {
    Ok(child_directory(root, "history")?.join(format!("queued-world-batches-{name}")))
}

fn read_batch(root: &Path, name: &str, request: &str) -> Result<Option<Batch>> {
    let directory = batch_directory(root, name)?;
    match fs::symlink_metadata(&directory) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(fail(error)),
        Ok(_) => no_redirect(&directory)?,
    }
    let found: Option<Batch> = read_bounded(&directory.join(format!("{request}.json")))?;
    if found.as_ref().is_some_and(|batch| batch.request != request) {
        return Err("InvalidWorldJournal".into());
    }
    Ok(found)
}

fn archive_last_batch(root: &Path, name: &str, batch: &Batch) -> Result<()> {
    if !token(&batch.request) {
        return Err("InvalidWorldJournal".into());
    }
    let existing = read_batch(root, name, &batch.request)?;
    if let Some(existing) = &existing {
        if existing.targets != batch.targets || existing.live_first != batch.live_first {
            return Err("InvalidWorldJournal".into());
        }
        if existing.states == batch.states {
            return Ok(());
        }
    }
    let directory = batch_directory(root, name)?;
    if !directory.exists() {
        fs::create_dir(&directory).map_err(fail)?;
    }
    no_redirect(&directory)?;
    core_maintenance::write_durable(
        &directory.join(format!("{}.json", batch.request)),
        batch,
        existing.is_some(),
    )
}

fn batch_states(batch: &Batch, edits: &Edits) -> BTreeMap<String, String> {
    batch
        .targets
        .keys()
        .map(|key| {
            let state = edits
                .get(key)
                .filter(|edit| edit.request == batch.request)
                .map(|edit| {
                    if edit.state == "Applying" {
                        "Unconfirmed"
                    } else {
                        edit.state.as_str()
                    }
                })
                .or_else(|| batch.states.get(key).map(String::as_str))
                .unwrap_or("Queued");
            (key.clone(), state.into())
        })
        .collect()
}

fn batch_receipt(name: &str, batch: &Batch, edits: &Edits, replayed: bool) -> Value {
    let mut receipt = json!({
        "name": name, "request": batch.request, "replayed": replayed, "count": batch.targets.len()
    });
    if batch.live_first {
        let states = batch_states(batch, edits);
        let rows: BTreeMap<_, _> = batch
            .targets
            .iter()
            .map(|(key, target)| (key, json!({"target":target,"state":states[key]})))
            .collect();
        receipt["rows"] = json!(rows);
    } else {
        receipt["state"] = json!("Queued");
    }
    receipt
}

fn unresolved(edit: &Edit) -> bool {
    edit.attempted_run.is_some() && matches!(edit.state.as_str(), "Applying" | "Unconfirmed")
}

fn can_retry_readback(edit: &Edit) -> bool {
    unresolved(edit) && !edit.persistence_checked && edit.readback_attempts < READBACK_ATTEMPTS
}

fn needs_stop_readback(edit: &Edit) -> bool {
    !edit.persistence_checked && (edit.state == "AwaitingNormalStop" || unresolved(edit))
}

fn submit(
    root: &Path,
    name: &str,
    request: &str,
    mut targets: Targets,
    mode: Mode,
) -> Result<Value> {
    let _writer = instance_writer_lock(root, name)?;
    let path = entry(root, name)?;
    let _lock = lock(root, name)?;
    let mut queue = load(&path)?;
    // Normalize numbers before checking request identity even if the core changed since the save.
    for target in targets.values_mut() {
        if let Some(number) = integer(target, 23999) {
            *target = number.to_string();
        }
    }
    let archived;
    let previous = match queue
        .last_batch
        .as_ref()
        .filter(|saved| saved.request == request)
    {
        Some(previous) => Some(previous),
        None => {
            archived = read_batch(root, name, request)?;
            archived.as_ref()
        }
    };
    if let Some(previous) = previous {
        if mode == Mode::Edit
            || previous.live_first != (mode == Mode::LiveFirst)
            || previous.targets != targets
        {
            return Err("RequestConflict".into());
        }
        return Ok(batch_receipt(name, previous, &queue.edits, true));
    }
    if let Some((key, existing)) = queue.edits.iter().find(|(_, edit)| edit.request == request) {
        if mode != Mode::Edit || targets.get(key) != Some(&existing.target) {
            return Err("RequestConflict".into());
        }
        return Ok(json!({"name":name,"request":request,"state":existing.state}));
    }

    let config = instance(root, name)?;
    let (directory, core) = verified(root, &config)?;
    core_maintenance::assert_start_allowed(&directory)?;
    for (key, target) in &targets {
        if !Target::field_for(&core.minecraft, key)?.valid(target) {
            return Err("InvalidWorldValue".into());
        }
    }
    let current = status(root, &config)?;
    if !matches!(
        current["state"].as_str(),
        Some("Stopped" | "Starting" | "Listening")
    ) {
        return Err("WorldRunUnavailable".into());
    }
    if current["state"] != "Stopped" && current["run"]["id"].as_str().is_none() {
        return Err("WorldRunUnavailable".into());
    }
    let created_at = epoch_ms();
    let queued_after = current["run"]["id"].as_str().map(str::to_owned);
    let running = (current["state"] != "Stopped")
        .then_some(queued_after.as_deref())
        .flatten();
    if targets.iter().any(|(key, target)| {
        queue.edits.get(key).is_some_and(|edit| {
            unresolved(edit) && (running == edit.attempted_run.as_deref() || *target == edit.target)
        })
    }) {
        return Err("WorldOutcomeUnconfirmed".into());
    }
    // Retain the latest observed results before any edit can be superseded.
    if let Some(mut previous) = queue.last_batch.clone() {
        previous.states = batch_states(&previous, &queue.edits);
        if mode != Mode::Edit {
            archive_last_batch(root, name, &previous)?;
        } else {
            queue.last_batch = Some(previous);
        }
    }
    for (key, target) in &targets {
        queue.edits.insert(
            key.clone(),
            Edit {
                request: request.into(),
                target: target.clone(),
                sha256: core.sha256.clone(),
                queued_after: queued_after.clone(),
                attempted_run: None,
                state: "Queued".into(),
                created_at,
                readback_attempts: 0,
                persistence_checked: false,
            },
        );
    }
    if mode != Mode::Edit {
        // The active edits and this receipt share one atomic publication. Prior receipts
        // may be archived independently because their edits were already committed.
        queue.last_batch = Some(Batch {
            request: request.into(),
            targets,
            live_first: mode == Mode::LiveFirst,
            states: BTreeMap::new(),
        });
    }
    core_maintenance::write_durable(&path, &queue, path.exists())?;
    if mode == Mode::Edit {
        return Ok(json!({"name":name,"request":request,"state":"Queued"}));
    }
    let batch = queue.last_batch.clone().ok_or("InvalidWorldJournal")?;
    if let Some(run) = running.filter(|_| mode == Mode::LiveFirst) {
        if let Ok(target) = Target::get(root, name, run) {
            for (key, value) in &batch.targets {
                let field = target.field(key)?;
                apply_live_rule(
                    &path,
                    &mut queue,
                    key,
                    run,
                    value,
                    || target.query(root, &field),
                    || {
                        target
                            .assert_same_run(root)
                            .map_err(|_| "WorldRunNotReady")?;
                        host_pipe::request_world_command(
                            root,
                            name,
                            &target.run,
                            &field.set_line(value),
                        )
                    },
                )?;
            }
        }
    }
    Ok(batch_receipt(name, &batch, &queue.edits, false))
}

fn explicitly_not_sent(error: &str) -> bool {
    // The Host returns these before writing bytes to the console. Transport
    // failures and timeout replies do not prove that the Host never wrote.
    matches!(
        error,
        "WorldRunNotReady"
            | "WorldCommandUnavailable"
            | "WorldConsoleUnavailable"
            | "WorldConsoleInputNotVerified"
            | "WorldConsoleInputBusy"
    )
}

fn record_resolution(edit: &mut Edit, verified: bool) {
    edit.state = if verified {
        "AwaitingNormalStop"
    } else {
        "Unconfirmed"
    }
    .into();
}

fn apply_live_rule(
    path: &Path,
    queue: &mut Queue,
    key: &str,
    run: &str,
    value: &str,
    query: impl Fn() -> Result<String>,
    set: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let Ok(before) = query() else {
        // Only a read was attempted, so the durable queued edit may run later.
        return Ok(());
    };
    let edit = queue.edits.get_mut(key).ok_or("InvalidWorldJournal")?;
    edit.attempted_run = Some(run.into());
    if before == value {
        record_resolution(edit, true);
        return core_maintenance::write_durable(path, queue, true);
    }
    // Journal before handing any mutating command to the Host. A crash
    // between publication and confirmation leaves only readback work.
    edit.state = "Applying".into();
    core_maintenance::write_durable(path, queue, true)?;
    let outcome = set();
    if outcome
        .as_ref()
        .is_err_and(|error| explicitly_not_sent(error))
    {
        let edit = queue.edits.get_mut(key).ok_or("InvalidWorldJournal")?;
        edit.attempted_run = None;
        edit.state = "Queued".into();
    } else {
        let verified = query().is_ok_and(|actual| actual == value);
        let edit = queue.edits.get_mut(key).ok_or("InvalidWorldJournal")?;
        record_resolution(edit, verified);
    }
    core_maintenance::write_durable(path, queue, true)
}

fn confirm_live_rule(edit: &mut Edit, query: impl FnOnce() -> Result<String>) {
    let verified = query().is_ok_and(|actual| actual == edit.target);
    edit.readback_attempts += 1;
    record_resolution(edit, verified);
}

pub(super) fn recover_live_rules(root: &Path, name: &str) -> Result<bool> {
    let path = entry(root, name)?;
    if !load(&path)?.edits.values().any(can_retry_readback) {
        return Ok(false);
    }
    let _writer = instance_writer_lock(root, name)?;
    let _lock = lock(root, name)?;
    let mut queue = load(&path)?;
    let config = instance(root, name)?;
    let current = status(root, &config)?;
    let Some(run) = current["run"]["id"].as_str() else {
        return Ok(false);
    };
    let maybe_edit = queue
        .edits
        .iter_mut()
        .find(|(_, edit)| can_retry_readback(edit) && edit.attempted_run.as_deref() == Some(run));
    let Some((key, edit)) = maybe_edit else {
        return Ok(false);
    };
    let Ok(target) = Target::get(root, name, run) else {
        return Ok(false);
    };
    if target.profile.sha256 != edit.sha256 {
        return Ok(false);
    }
    let field = target.field(key)?;
    confirm_live_rule(edit, || target.query(root, &field));
    core_maintenance::write_durable(&path, &queue, true)?;
    Ok(true)
}

pub(super) fn values(root: &Path, name: &str) -> Result<Value> {
    if !token(name) {
        return Err("InvalidWorldRequest".into());
    }
    let config = instance(root, name)?;
    let (directory, core) = verified(root, &config)?;
    let keys: Vec<_> = RULE_KEYS
        .iter()
        .map(|(key, _, _)| *key)
        .chain(std::iter::once(TIME))
        .filter(|key| Target::field_for(&core.minecraft, key).is_ok())
        .collect();
    let edits = load(&entry(root, name)?)?.edits;
    let observation = status(root, &config)?;
    let stopped = observation["state"] == "Stopped";
    let mut saved = BTreeMap::new();
    if stopped {
        // Serialize the stopped-world read with launch preflight. Otherwise the
        // short read lease can still be held when the new Host claims the world.
        let _writer = instance_writer_lock(root, name)?;
        if let Ok(_lease) = claim_runtime_directory(&directory) {
            if status(root, &config)?["state"] == "Stopped" {
                saved = prepare_diagnostics::observe_saved_world(&directory, &core, &keys);
            }
        }
    } else if observation["state"] == "Listening" {
        // The atomic world save may reflect an earlier tick while Java is running.
        // Present it as the latest saved value without treating it as a live readback.
        saved = prepare_diagnostics::observe_saved_world(&directory, &core, &keys);
    }
    let mut response = serde_json::Map::new();
    for key in keys {
        let edit = edits.get(key);
        let state = match edit {
            Some(edit) if edit.sha256 != core.sha256 => "Unconfirmed",
            Some(edit) => edit.state.as_str(),
            None if saved.contains_key(key) && stopped => "Saved",
            None if saved.contains_key(key) => "LastSaved",
            None => "Unknown",
        };
        response.insert(
            key.into(),
            json!({
                "value":saved.get(key),
                "target":edit.filter(|edit| edit.state != "Saved").map(|edit| &edit.target),
                "state":state
            }),
        );
    }
    Ok(Value::Object(response))
}

fn eligible(edit: &Edit, run: &str) -> bool {
    edit.state == "Queued"
        && edit.queued_after.as_deref() != Some(run)
        && edit.attempted_run.is_none()
}

pub(in crate::windows) fn has_queued(root: &Path, name: &str, run: &str) -> Result<bool> {
    Ok(load(&entry(root, name)?)?
        .edits
        .values()
        .any(|edit| eligible(edit, run)))
}

pub(in crate::windows) fn apply_queued(
    root: &Path,
    config: &Instance,
    run: &Run,
    write: impl Fn(&str, Instant) -> Result<()>,
) -> Result<()> {
    let (directory, core) = verified(root, config)?;
    let name = &config.name;
    let path = entry(root, name)?;
    let edits = load(&path)?.edits;
    let mut keys: Vec<_> = edits.keys().collect();
    keys.sort_by_key(|key| key.as_str() == TIME);
    let mut failure = None;
    for key in keys {
        let target = {
            let _lock = lock(root, name)?;
            let mut current = load(&path)?;
            let Some(edit) = current
                .edits
                .get_mut(key)
                .filter(|edit| eligible(edit, &run.id))
            else {
                continue;
            };
            if edit.sha256 != core.sha256 {
                edit.state = "Unconfirmed".into();
                core_maintenance::write_durable(&path, &current, true)?;
                continue;
            }
            edit.clone()
        };
        let field = Target::field_for(&core.minecraft, key)?;
        let setter = field.set_line(&target.target);
        let attempted = std::cell::Cell::new(false);
        let rejected = std::cell::Cell::new(false);
        let write_guarded = |line: &str, deadline| {
            if line == setter {
                let _lock = lock(root, name)?;
                let mut current = load(&path)?;
                let edit = current
                    .edits
                    .get_mut(key)
                    .filter(|edit| edit.request == target.request && eligible(edit, &run.id))
                    .ok_or("WorldRuleRequestChanged")?;
                edit.state = "Applying".into();
                edit.attempted_run = Some(run.id.clone());
                core_maintenance::write_durable(&path, &current, true)?;
                attempted.set(true);
            }
            let result = write(line, deadline);
            if line == setter {
                rejected.set(
                    result
                        .as_ref()
                        .is_err_and(|error| explicitly_not_sent(error)),
                );
            }
            result
        };
        let result = first_ready::apply_rules(
            root,
            &directory,
            config,
            run,
            &core,
            &[(key.clone(), target.target.clone())],
            write_guarded,
            || {},
        );
        if result.is_ok() || attempted.get() {
            let _lock = lock(root, name)?;
            let mut current = load(&path)?;
            if let Some(edit) = current
                .edits
                .get_mut(key)
                .filter(|edit| edit.request == target.request)
            {
                if rejected.get() {
                    edit.state = "Queued".into();
                    edit.attempted_run = None;
                } else {
                    edit.attempted_run = Some(run.id.clone());
                    record_resolution(edit, result.is_ok());
                }
                core_maintenance::write_durable(&path, &current, true)?;
            }
        }
        if let Err(error) = result {
            if error != "WorldRuleRequestChanged" && failure.is_none() {
                failure = Some(error);
            }
        }
    }
    failure.map_or(Ok(()), Err)
}

pub(super) fn reconcile(root: &Path, name: &str) -> Result<()> {
    let path = entry(root, name)?;
    if !path.exists() {
        return Ok(());
    }
    let _lock = lock(root, name)?;
    let mut queue = load(&path)?;
    if !queue.edits.values().any(needs_stop_readback) {
        return Ok(());
    }
    let config = instance(root, name)?;
    let observation = status(root, &config)?;
    if observation["state"] != "Stopped" {
        return Ok(());
    }
    let mut saved = BTreeMap::new();
    if let Some(run) = observation["run"]["id"].as_str() {
        let folder = child_directory(&child_directory(root, "runs")?, name)?;
        let release = folder.join("run-release.json");
        if observation["stopPhase"] == "Sent" && release.is_file() {
            no_redirect(&release)?;
            let settled: Value = read(&release)?;
            if settled["run"] == observation["run"] && settled["exit"] == 0 {
                let (directory, core) = verified(root, &config)?;
                let _lease = claim_runtime_directory(&directory)?;
                let keys: Vec<_> = queue
                    .edits
                    .iter()
                    .filter(|(_, edit)| {
                        needs_stop_readback(edit)
                            && edit.attempted_run.as_deref() == Some(run)
                            && edit.sha256 == core.sha256
                    })
                    .map(|(key, _)| key.as_str())
                    .collect();
                if !keys.is_empty() {
                    saved = prepare_diagnostics::observe_saved_world(&directory, &core, &keys);
                }
            }
        }
    }
    for (key, edit) in &mut queue.edits {
        if !needs_stop_readback(edit) {
            continue;
        }
        edit.state = if saved.get(key) == Some(&edit.target) {
            "Saved"
        } else {
            "Unconfirmed"
        }
        .into();
        edit.persistence_checked = true;
    }
    core_maintenance::write_durable(&path, &queue, true)
}

pub(super) fn needs_reconcile(root: &Path, name: &str) -> Result<bool> {
    Ok(load(&entry(root, name)?)?
        .edits
        .values()
        .any(needs_stop_readback))
}

pub(super) fn activity(root: &Path, name: &str) -> Result<Vec<Value>> {
    let mut work: BTreeMap<String, (u8, Value)> = BTreeMap::new();
    for (key, edit) in load(&entry(root, name)?)?.edits {
        let (rank, state, message) = match edit.state.as_str() {
            "Saved" => (0, "Completed", "已核對世界存檔"),
            "AwaitingNormalStop" => (1, "VerifiedLive", "正常停止後自動核對存檔"),
            "Queued" => (2, "Pending", "下次啟動套用"),
            "Applying" => (3, "Pending", "世界變更待核對"),
            "Unconfirmed" if !edit.persistence_checked => {
                (4, "Unconfirmed", "已嘗試送出，結果未確認")
            }
            _ => (4, "Unconfirmed", "世界存檔結果未確認"),
        };
        if work
            .get(&edit.request)
            .is_some_and(|(previous, _)| *previous >= rank)
        {
            continue;
        }
        let row = json!({"id":edit.request,"instance":name,"state":state,"message":message,
               "createdAt":edit.created_at,"action":if key == TIME { "world-live-time" } else { "world-live-rule" }});
        work.insert(edit.request, (rank, row));
    }
    Ok(work.into_values().map(|(_, row)| row).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
                "ih-world-queue-test-{}-{}",
                std::process::id(),
                clock()
            ));
            crate::windows::dispatch(&[root.to_string_lossy().into_owned(), "init".into()])
                .unwrap();
            let directory = root.join("instances").join("paper");
            fs::create_dir(&directory).unwrap();
            let jar = directory.join("server.jar");
            fs::write(&jar, b"synthetic registered artifact").unwrap();
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let port = listener.local_addr().unwrap().port();
            fs::write(
                directory.join("server.properties"),
                format!("server-ip=127.0.0.1\nserver-port={port}\n"),
            )
            .unwrap();
            drop(listener);
            save(
                &record_path(&root, "paper"),
                &Instance {
                    name: "paper".into(),
                    java: directory.join("unused-java.exe"),
                    port,
                    velocity_listen_address: None,
                    removed: false,
                    directory: None,
                    jvm: None,
                    timing: None,
                    auto_restart: None,
                    core_profile: Some(CoreProfile {
                        family: "PAPER".into(),
                        minecraft: "26.3".into(),
                        build: "100".into(),
                        sha256: content_revision::file_revision(&jar).unwrap(),
                    }),
                    core_local_create: true,
                    management_discovered: false,
                },
            )
            .unwrap();
            Self(root)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn batch_validates_before_writing_and_replays_after_another_save() {
        let fixture = Fixture::new();
        let root = &fixture.0;
        let path = entry(root, "paper").unwrap();
        let legacy = Edits::from([(
            TIME.into(),
            Edit {
                request: "legacy-time".into(),
                target: "6000".into(),
                sha256: "legacy-fixture".into(),
                queued_after: None,
                attempted_run: None,
                state: "Queued".into(),
                created_at: 1,
                readback_attempts: 0,
                persistence_checked: false,
            },
        )]);
        save(&path, &legacy).unwrap();
        let before = fs::read(&path).unwrap();
        assert!(submit_rules(
            root,
            "paper",
            "batch-one",
            r#"{"gamerules.spawn_mobs":"false","serverProperties.pvp":"INVALID"}"#
        )
        .is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(submit_rules(root, "paper", "batch-one", r#"{"initialTime":"2000"}"#).is_err());
        let targets = r#"{"gamerules.spawn_mobs":"false","serverProperties.pvp":"false"}"#;
        assert_eq!(
            submit_rules(root, "paper", "batch-one", targets).unwrap()["count"],
            2
        );
        assert_eq!(load(&path).unwrap().edits[TIME].target, "6000");
        assert_eq!(
            activity(root, "paper")
                .unwrap()
                .iter()
                .filter(|row| row["id"] == "batch-one")
                .count(),
            1
        );
        let replacement = r#"{"gamerules.spawn_mobs":"true","serverProperties.pvp":"true"}"#;
        submit_rules(root, "paper", "batch-two", replacement).unwrap();
        assert_eq!(
            submit_rules(root, "paper", "batch-one", targets).unwrap()["replayed"],
            true
        );
        assert_eq!(
            submit_rules(root, "paper", "batch-one", replacement).unwrap_err(),
            "RequestConflict"
        );
        assert_eq!(
            load(&path).unwrap().edits["serverProperties.pvp"].target,
            "true"
        );
    }

    #[test]
    fn ambiguous_live_rules_are_reconciled_through_readback_without_replay() {
        let fixture = Fixture::new();
        let root = &fixture.0;
        let path = entry(root, "paper").unwrap();
        let batch = r#"{"gamerules.spawn_mobs":"false","serverProperties.pvp":"false"}"#;
        let pvp = "serverProperties.pvp";
        let mobs = "gamerules.spawn_mobs";
        let send_count = std::cell::Cell::new(0);
        assert_eq!(
            save_rules(root, "paper", "live-batch", batch).unwrap()["rows"][pvp]["state"],
            "Queued"
        );
        let mut queue = load(&path).unwrap();
        queue.edits.get_mut(pvp).unwrap().queued_after = Some("first-run".into());
        queue.edits.get_mut(mobs).unwrap().queued_after = Some("first-run".into());
        apply_live_rule(
            &path,
            &mut queue,
            pvp,
            "first-run",
            "false",
            || Ok("true".into()),
            || {
                let recorded = load(&path).unwrap();
                assert_eq!(recorded.edits[pvp].state, "Applying");
                assert_eq!(
                    recorded.edits[pvp].attempted_run.as_deref(),
                    Some("first-run")
                );
                send_count.set(send_count.get() + 1);
                Err("Timeout".into())
            },
        )
        .unwrap();
        apply_live_rule(
            &path,
            &mut queue,
            mobs,
            "first-run",
            "false",
            || Ok("true".into()),
            || Err("WorldConsoleInputNotVerified".into()),
        )
        .unwrap();
        let replay = save_rules(root, "paper", "live-batch", batch).unwrap();
        assert_eq!(replay["replayed"], true);
        assert_eq!(replay["rows"][pvp]["state"], "Unconfirmed");
        assert_eq!(replay["rows"][mobs]["state"], "Queued");
        let duplicate = r#"{"serverProperties.pvp":"false"}"#;
        assert_eq!(
            save_rules(root, "paper", "duplicate", duplicate).unwrap_err(),
            "WorldOutcomeUnconfirmed"
        );
        assert!(!eligible(&queue.edits[pvp], "second-run"));
        assert!(eligible(&queue.edits[mobs], "second-run"));
        assert!(!eligible(&queue.edits[mobs], "first-run"));
        let edit = queue.edits.get_mut(pvp).unwrap();
        assert!(can_retry_readback(edit));
        confirm_live_rule(edit, || Ok("false".into()));
        core_maintenance::write_durable(&path, &queue, true).unwrap();
        assert_eq!(send_count.get(), 1);
        assert!(!eligible(&queue.edits[pvp], "second-run"));
        assert_eq!(
            save_rules(root, "paper", "live-batch", batch).unwrap()["rows"][pvp]["state"],
            "AwaitingNormalStop"
        );
        save_rules(
            root,
            "paper",
            "replace",
            r#"{"serverProperties.pvp":"true"}"#,
        )
        .unwrap();
        assert_eq!(
            save_rules(root, "paper", "live-batch", batch).unwrap()["rows"][pvp]["state"],
            "AwaitingNormalStop"
        );
    }
}
