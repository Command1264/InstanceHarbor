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
    /// A live-first save that could not be sent yet, for example while the user
    /// was typing in the console. The Host retries it during the same run.
    #[serde(default, skip_serializing_if = "is_false")]
    live_retry: bool,
    /// After a normal stop, an unsent rule is written into the world save:
    /// `Written`, or the reason it was left to the next start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    save_write: Option<String>,
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
    let jar = core_artifact::jar(&directory, config);
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
            .map(|(key, target)| {
                let retry = states[key] == "Queued"
                    && edits.get(key).is_some_and(|edit| edit.request == batch.request && edit.live_retry);
                (key, json!({"target":target,"state":if retry { "LiveRetry" } else { states[key].as_str() }}))
            })
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
                live_retry: false,
                save_write: None,
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
        // Only a read was attempted. The Host retries the edit while this run lasts.
        let edit = queue.edits.get_mut(key).ok_or("InvalidWorldJournal")?;
        edit.live_retry = true;
        return core_maintenance::write_durable(path, queue, true);
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
        edit.live_retry = true;
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
            Some(edit) if edit.state == "Queued" && edit.live_retry && !stopped
                && observation["run"]["id"].as_str() == edit.queued_after.as_deref() => "LiveRetry",
            Some(edit) if edit.state == "Queued" && edit.save_write.as_deref() == Some("Written") => "WrittenToSave",
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
                "state":state,
                // Why an unsent rule could not be written to the save; the next start applies it.
                "saveWriteReason":edit
                    .filter(|edit| edit.state == "Queued")
                    .and_then(|edit| edit.save_write.as_deref())
                    .filter(|reason| *reason != "Written")
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

/// A live-first edit saved during this run that the Host could not send yet.
fn live_retry_due(edit: &Edit, run: &str) -> bool {
    edit.state == "Queued"
        && edit.live_retry
        && edit.queued_after.as_deref() == Some(run)
        && edit.attempted_run.is_none()
}

pub(in crate::windows) fn has_queued(root: &Path, name: &str, run: &str) -> Result<bool> {
    Ok(load(&entry(root, name)?)?
        .edits
        .values()
        .any(|edit| eligible(edit, run)))
}

pub(in crate::windows) fn has_live_retry(root: &Path, name: &str, run: &str) -> Result<bool> {
    Ok(load(&entry(root, name)?)?
        .edits
        .values()
        .any(|edit| live_retry_due(edit, run)))
}

/// Applies edits queued before this run started.
pub(in crate::windows) fn apply_queued(
    root: &Path,
    config: &Instance,
    run: &Run,
    write: impl Fn(&str, Instant) -> Result<()>,
) -> Result<()> {
    apply_matching(root, config, run, write, eligible)
}

/// Applies live-first edits of this run once the console can take them.
pub(in crate::windows) fn apply_live_retry(
    root: &Path,
    config: &Instance,
    run: &Run,
    write: impl Fn(&str, Instant) -> Result<()>,
) -> Result<()> {
    apply_matching(root, config, run, write, live_retry_due)
}

fn apply_matching(
    root: &Path,
    config: &Instance,
    run: &Run,
    write: impl Fn(&str, Instant) -> Result<()>,
    due: fn(&Edit, &str) -> bool,
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
                .filter(|edit| due(edit, &run.id))
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
                    .filter(|edit| edit.request == target.request && due(edit, &run.id))
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
                    // Not sent; a live retry stays due for the next attempt.
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

/// The run id when the last run stopped through InstanceHarbor and Java exited 0.
fn normal_stop_run(root: &Path, name: &str, observation: &Value) -> Result<Option<String>> {
    let Some(run) = observation["run"]["id"].as_str() else {
        return Ok(None);
    };
    let folder = child_directory(&child_directory(root, "runs")?, name)?;
    let release = folder.join("run-release.json");
    if observation["stopPhase"] != "Sent" || !release.is_file() {
        return Ok(None);
    }
    no_redirect(&release)?;
    let settled: Value = read(&release)?;
    Ok((settled["run"] == observation["run"] && settled["exit"] == 0).then(|| run.to_owned()))
}

/// A game rule that was never sent and has not been written to the save yet.
fn awaits_save_write(edit: &Edit) -> bool {
    edit.state == "Queued" && edit.attempted_run.is_none() && edit.save_write.is_none()
}

const SAVE_BACKUPS: usize = 20;

fn save_backup(root: &Path, name: &str, file: &str) -> Result<PathBuf> {
    let folder = child_directory(root, "history")?.join(format!("world-save-backups-{name}"));
    match fs::create_dir(&folder) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(fail(error)),
    }
    no_redirect(&folder)?;
    let mut existing: Vec<_> = fs::read_dir(&folder)
        .map_err(fail)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .map(|entry| entry.path())
        .collect();
    existing.sort();
    // Names start with a millisecond clock, so the oldest sort first.
    while existing.len() >= SAVE_BACKUPS {
        fs::remove_file(existing.remove(0)).map_err(fail)?;
    }
    Ok(folder.join(format!("{:013}-{file}", clock())))
}

/// Writes unsent game rules into the world save after a normal stop, so the save
/// already holds them. The next start still confirms each one through the console.
pub(super) fn write_unsent(root: &Path, name: &str) -> Result<()> {
    let path = entry(root, name)?;
    if !path.exists() {
        return Ok(());
    }
    let _lock = lock(root, name)?;
    let mut queue = load(&path)?;
    if !queue.edits.iter().any(|(key, edit)| key != TIME && awaits_save_write(edit)) {
        return Ok(());
    }
    let config = instance(root, name)?;
    let observation = status(root, &config)?;
    if observation["state"] != "Stopped" {
        return Ok(());
    }
    let run = normal_stop_run(root, name, &observation)?;
    let (directory, core) = verified(root, &config)?;
    let keys: Vec<String> = queue
        .edits
        .iter()
        .filter(|(key, edit)| *key != TIME && awaits_save_write(edit))
        .map(|(key, _)| key.clone())
        .collect();
    let mut targets = Vec::new();
    for key in &keys {
        let edit = &queue.edits[key];
        if edit.sha256 != core.sha256 {
            queue.edits.get_mut(key).ok_or("InvalidWorldJournal")?.save_write = Some("WorldCoreChanged".into());
        } else if run.is_none() || edit.queued_after != run {
            // Without proof of a normal stop the save may be incomplete; the next start applies it.
            queue.edits.get_mut(key).ok_or("InvalidWorldJournal")?.save_write = Some("WorldStopUnverified".into());
        } else {
            targets.push((key.clone(), edit.target.clone()));
        }
    }
    if !targets.is_empty() {
        // The Host can hold the world for a moment after Java exits. Record nothing then;
        // the Manager sweep or the next start preflight tries again.
        let Ok(_lease) = claim_runtime_directory(&directory) else {
            return core_maintenance::write_durable(&path, &queue, true);
        };
        let file = if core.minecraft.starts_with("26.") { "game_rules.dat" } else { "level.dat" };
        let outcome = save_backup(root, name, file)
            .and_then(|backup| prepare_diagnostics::write_saved_rules(&directory, &core, &targets, &backup));
        let result = match outcome {
            Ok(()) => "Written".to_owned(),
            Err(error) => error,
        };
        for (key, _) in &targets {
            queue.edits.get_mut(key).ok_or("InvalidWorldJournal")?.save_write = Some(result.clone());
        }
    }
    core_maintenance::write_durable(&path, &queue, true)
}

pub(super) fn needs_save_write(root: &Path, name: &str) -> Result<bool> {
    Ok(load(&entry(root, name)?)?
        .edits
        .iter()
        .any(|(key, edit)| key != TIME && awaits_save_write(edit)))
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
    if let Some(run) = normal_stop_run(root, name, &observation)? {
        let (directory, core) = verified(root, &config)?;
        let _lease = claim_runtime_directory(&directory)?;
        let keys: Vec<_> = queue
            .edits
            .iter()
            .filter(|(_, edit)| {
                needs_stop_readback(edit)
                    && edit.attempted_run.as_deref() == Some(run.as_str())
                    && edit.sha256 == core.sha256
            })
            .map(|(key, _)| key.as_str())
            .collect();
        if !keys.is_empty() {
            saved = prepare_diagnostics::observe_saved_world(&directory, &core, &keys);
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
            "Queued" if edit.save_write.as_deref() == Some("Written") => {
                (2, "Pending", "已寫入世界存檔，下次啟動時核對")
            }
            "Queued" if edit.save_write.as_deref().is_some_and(|reason| reason != "WorldStopUnverified") => {
                (2, "Pending", "無法寫入世界存檔，下次啟動套用")
            }
            "Queued" if edit.live_retry => (2, "Pending", "等後台輸入完成後即時套用；未送出時下次啟動套用"),
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
                    core_file: None,
                    name: "paper".into(),
                    java: directory.join("unused-java.exe"),
                    port,
                    velocity_listen_address: None,
                    removed: false,
                    directory: None,
                    jvm: None,
                    timing: None,
                    auto_restart: None,
                    core_profile: Some(CoreProfile { unverified: false, java: None,
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
                live_retry: false,
                save_write: None,
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
        // Not sent during this run: the Host retries it live, then the next start applies it.
        assert_eq!(replay["rows"][mobs]["state"], "LiveRetry");
        assert!(live_retry_due(&queue.edits[mobs], "first-run"));
        assert!(!live_retry_due(&queue.edits[mobs], "second-run"));
        assert!(!live_retry_due(&queue.edits[pvp], "first-run"));
        let mut read_only = load(&path).unwrap();
        read_only.edits.get_mut(pvp).unwrap().live_retry = false;
        let pvp_before = read_only.edits[pvp].clone();
        read_only.edits.insert("gamerules.advance_time".into(), Edit { live_retry: false, ..pvp_before });
        let queue_path = path.with_extension("probe.json");
        fs::copy(&path, &queue_path).unwrap();
        apply_live_rule(&queue_path, &mut read_only, "gamerules.advance_time", "first-run", "false",
            || Err("WorldConsoleInputBusy".into()), || panic!("a failed read never sends"))
            .unwrap();
        assert!(read_only.edits["gamerules.advance_time"].live_retry, "a busy console read retries live");
        fs::remove_file(&queue_path).unwrap();
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
    fn queue_rule(root: &Path, queued_after: &str) {
        let config = instance(root, "paper").unwrap();
        let core = config.core_profile.clone().unwrap();
        let queue = Queue {
            edits: Edits::from([(
                "gamerules.spawn_mobs".into(),
                Edit {
                    request: "typed-while-stopping".into(),
                    target: "false".into(),
                    sha256: core.sha256,
                    queued_after: Some(queued_after.into()),
                    attempted_run: None,
                    state: "Queued".into(),
                    created_at: 1,
                    readback_attempts: 0,
                    persistence_checked: false,
                    live_retry: true,
                    save_write: None,
                },
            )]),
            last_batch: None,
        };
        let path = entry(root, "paper").unwrap();
        core_maintenance::write_durable(&path, &queue, path.exists()).unwrap();
    }

    fn stopped_run(root: &Path, through_instanceharbor: bool) -> Value {
        let folder = root.join("runs").join("paper");
        fs::create_dir_all(&folder).unwrap();
        let dead = Identity { pid: u32::MAX, created: 1 };
        let run = Run { id: "run-1".into(), host: dead.clone(), java: dead };
        save(&folder.join("identity.json"), &run).unwrap();
        save(&folder.join("run-release.json"), &json!({"run":run,"exit":0})).unwrap();
        if through_instanceharbor {
            fs::write(folder.join("stop-sent"), b"").unwrap();
        }
        json!(run)
    }

    fn rules_file(root: &Path) -> PathBuf {
        root.join("instances/paper/world/dimensions/minecraft/overworld/data/minecraft/game_rules.dat")
    }

    fn write_world(root: &Path) -> Vec<u8> {
        let file = rules_file(root);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        // {"": {data: {minecraft:spawn_mobs: 1b}}}
        let raw = b"\x0a\0\0\x0a\0\x04data\x01\0\x14minecraft:spawn_mobs\x01\0\0".to_vec();
        use flate2::{write::GzEncoder, Compression};
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        std::io::Write::write_all(&mut encoder, &raw).unwrap();
        let bytes = encoder.finish().unwrap();
        fs::write(&file, &bytes).unwrap();
        bytes
    }

    #[test]
    fn unsent_rules_are_written_to_the_save_after_a_normal_stop() {
        let fixture = Fixture::new();
        let root = &fixture.0;
        let original = write_world(root);
        stopped_run(root, true);
        queue_rule(root, "run-1");
        assert!(world_live::has_queued(root, "paper").unwrap());
        world_live::recover_saved(root, "paper").unwrap();

        let edit = &load(&entry(root, "paper").unwrap()).unwrap().edits["gamerules.spawn_mobs"];
        assert_eq!(edit.save_write.as_deref(), Some("Written"));
        // Still due at the next start, which confirms it through the console.
        assert!(eligible(edit, "run-2"));
        assert!(!world_live::has_queued(root, "paper").unwrap());
        let directory = root.join("instances/paper");
        let core = instance(root, "paper").unwrap().core_profile.unwrap();
        let saved = prepare_diagnostics::observe_saved_world(&directory, &core, &["gamerules.spawn_mobs"]);
        assert_eq!(saved.get("gamerules.spawn_mobs").map(String::as_str), Some("false"));
        assert_eq!(values(root, "paper").unwrap()["gamerules.spawn_mobs"]["state"], "WrittenToSave");
        let backups: Vec<_> = fs::read_dir(root.join("history/world-save-backups-paper")).unwrap().collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read(backups[0].as_ref().unwrap().path()).unwrap(), original);
        assert_eq!(activity(root, "paper").unwrap()[0]["message"], "已寫入世界存檔，下次啟動時核對");
    }

    #[test]
    fn a_stop_without_proof_leaves_the_save_and_explains_why() {
        let fixture = Fixture::new();
        let root = &fixture.0;
        let original = write_world(root);
        stopped_run(root, false);
        queue_rule(root, "run-1");
        world_live::recover_saved(root, "paper").unwrap();

        let edit = &load(&entry(root, "paper").unwrap()).unwrap().edits["gamerules.spawn_mobs"];
        assert_eq!(edit.save_write.as_deref(), Some("WorldStopUnverified"));
        assert_eq!(fs::read(rules_file(root)).unwrap(), original);
        let value = &values(root, "paper").unwrap()["gamerules.spawn_mobs"];
        assert_eq!(value["state"], "Queued");
        assert_eq!(value["saveWriteReason"], "WorldStopUnverified");
        assert_eq!(activity(root, "paper").unwrap()[0]["message"], "等後台輸入完成後即時套用；未送出時下次啟動套用");
        // The reason is recorded once, so the Manager sweep does not retry every second.
        assert!(!world_live::has_queued(root, "paper").unwrap());
    }
}
