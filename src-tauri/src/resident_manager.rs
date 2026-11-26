//! Windows batch lifecycle owner. Persist intent before dispatching any Host command.
use super::*;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

const START_SLOTS: usize = 2;
const READY_MS: u64 = 150_000;
const MAX_TARGETS: usize = 64;
const MAX_ACTIVE_BATCHES: usize = 8;

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Target {
    name: String,
    guard: StartGuard,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
enum Action {
    #[default]
    Start,
    Stop,
    Restart,
}

impl Action {
    fn verb(self) -> &'static str {
        match self {
            Self::Start => "batch-start",
            Self::Stop => "batch-stop",
            Self::Restart => "batch-restart",
        }
    }
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
enum Phase {
    Queued,
    StopQueued,
    StopDispatching,
    Stopping,
    StopUnconfirmed,
    Skipped,
    Dispatching,
    Starting,
    Completed,
    Cancelled,
    Failed,
    Unconfirmed,
}

impl Phase {
    fn pending(self) -> bool {
        self == Self::Queued
            || self == Self::StopQueued
            || self.stopping()
            || matches!(self, Self::Dispatching | Self::Starting)
    }

    fn stopping(self) -> bool {
        matches!(self, Self::StopDispatching | Self::Stopping)
    }

    fn occupies_slot(self) -> bool {
        matches!(self, Self::Dispatching | Self::Starting | Self::Unconfirmed)
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    target: Target,
    registration: String,
    phase: Phase,
    request: String,
    run: Option<Run>,
    deadline: u64,
    reason: Option<String>,
    #[serde(default)]
    stop_run: Option<Run>,
    #[serde(default)]
    stop_intent: Option<BatchStopIntent>,
    #[serde(default)]
    stop_verified_at: Option<u64>,
    #[serde(default)]
    restart_delay_ms: u64,
    #[serde(default)]
    ready_wait_ms: Option<u64>,
    #[serde(default)]
    suppress_start: bool,
    #[serde(default)]
    supersedes: Option<String>,
}

impl Item {
    fn start_guard(&self) -> StartGuard {
        StartGuard {
            registration: Some(self.registration.clone()),
            previous_identity: self.stop_run.clone(),
            // Batch starts never open a console window; it needs a per-start confirmation.
            console_window: false,
            ..self.target.guard.clone()
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Batch {
    id: String,
    created_at: u64,
    items: Vec<Item>,
    #[serde(default)]
    action: Action,
}

struct State {
    batches: Vec<Batch>,
    core_batches: Result<core_batch::Journal>,
    core_in_flight: bool,
    exiting: bool,
    fault: bool,
    /// GUI processes that asked to close with the Manager (tray Quit).
    gui: Vec<AttachedGui>,
}

/// A GUI process that asked to close with the Manager. A PID alone is not enough:
/// Windows reuses PIDs, so Quit checks the creation time and executable again.
struct AttachedGui {
    process: Identity,
    /// Canonical path of the accepted GUI executable it was running at attach.
    image: PathBuf,
}

/// What a PID refers to now: the process identity and its canonical executable.
type GuiProbe = (io::Result<Option<Identity>>, Option<PathBuf>);

fn probe_gui(pid: u32) -> GuiProbe {
    // The image is read first. A matching identity afterwards proves the recorded process
    // was alive the whole time, so the image belonged to it and not to a PID reuse.
    let image = process_image(pid).and_then(|image| image.canonicalize().ok());
    (identity(pid), image)
}

/// The PIDs of attached GUIs that are still the recorded process running the recorded
/// executable. Any other process at such a PID must never receive WM_CLOSE.
fn verified_guis(recorded: &[AttachedGui], probe: impl Fn(u32) -> GuiProbe) -> Vec<u32> {
    recorded
        .iter()
        .filter(|gui| {
            let (current, image) = probe(gui.process.pid);
            recorded_alive(&gui.process, current).unwrap_or(false) && image.as_ref() == Some(&gui.image)
        })
        .map(|gui| gui.process.pid)
        .collect()
}

/// Tray Quit and `app-exit`: ends the Manager and closes the windows of attached GUIs
/// that are still the same processes. `Ok(false)` while batch or core-update work would
/// be cut off.
#[cfg_attr(test, allow(dead_code))]
fn exit_app(state: &Mutex<State>) -> Result<bool> {
    exit_app_locked(state.lock().map_err(fail)?)
}

fn exit_app_locked(mut state: std::sync::MutexGuard<'_, State>) -> Result<bool> {
    if active_work(&state) {
        return Ok(false);
    }
    state.exiting = true;
    let gui = std::mem::take(&mut state.gui);
    drop(state);
    manager_tray::close_gui_windows(&verified_guis(&gui, probe_gui));
    Ok(true)
}

/// Work the Manager must finish before it may exit; servers themselves never count.
fn active_work(state: &State) -> bool {
    state.batches.iter().any(|batch| batch.items.iter().any(|item| item.phase.pending()))
        || state.core_in_flight
        || state.core_batches.as_ref().is_ok_and(core_batch::Journal::active)
}

fn now() -> u64 {
    epoch_ms()
}

fn endpoint(root: &Path) -> Result<String> {
    let key = content_revision::revision(root.to_string_lossy().to_lowercase().as_bytes())?;
    Ok(format!(r"\\.\pipe\instanceharbor-manager-{key}"))
}

fn lock_file(path: &Path) -> Result<File> {
    if path.exists() {
        no_redirect(path)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(fail)?;
    file.try_lock().map_err(|_| "ManagerAlreadyRunning")?;
    Ok(file)
}

fn metadata(root: &Path) -> PathBuf {
    root.join("resident-manager.json")
}

pub(super) fn published_manager(root: &Path) -> Result<Option<Identity>> {
    let path = metadata(root);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(fail(error)),
        Ok(_) => no_redirect(&path)?,
    }
    let peer = read(&path)?;
    Ok(alive(&peer)?.then_some(peer))
}

pub(super) fn observing(root: &Path) -> bool {
    matches!((published_manager(root), executable()), (Ok(Some(peer)), Ok(expected))
        if process_image(peer.pid).and_then(|image| image.canonicalize().ok()).as_ref() == Some(&expected))
}

fn executable() -> Result<PathBuf> {
    let current = std::env::current_exe().map_err(fail)?;
    current
        .with_file_name("ihctl.exe")
        .canonicalize()
        .map_err(fail)
}

fn call(root: &Path, verb: &str, args: &[String]) -> Result<Value> {
    let path = metadata(root);
    if !path.exists() {
        return Err("ManagerUnavailable".into());
    }
    no_redirect(&path)?;
    let peer: Identity = read(&path)?;
    if !alive(&peer)? {
        return Err("ManagerUnavailable".into());
    }
    if process_image(peer.pid).and_then(|image| image.canonicalize().ok()) != Some(executable()?) {
        return Err("PeerMismatch".into());
    }
    let frame = serde_json::to_string(&json!([verb, args])).map_err(fail)?;
    let response = host_pipe::local_request(&endpoint(root)?, &peer, &frame)?;
    let reply: Value = serde_json::from_str(&response).map_err(|_| "ManagerResultUnconfirmed")?;
    if let Some(error) = reply["error"].as_str() {
        Err(error.into())
    } else {
        reply
            .get("result")
            .cloned()
            .ok_or("ManagerResultUnconfirmed".into())
    }
}

pub(super) fn existing_snapshot(root: &Path, include_metadata: bool) -> Result<Value> {
    call(root, "instance-snapshot", &[if include_metadata { "metadata" } else { "rows" }.into()])
}

pub(super) fn client(root: &Path, verb: &str, args: &[String]) -> Result<Value> {
    let command = if matches!(verb, "batch-connect" | "manager-connect") {
        "batch-list"
    } else {
        verb
    };
    if verb != "manager-connect" && command == "batch-list" && args.is_empty() && !root.join("batches").exists()
        && !(verb == "batch-connect" && presentation::core_prefetch_enabled(root).unwrap_or(false))
    {
        return Ok(json!([]));
    }
    match call(root, command, args) {
        Err(error)
            if error == "ManagerUnavailable"
                && matches!(
                    verb,
                    "batch-start"
                        | "batch-stop"
                        | "batch-restart"
                        | "batch-connect"
                        | "core-batch-submit"
                        | "core-batch-cancel"
                        | "manager-connect"
                        | "agent-list"
                        | "console-open"
                        | "host-control"
                        | "host-recover"
                        | "gui-attach"
                ) =>
        {
            launch_detached(&executable()?, root, "manager-serve")?;
        }
        Err(error) if error == "ManagerPipeUnavailable" => {}
        result => return result,
    }
    let until = Instant::now() + Duration::from_secs(3);
    while Instant::now() < until {
        match call(root, command, args) {
            Err(error)
                if matches!(
                    error.as_str(),
                    "ManagerUnavailable" | "ManagerPipeUnavailable"
                ) =>
            {
                thread::sleep(Duration::from_millis(50))
            }
            result => return result,
        }
    }
    Err("ManagerUnavailable".into())
}

/// Servers the list shows as running: the same rule as the GUI's `canStop`.
fn running_servers(snapshot: &Value) -> u32 {
    snapshot["rows"].as_array().map_or(0, |rows| {
        rows.iter()
            .filter(|row| !row["run"].is_null() && matches!(row["state"].as_str(), Some("Listening" | "Starting")))
            .count() as u32
    })
}

/// What tray Quit asks about: every server not confirmed `Stopped` (starting, running,
/// undecided or unreadable), and how many of those rely on the Manager's automatic restart.
fn servers_not_stopped(snapshot: &Value, auto_restart: impl Fn(&str) -> bool) -> (u32, u32) {
    let rows = snapshot["rows"].as_array().map(Vec::as_slice).unwrap_or_default();
    rows.iter()
        .filter(|row| row["state"].as_str() != Some("Stopped"))
        .fold((0, 0), |(count, guarded), row| {
            let restarts = row["name"].as_str().is_some_and(&auto_restart);
            (count + 1, guarded + u32::from(restarts))
        })
}

#[cfg_attr(test, allow(dead_code))]
fn restarts_automatically(root: &Path, name: &str) -> bool {
    instance_name(name)
        && instance(root, name).is_ok_and(|config| {
            !config.removed && config.auto_restart.as_ref().is_some_and(|policy| policy.valid() && policy.enabled())
        })
}

fn batch_path(directory: &Path, id: &str) -> PathBuf {
    directory.join(format!("{id}.json"))
}

fn persist(directory: &Path, batch: &Batch) -> Result<()> {
    let path = batch_path(directory, &batch.id);
    if path.exists() {
        no_redirect(&path)?;
    }
    save(&path, batch)
}

fn load(directory: &Path) -> Result<Vec<Batch>> {
    let mut batches = Vec::new();
    for entry in fs::read_dir(directory).map_err(fail)? {
        let path = entry.map_err(fail)?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        no_redirect(&path)?;
        let batch: Batch = read(&path)?;
        if !token(&batch.id)
            || path != batch_path(directory, &batch.id)
            || batch.items.is_empty()
            || batch.items.len() > MAX_TARGETS
            || batch
                .items
                .iter()
                .any(|item| !instance_name(&item.target.name) || !token(&item.request))
        {
            return Err("InvalidBatchJournal".into());
        }
        batches.push(batch);
    }
    batches.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
    Ok(batches)
}

fn view(batch: &Batch) -> Value {
    let phase = if batch.items.iter().any(|item| item.phase.pending()) {
        "Pending"
    } else if batch
        .items
        .iter()
        .any(|item| matches!(item.phase, Phase::Unconfirmed | Phase::StopUnconfirmed))
    {
        "Unconfirmed"
    } else if batch.items.iter().any(|item| item.phase == Phase::Failed) {
        "Failed"
    } else if batch
        .items
        .iter()
        .any(|item| item.phase == Phase::Cancelled)
    {
        "Cancelled"
    } else {
        "Completed"
    };
    let items: Vec<_> = batch
        .items
        .iter()
        .map(|item| {
            json!({
                "name": item.target.name, "phase": item.phase, "reason": item.reason,
                "runId": item.run.as_ref().or(item.stop_run.as_ref()).map(|run| &run.id),
                "canCancel": matches!(item.phase, Phase::Queued | Phase::StopQueued)
                    || (batch.action == Action::Restart && item.phase.stopping() && !item.suppress_start),
            })
        })
        .collect();
    let name = match batch.action {
        Action::Start => "批次啟動",
        Action::Stop => "批次正常停止",
        Action::Restart => "批次重新啟動",
    };
    json!({"id":batch.id,"action":batch.action.verb(),"instance":name,
        "createdAt":batch.created_at,"state":phase,"items":items})
}

fn reply(root: &Path, directory: &Path, state: &Mutex<State>, input: &str) -> Result<Value> {
    let (verb, args): (String, Vec<String>) =
        serde_json::from_str(input).map_err(|_| "InvalidManagerRequest")?;
    if verb == "instance-snapshot" {
        return match args.as_slice() {
            [mode] if matches!(mode.as_str(), "metadata" | "rows") => {
                instance_snapshot::read(root, mode == "metadata")
            }
            _ => Err("InvalidManagerRequest".into()),
        };
    }
    if verb == "agent-list" {
        return agent_inventory::list(root, &args);
    }
    let mut state = state.lock().map_err(fail)?;
    if state.exiting {
        return Err("ManagerExiting".into());
    }
    if state.fault {
        return Err("ManagerJournalUnavailable".into());
    }
    if matches!(verb.as_str(), "host-control" | "host-recover") {
        drop(state);
        let [name, expected, trailing @ ..] = args.as_slice() else {
            return Err("InvalidHostArguments".into());
        };
        if !instance_name(name) || !token(expected) {
            return Err("InvalidHostIdentity".into());
        }
        let stopping = matches!(trailing, [request] if verb == "host-control"
            && (request == &format!("Stop {expected}") || request == &format!("ForceStop {expected}")));
        if !stopping && !admitted(root, name)? {
            return Err("InvalidHostIdentity".into());
        }
        let config = instance(root, name)?;
        if config.removed {
            return Err("InstanceRemoved".into());
        }
        let observed = status(root, &config)?;
        if !matches!(observed["state"].as_str(), Some("Listening" | "Starting")) {
            return Err("HostUnconfirmed".into());
        }
        let run: Run = serde_json::from_value(observed["run"].clone()).map_err(fail)?;
        if run.id != *expected {
            return Err("RunMismatch".into());
        }
        return match (verb.as_str(), trailing) {
            ("host-recover", []) => {
                host_pipe::recover(root, &run)?;
                Ok(json!({"state":"Recovered","run":run}))
            }
            ("host-control", [request]) => {
                if request.contains(['\n', '\r'])
                    || !(request == &format!("Stop {expected}")
                        || request == &format!("ForceStop {expected}")
                        || request.starts_with(&format!("World {expected} ")))
                {
                    return Err("InvalidHostArguments".into());
                }
                Ok(json!(host_pipe::control_as_manager(&run, request)))
            }
            _ => Err("InvalidHostArguments".into()),
        };
    }
    if let ("console-open", [name, run]) = (verb.as_str(), args.as_slice()) {
        drop(state);
        return console_viewer::open(root, name, run);
    }
    if verb == "batch-list" && args.is_empty() {
        let list: Vec<_> = state
            .batches
            .iter()
            .rev()
            .enumerate()
            .filter(|(index, batch)| *index < 8 || active(batch))
            .map(|(_, batch)| view(batch))
            .collect();
        return Ok(json!(list));
    }
    match (verb.as_str(), args.as_slice()) {
        ("core-batch-submit", [id, targets]) => state
            .core_batches
            .as_mut()
            .map_err(|_| "CoreBatchJournalUnavailable")?
            .submit(root, id, targets),
        ("core-batch-cancel", [id]) => state
            .core_batches
            .as_mut()
            .map_err(|_| "CoreBatchJournalUnavailable")?
            .cancel(root, id),
        (verb @ ("batch-start" | "batch-stop" | "batch-restart"), [id, raw]) if token(id) => {
            let action = match verb {
                "batch-stop" => Action::Stop,
                "batch-restart" => Action::Restart,
                _ => Action::Start,
            };
            let targets: Vec<Target> =
                serde_json::from_str(raw).map_err(|_| "InvalidBatchTargets")?;
            if let Some(prior) = state.batches.iter().find(|batch| batch.id == *id) {
                if prior.action == action
                    && prior
                        .items
                        .iter()
                        .map(|item| &item.target)
                        .eq(targets.iter())
                {
                    return Ok(view(prior));
                }
                return Err("RequestConflict".into());
            }
            if targets.is_empty() || targets.len() > MAX_TARGETS {
                return Err("InvalidBatchTargets".into());
            }
            if action == Action::Start
                && state
                    .batches
                    .iter()
                    .flat_map(|batch| &batch.items)
                    .any(|item| item.phase == Phase::Unconfirmed)
            {
                return Err("BatchNeedsReconciliation".into());
            }
            if state.batches.iter().filter(|batch| active(batch)).count() >= MAX_ACTIVE_BATCHES {
                return Err("ManagerQueueFull".into());
            }
            let mut names = HashSet::new();
            let mut items = Vec::new();
            for target in targets {
                if !instance_name(&target.name)
                    || target.guard.registration.is_some()
                    || target.guard.previous_identity.is_some()
                    || !names.insert(target.name.clone())
                {
                    return Err("InvalidBatchTargets".into());
                }
                let existing = state.batches.iter().rev().find_map(|batch| {
                    batch
                        .items
                        .iter()
                        .find(|item| {
                            item.target.name == target.name
                                && (item.phase.pending() || item.phase.occupies_slot())
                                && !superseded(&state, &batch.id, &item.target.name)
                        })
                        .map(|item| (batch, item))
                });
                let superseded = match existing {
                    Some((batch, item))
                        if action == Action::Stop
                            && batch.action != Action::Stop
                            && matches!(
                                item.phase,
                                Phase::Queued
                                    | Phase::StopQueued
                                    | Phase::StopDispatching
                                    | Phase::Stopping
                            ) =>
                    {
                        Some((batch.id.clone(), item))
                    }
                    Some((_, item))
                        if action == Action::Stop
                            && matches!(item.phase, Phase::Starting | Phase::Unconfirmed) =>
                    {
                        None
                    }
                    Some(_) if action == Action::Stop => {
                        let mut item = prepare_item(root, id, target, action, None)?;
                        item.phase = Phase::Failed;
                        item.reason = Some("BatchTargetPending".into());
                        items.push(item);
                        continue;
                    }
                    Some(_) => return Err("BatchTargetPending".into()),
                    None => None,
                };
                items.push(prepare_item(root, id, target, action, superseded)?);
            }
            let batch = Batch {
                id: id.clone(),
                created_at: now(),
                items,
                action,
            };
            persist(directory, &batch)?;
            let result = view(&batch);
            state.batches.push(batch);
            Ok(result)
        }
        ("batch-cancel", [id]) => {
            let batch = state
                .batches
                .iter_mut()
                .find(|batch| batch.id == *id)
                .ok_or("UnknownBatch")?;
            let mut next = batch.clone();
            for item in &mut next.items {
                if matches!(item.phase, Phase::Queued | Phase::StopQueued) {
                    item.phase = Phase::Cancelled;
                } else if next.action == Action::Restart && item.phase.stopping() {
                    item.suppress_start = true;
                }
            }
            persist(directory, &next)?;
            *batch = next;
            Ok(view(batch))
        }
        ("manager-exit", []) => {
            if active_work(&state) {
                return Err("ManagerHasActiveWork".into());
            }
            state.exiting = true;
            Ok(json!({"exiting":true}))
        }
        // The same flow as tray Quit, without its question; Setup uses it to update.
        ("app-exit", []) => {
            if !exit_app_locked(state)? {
                return Err("ManagerHasActiveWork".into());
            }
            Ok(json!({"exiting":true}))
        }
        ("gui-attach", [pid]) => {
            let pid: u32 = pid.parse().map_err(|_| "InvalidManagerRequest")?;
            let (current, image) = probe_gui(pid);
            let exe = executable()?;
            let image = [crate::desktop_channel::GUI_EXECUTABLE, "instanceharbor-acceptance.exe"]
                .iter()
                .filter_map(|name| exe.with_file_name(name).canonicalize().ok())
                .find(|gui| image.as_ref() == Some(gui))
                .ok_or("PeerMismatch")?;
            let process = current.map_err(fail)?.ok_or("PeerMismatch")?;
            let live = verified_guis(&state.gui, probe_gui);
            state.gui.retain(|known| live.contains(&known.process.pid));
            if !state.gui.iter().any(|known| known.process == process) {
                state.gui.push(AttachedGui { process, image });
            }
            Ok(json!({"attached":true}))
        }
        _ => Err("InvalidManagerRequest".into()),
    }
}

fn active(batch: &Batch) -> bool {
    batch
        .items
        .iter()
        .any(|item| item.phase.pending() || item.phase.occupies_slot())
}

fn observed(root: &Path, name: &str) -> Result<Value> {
    let config = instance(root, name)?;
    if config.removed {
        return Err("InstanceRemovedFromManagement".into());
    }
    let included = admitted(root, name)?;
    let mut observation = status(root, &config)?;
    if !included {
        observation["outsideManagement"] = json!(true);
    }
    Ok(observation)
}

fn superseded(state: &State, id: &str, name: &str) -> bool {
    state.batches.iter().any(|batch| {
        batch
            .items
            .iter()
            .any(|item| item.target.name == name && item.supersedes.as_deref() == Some(id))
    })
}

fn prepare_item(
    root: &Path,
    id: &str,
    target: Target,
    action: Action,
    prior: Option<(String, &Item)>,
) -> Result<Item> {
    let request = content_revision::revision(format!("{id}/{}", target.name).as_bytes())?;
    let mut item = Item {
        target,
        request,
        registration: String::new(),
        phase: Phase::Failed,
        run: None,
        deadline: 0,
        reason: None,
        stop_run: None,
        stop_intent: None,
        stop_verified_at: None,
        restart_delay_ms: 0,
        ready_wait_ms: None,
        suppress_start: false,
        supersedes: prior.as_ref().map(|(id, _)| id.clone()),
    };
    let prepare = |item: &mut Item| -> Result<()> {
        let name = &item.target.name;
        let registration = registration_revision(root, name)?;
        let current = observed(root, name)?;
        if action != Action::Stop && current["outsideManagement"] == true {
            return Err("InstanceExcludedOrInvalid".into());
        }
        if registration_revision(root, name)? != registration
            || current["run"]["id"].as_str() != item.target.guard.previous_run.as_deref()
            || current["stopToken"].as_str() != item.target.guard.stop_token.as_deref()
        {
            return Err("StartIntentChanged".into());
        }
        let config = instance(root, name)?;
        if let Some(timing) = &config.timing {
            timing.validate()?;
            item.restart_delay_ms = timing.restart_delay_ms.unwrap_or(0);
            item.ready_wait_ms = timing.ready_seconds.map(|value| value * 1000);
        }
        item.registration = registration;
        if current["state"] == "Stopped" {
            if action == Action::Stop {
                item.phase = Phase::Skipped;
            } else {
                instance_port_free(&instance(root, name)?)?;
                if action == Action::Restart && !current["run"].is_null() {
                    item.stop_run =
                        Some(serde_json::from_value(current["run"].clone()).map_err(fail)?);
                }
                item.phase = Phase::Queued;
            }
            return Ok(());
        }
        if action == Action::Start {
            return Err("InstanceNotConfirmedStopped".into());
        }
        if current["state"] == "PortOccupied" {
            return Err("PortOccupied".into());
        }
        if !matches!(current["state"].as_str(), Some("Listening" | "Starting")) {
            return Err("RunUnconfirmed".into());
        }
        let run: Run = serde_json::from_value(current["run"].clone()).map_err(fail)?;
        if let Some((_, previous)) = prior.as_ref() {
            if previous.phase.stopping()
                && previous.stop_run.as_ref() == Some(&run)
                && previous.registration == item.registration
            {
                item.phase = Phase::Stopping;
                item.stop_intent.clone_from(&previous.stop_intent);
                item.stop_run = Some(run);
                return Ok(());
            }
        }
        item.stop_run = Some(run);
        item.stop_intent = Some(BatchStopIntent {
            request: item.request.clone(),
            deadline: now() + normal_stop_wait_ms(root, &config),
        });
        item.phase = Phase::StopQueued;
        Ok(())
    };
    if let Err(reason) = prepare(&mut item) {
        if action == Action::Start {
            return Err(reason);
        }
        item.reason = Some(reason);
        item.supersedes = None;
    }
    if action != Action::Stop && item.phase == Phase::Queued {
        if let Some(reason) = queued_start_problem(&instance(root, &item.target.name)?.java) {
            item.phase = Phase::Failed;
            item.reason = Some(reason.into());
        }
    }
    Ok(item)
}

fn stop_snapshot(root: &Path, item: &Item, compare_intent: bool) -> Result<Value> {
    let name = &item.target.name;
    if registration_revision(root, name)? != item.registration {
        return Err("RegistrationChanged".into());
    }
    let current = observed(root, name)?;
    if serde_json::from_value::<Run>(current["run"].clone())
        .ok()
        .as_ref()
        != item.stop_run.as_ref()
        || (compare_intent
            && current["stopToken"].as_str() != item.target.guard.stop_token.as_deref())
    {
        return Err("StopIntentChanged".into());
    }
    if current["state"] == "PortOccupied" {
        return Err("PortOccupied".into());
    }
    Ok(current)
}

fn issue_stop(root: &Path, item: &Item, exe: &Path) -> Result<()> {
    let run = item.stop_run.as_ref().ok_or("RunUnconfirmed")?;
    let intent = item.stop_intent.as_ref().ok_or("StopIntentUnconfirmed")?;
    let _writer = instance_writer_lock(root, &item.target.name)?;
    let _transition = transition_guard(root, &item.target.name)?;
    if now() >= intent.deadline {
        return Err("StopTimeout".into());
    }
    let current = stop_snapshot(root, item, true)?;
    if !matches!(current["state"].as_str(), Some("Listening" | "Starting"))
        || process_image(run.host.pid)
            .and_then(|path| path.canonicalize().ok())
            .as_deref()
            != Some(exe)
        || !alive(&run.host)?
        || !alive(&run.java)?
    {
        return Err("HostUnconfirmed".into());
    }
    host_pipe::submit_batch_stop(run, intent)
}

fn inspect_stop(root: &Path, item: &mut Item, action: Action) -> Result<()> {
    let intent = item.stop_intent.as_ref().ok_or("StopIntentUnconfirmed")?;
    if now() >= intent.deadline {
        return Err("StopTimeout".into());
    }
    let current = stop_snapshot(
        root,
        item,
        action == Action::Restart && !item.suppress_start,
    )?;
    let folder = child_directory(&child_directory(root, "runs")?, &item.target.name)?;
    let acceptance = folder.join("batch-stop-accepted.json");
    if acceptance.exists() {
        no_redirect(&acceptance)?;
        if read::<BatchStopIntent>(&acceptance)? != *intent {
            return Err("StopIntentChanged".into());
        }
        item.phase = Phase::Stopping;
    }
    if current["state"] != "Stopped" {
        return Ok(());
    }

    let _writer = instance_writer_lock(root, &item.target.name)?;
    let _transition = transition_guard(root, &item.target.name)?;
    let current = stop_snapshot(
        root,
        item,
        action == Action::Restart && !item.suppress_start,
    )?;
    if current["state"] != "Stopped" {
        return Err("RunUnconfirmed".into());
    }
    let completion = folder.join("batch-stop-completion.json");
    no_redirect(&completion).map_err(|_| "StopResultUnconfirmed")?;
    let proof: BatchStopCompletion = read(&completion)?;
    let exit_path = folder.join("exit.json");
    no_redirect(&exit_path)?;
    if read::<Option<i32>>(&exit_path)? != Some(0)
        || item.stop_run.as_ref() != Some(&proof.run)
        || proof.intent != *intent
        || proof.finished_at >= intent.deadline
    {
        return Err("StopResultUnconfirmed".into());
    }
    instance_port_free(&instance(root, &item.target.name)?)?;
    let verified_at = now();
    if verified_at >= intent.deadline {
        return Err("StopTimeout".into());
    }
    item.stop_verified_at = Some(verified_at);
    item.phase = if action == Action::Stop {
        Phase::Completed
    } else if item.suppress_start {
        Phase::Cancelled
    } else {
        Phase::Queued
    };
    Ok(())
}

type StopResult = (usize, usize, Result<()>);

fn advance_stops(
    root: &Path,
    directory: &Path,
    state: &Mutex<State>,
    exe: &Path,
    send: &std::sync::mpsc::Sender<StopResult>,
    receive: &std::sync::mpsc::Receiver<StopResult>,
    in_flight: &mut usize,
) -> Result<()> {
    for (batch, index, result) in receive.try_iter() {
        *in_flight -= 1;
        if let Err(error) = result {
            if !matches!(
                error.as_str(),
                "Timeout" | "HostUnavailable" | "StopSubmissionUnconfirmed"
            ) {
                let mut locked = state.lock().map_err(fail)?;
                let mut item = locked.batches[batch].items[index].clone();
                if item.phase == Phase::StopDispatching {
                    item.phase = Phase::StopUnconfirmed;
                    item.reason = Some(error);
                    replace(directory, &mut locked, batch, index, item)?;
                }
            }
        }
    }
    let mut state = state.lock().map_err(fail)?;
    if state.exiting || state.fault {
        return Ok(());
    }
    for batch in 0..state.batches.len() {
        let action = state.batches[batch].action;
        for index in 0..state.batches[batch].items.len() {
            let saved = &state.batches[batch].items[index];
            if !saved.phase.stopping() && !matches!(saved.phase, Phase::Queued | Phase::StopQueued)
            {
                continue;
            }
            let mut item = saved.clone();
            if superseded(&state, &state.batches[batch].id, &item.target.name) {
                if matches!(item.phase, Phase::StopQueued | Phase::Queued) {
                    item.phase = Phase::Cancelled;
                    item.reason = Some("SupersededByStop".into());
                    replace(directory, &mut state, batch, index, item)?;
                    continue;
                }
                item.suppress_start = true;
            }
            if item.phase.stopping() {
                if let Err(reason) = inspect_stop(root, &mut item, action) {
                    if reason != "ManagerBusy" {
                        item.phase = Phase::StopUnconfirmed;
                        item.reason = Some(reason);
                    }
                }
                replace(directory, &mut state, batch, index, item)?;
            } else if item.phase == Phase::StopQueued {
                if item
                    .stop_intent
                    .as_ref()
                    .is_none_or(|intent| now() >= intent.deadline)
                {
                    item.phase = Phase::StopUnconfirmed;
                    item.reason = Some("StopTimeout".into());
                    replace(directory, &mut state, batch, index, item)?;
                } else if *in_flight < 16 {
                    item.phase = Phase::StopDispatching;
                    replace(directory, &mut state, batch, index, item.clone())?;
                    let (root, exe, send) = (root.to_path_buf(), exe.to_path_buf(), send.clone());
                    thread::spawn(move || {
                        let _ = send.send((batch, index, issue_stop(&root, &item, &exe)));
                    });
                    *in_flight += 1;
                }
            }
        }
    }
    Ok(())
}

fn accepted_run(root: &Path, item: &Item) -> Result<Run> {
    let path = child_directory(root, "jobs")?.join(format!("{}.json", item.request));
    no_redirect(&path)?;
    let receipt: Receipt = read(&path)?;
    if !receipt.complete
        || receipt.arguments
            != [
                "start-if-current".to_owned(),
                item.target.name.clone(),
                item.request.clone(),
                String::new(),
                serde_json::to_string(&item.start_guard()).map_err(fail)?,
            ]
    {
        return Err("StartReceiptUnconfirmed".into());
    }
    let run = receipt
        .output
        .and_then(|output| output.get("run").cloned())
        .ok_or("StartReceiptUnconfirmed")?;
    serde_json::from_value(run).map_err(fail)
}

fn inspect(root: &Path, item: &mut Item, trusted_exe: &Path, recovering: bool) -> Result<()> {
    let run = match &item.run {
        Some(run) => run.clone(),
        None => accepted_run(root, item)?,
    };
    let current = observed(root, &item.target.name)?;
    let actual: Run =
        serde_json::from_value(current["run"].clone()).map_err(|_| "RunUnconfirmed")?;
    if actual != run {
        return Err("RunUnconfirmed".into());
    }
    item.run = Some(run.clone());
    if current["state"] == "Stopped" {
        item.phase = Phase::Failed;
        item.reason = Some("ExitedBeforeReady".into());
        return Ok(());
    }
    if !matches!(current["state"].as_str(), Some("Starting" | "Listening"))
        || process_image(run.host.pid)
            .and_then(|image| image.canonicalize().ok())
            .as_deref()
            != Some(trusted_exe)
        || !alive(&run.java)?
    {
        // A newly launched Host may not have opened its control pipe yet.
        if !recovering && item.phase == Phase::Starting && now() < item.deadline {
            return Ok(());
        }
        return Err("HostUnconfirmed".into());
    }
    if let Err(reason) = host_pipe::verify_run(&run) {
        if !recovering && item.phase == Phase::Starting && now() < item.deadline {
            return Ok(());
        }
        return Err(reason);
    }
    if current["state"] == "Listening" && current["consoleReady"] == true {
        item.phase = Phase::Completed;
    } else if now() >= item.deadline {
        return Err("ReadinessUnconfirmed".into());
    } else {
        item.phase = Phase::Starting;
    }
    Ok(())
}

fn replace(
    directory: &Path,
    state: &mut State,
    batch: usize,
    index: usize,
    item: Item,
) -> Result<()> {
    let mut updated = state.batches[batch].clone();
    if serde_json::to_value(&updated.items[index]).map_err(fail)?
        == serde_json::to_value(&item).map_err(fail)?
    {
        return Ok(());
    }
    updated.items[index] = item;
    persist(directory, &updated)?;
    state.batches[batch] = updated;
    Ok(())
}

fn advance(
    root: &Path,
    directory: &Path,
    state: &Mutex<State>,
    exe: &Path,
    recovering: bool,
) -> Result<()> {
    let pending: Vec<_> = state
        .lock()
        .map_err(fail)?
        .batches
        .iter()
        .enumerate()
        .flat_map(|(batch, work)| {
            work.items
                .iter()
                .enumerate()
                .filter(|(_, item)| matches!(item.phase, Phase::Dispatching | Phase::Starting))
                .map(move |(index, item)| (batch, index, item.clone()))
        })
        .collect();
    for (batch, index, mut item) in pending {
        if let Err(reason) = inspect(root, &mut item, exe, recovering) {
            item.phase = Phase::Unconfirmed;
            item.reason = Some(match reason.as_str() {
                "ReadinessUnconfirmed"
                | "HostUnconfirmed"
                | "HostProbeTimeout"
                | "HostPipeUnavailable" => reason,
                _ => "RunUnconfirmed".into(),
            });
        }
        let mut locked = state.lock().map_err(fail)?;
        replace(directory, &mut locked, batch, index, item)?;
    }
    let current_time = now();
    let (batch, index, mut item) = {
        let mut state = state.lock().map_err(fail)?;
        if state.exiting || state.fault {
            return Ok(());
        }
        let items = state.batches.iter().flat_map(|batch| &batch.items);
        if items.clone().any(|item| item.phase == Phase::Unconfirmed)
            || items.filter(|item| item.phase.occupies_slot()).count() >= START_SLOTS
        {
            return Ok(());
        }
        let Some((batch, index, mut item)) =
            state.batches.iter().enumerate().find_map(|(batch, work)| {
                work.items
                    .iter()
                    .position(|item| {
                        item.phase == Phase::Queued
                            && item.stop_verified_at.is_none_or(|verified| {
                                current_time >= verified.saturating_add(item.restart_delay_ms)
                            })
                            && !superseded(&state, &work.id, &item.target.name)
                    })
                    .map(|index| (batch, index, work.items[index].clone()))
            })
        else {
            return Ok(());
        };
        if item.stop_intent.as_ref().is_some_and(|intent| {
            item.stop_verified_at
                .is_none_or(|verified| verified >= intent.deadline)
        }) {
            item.phase = Phase::StopUnconfirmed;
            item.reason = Some("StopResultUnconfirmed".into());
            return replace(directory, &mut state, batch, index, item);
        }
        item.phase = Phase::Dispatching;
        item.deadline = now() + item.ready_wait_ms.unwrap_or(READY_MS);
        replace(directory, &mut state, batch, index, item.clone())?;
        (batch, index, item)
    };
    let command = [
        root.to_string_lossy().into_owned(),
        "start-if-current".into(),
        item.target.name.clone(),
        item.request.clone(),
        serde_json::to_string(&item.start_guard()).map_err(fail)?,
    ];
    match dispatch(&command) {
        Ok(_) => {
            item.run = Some(accepted_run(root, &item)?);
            item.phase = Phase::Starting;
        }
        Err(error) => {
            if global_java::is_unavailable(&error) {
                item.phase = Phase::Failed;
                item.reason = Some(error);
                let mut locked = state.lock().map_err(fail)?;
                return replace(directory, &mut locked, batch, index, item);
            }
            let receipt_exists = root
                .join("jobs")
                .join(format!("{}.json", item.request))
                .exists();
            item.phase = if receipt_exists {
                Phase::Unconfirmed
            } else {
                Phase::Failed
            };
            item.reason = Some(
                if receipt_exists {
                    "DispatchUnconfirmed"
                } else {
                    start_rejection_reason(&error)
                }
                .into(),
            );
        }
    }
    let mut locked = state.lock().map_err(fail)?;
    replace(directory, &mut locked, batch, index, item)
}

/// The batch journal keeps only stable codes, never raw OS text. Codes the GUI can
/// explain on their own pass through; anything else becomes StartRejected.
fn start_rejection_reason(error: &str) -> &'static str {
    let code = error.split(':').next().unwrap_or_default().trim();
    match code {
        "StartIntentChanged" => "StartIntentChanged",
        "JavaExecutableMissing" => "JavaExecutableMissing",
        "JavaExecutableUnreadable" => "JavaExecutableUnreadable",
        "PortOccupied" => "PortOccupied",
        _ => "StartRejected",
    }
}

/// A start that would fail on a missing registered Java fails when the batch is
/// created, so the GUI can report it at once while the other instances start.
fn queued_start_problem(java: &Path) -> Option<&'static str> {
    if java.as_os_str().is_empty() {
        return None;
    }
    registered_java_present(java)
        .err()
        .map(|error| start_rejection_reason(&error))
}

fn auto_restart_one(root: &Path, name: &str) -> Result<bool> {
    let config = instance(root, name)?;
    let Some(policy) = config.auto_restart.as_ref() else { return Ok(false) };
    if config.removed || !policy.valid() || !policy.enabled() || !admitted(root, name)? {
        return Ok(false);
    }
    let _writer = instance_writer_lock(root, name)?;
    let registration = registration_revision(root, name)?;
    let config = instance(root, name)?;
    if config.removed || !admitted(root, name)? { return Ok(false); }
    let Some(policy) = config.auto_restart.as_ref().filter(|policy| policy.valid() && policy.enabled()) else {
        return Ok(false);
    };
    let snapshot = status(root, &config)?;
    if snapshot["state"] != "Stopped" || !snapshot["stopToken"].is_null() || snapshot["run"].is_null() {
        return Ok(false);
    }
    let run: Run = serde_json::from_value(snapshot["run"].clone()).map_err(fail)?;
    let folder = child_directory(&child_directory(root, "runs")?, name)?;
    let clock = now();
    let Some(exit) = auto_restart::eligible_exit(&folder, &run, policy, clock)? else {
        return Ok(false);
    };
    let buffer = config.timing.as_ref().and_then(|timing| timing.restart_delay_ms).unwrap_or(0);
    let Some(attempt) = auto_restart::next_attempt(&folder, &exit, policy, clock, buffer, &registration)? else {
        return Ok(false);
    };
    if instance_port_free(&config).is_err() {
        return Ok(false);
    }
    match claim_runtime_directory(&instance_path(root, name)?) {
        Ok(lease) => drop(lease),
        Err(error) if error == "InstanceDirectoryInUse" => return Ok(false),
        Err(error) => return Err(error),
    }
    let request = content_revision::revision(format!("auto-restart/{name}/{}/{}", run.id, attempt).as_bytes())?;
    if root.join("jobs").join(format!("{request}.json")).exists() {
        auto_restart::pause_after_dispatch(&folder, policy)?;
        return Ok(false);
    }
    let guard = StartGuard {
        previous_run: Some(run.id.clone()), stop_token: None,
        registration: Some(registration), previous_identity: Some(run), auto_attempt: Some(attempt),
        console_window: false,
    };
    lifecycle(root, &config, "start-if-current", &request, None, Some(&guard)).map(|_| true)
}

fn process_auto_restart(root: &Path, state: &Mutex<State>) -> Result<()> {
    {
        let current = state.lock().map_err(fail)?;
        if current.exiting || current.fault || current.batches.iter().any(|batch| active(batch)) {
            return Ok(());
        }
    }
    for item in fs::read_dir(root.join("registry")).map_err(fail)? {
        let path = item.map_err(fail)?.path();
        let Some(name) = path.file_stem().and_then(|name| name.to_str()) else { continue };
        if path.extension().is_some_and(|ext| ext == "json") && instance_name(name) {
            match auto_restart_one(root, name) {
                Ok(true) => break,
                Ok(false) => (),
                Err(error) if matches!(error.as_str(), "ManagerBusy" | "StartIntentChanged") => (),
                Err(error) => eprintln!("Automatic restart failed for {name}: {error}"),
            }
        }
    }
    Ok(())
}

fn recover_deferred_settings(root: &Path) -> Result<()> {
    let mut checked_live_rule = false;
    for entry in fs::read_dir(root.join("registry")).map_err(fail)? {
        let path = entry.map_err(fail)?.path();
        let Some(name) = path.file_stem().and_then(|name| name.to_str()) else { continue };
        if path.extension().is_none_or(|ext| ext != "json") || !instance_name(name) {
            continue;
        }
        let config = instance(root, name)?;
        if config.removed || !admitted(root, name)? {
            continue;
        }
        match status(root, &config)?["state"].as_str() {
            Some("Stopped") => (),
            Some("Listening") if !checked_live_rule => {
                match world_live::recover_pending_rules(root, name) {
                    Ok(checked) => checked_live_rule = checked,
                    Err(error) if error == "ManagerBusy" => (),
                    Err(error) => eprintln!("Deferred world readback for {name}: {error}"),
                }
                continue;
            }
            _ => continue,
        }
        let pending = deferred_settings::has_pending(root, name)?;
        let live_pending = world_live::has_queued(root, name)?;
        if !pending && !live_pending { continue; }
        if live_pending {
            if let Err(error) = world_live::recover_saved(root, name) {
                if error != "ManagerBusy" { eprintln!("Deferred world command for {name}: {error}"); }
            }
        }
        if pending {
            let writer = match instance_writer_lock(root, name) {
                Ok(writer) => writer,
                Err(error) if error == "ManagerBusy" => continue,
                Err(error) => return Err(error),
            };
            if let Err(error) = deferred_settings::apply_locked(root, name) {
                eprintln!("Deferred file settings for {name}: {error}");
            }
            drop(writer);
        }
    }
    Ok(())
}

fn advance_core_batches(root: &Path, state: &Mutex<State>) -> Result<()> {
    let work = {
        let mut state = state.lock().map_err(fail)?;
        if state.exiting || state.fault {
            return Ok(());
        }
        let Ok(journal) = &mut state.core_batches else { return Ok(()) };
        let work = journal.next(root)?;
        if work.is_none() {
            // Candidates of finished, cancelled or interrupted batches (#60).
            if let Err(error) = journal.collect_candidates(root) {
                eprintln!("Core batch candidate cleanup failed: {error}");
            }
        }
        state.core_in_flight = work.is_some();
        work
    };
    if let Some(work) = work {
        let progress = core_batch::progress_reporter(root, &work);
        let outcome = core_batch::perform(root, &work, &progress, || {
            // A durable cancel and entry into core maintenance must share one order.
            let mut state = state.lock().map_err(fail)?;
            let journal = state.core_batches.as_mut().map_err(|_| "CoreBatchJournalUnavailable")?;
            journal.begin_commit(root, &work)
        });
        let mut state = state.lock().map_err(fail)?;
        state.core_in_flight = false;
        let journal = state.core_batches.as_mut().map_err(|_| "CoreBatchJournalUnavailable")?;
        journal.finish(root, &work, outcome)?;
    }
    Ok(())
}

pub(super) fn run(root: &Path) -> Result<Value> {
    let _lease = lock_file(&root.join("resident-manager.lock"))?;
    let directory = root.join("batches");
    if !directory.exists() {
        fs::create_dir(&directory).map_err(fail)?;
    }
    let directory = child_directory(root, "batches")?;
    let exe = executable()?;
    let mut peers = vec![exe.clone()];
    if let Ok(gui) = exe.with_file_name(crate::desktop_channel::GUI_EXECUTABLE).canonicalize() {
        peers.push(gui);
    }
    if let Ok(gui) = exe
        .with_file_name("instanceharbor-acceptance.exe")
        .canonicalize()
    {
        peers.push(gui);
    }
    let state = Arc::new(Mutex::new(State {
        batches: load(&directory)?,
        core_batches: core_batch::Journal::open(root),
        core_in_flight: false,
        exiting: false,
        fault: false,
        gui: Vec::new(),
    }));
    let identity = identity(std::process::id())
        .map_err(fail)?
        .ok_or("ManagerIdentityUnavailable")?;
    let descriptor = metadata(root);
    if descriptor.exists() {
        no_redirect(&descriptor)?;
    }
    let name = endpoint(root)?;
    let (send, receive) = std::sync::mpsc::channel();
    let (ready_send, ready_receive) = std::sync::mpsc::sync_channel(1);
    let server_state = Arc::clone(&state);
    let server_root = root.to_path_buf();
    let server_directory = directory.clone();
    thread::spawn(move || {
        let result = host_pipe::serve_local(
            &name,
            &peers,
            move || {
                save(&descriptor, &identity).map_err(io::Error::other)?;
                ready_send.send(()).map_err(io::Error::other)
            },
            |request| {
                let response = match reply(&server_root, &server_directory, &server_state, request)
                {
                    Ok(value) => json!({"result":value}),
                    Err(error) => json!({"error":error}),
                }
                .to_string();
                if response.len() + 1 > host_pipe::MANAGER_FRAME_LIMIT {
                    r#"{"error":"ManagerFrameTooLarge"}"#.into()
                } else {
                    response
                }
            },
        );
        let _ = send.send(result);
    });
    if ready_receive.recv_timeout(Duration::from_secs(3)).is_err() {
        return Err("ManagerPipeUnavailable".into());
    }
    // Dropped on every return below, which removes the icon before the process ends.
    #[cfg(not(test))]
    let _tray = {
        let (count_root, check_root) = (root.to_path_buf(), root.to_path_buf());
        let (check_state, exit_state, exiting_state) = (Arc::clone(&state), Arc::clone(&state), Arc::clone(&state));
        manager_tray::start(
            root,
            exe.with_file_name(crate::desktop_channel::GUI_EXECUTABLE),
            manager_tray::Hooks {
                running: Box::new(move || {
                    let snapshot = instance_snapshot::read(&count_root, false)?;
                    Ok(running_servers(&snapshot))
                }),
                check_exit: Box::new(move || {
                    if active_work(&*check_state.lock().map_err(fail)?) {
                        return Ok(manager_tray::ExitCheck::Busy);
                    }
                    // Unreadable states are treated as possibly running: ask instead of quitting.
                    let Ok(snapshot) = instance_snapshot::read(&check_root, false) else {
                        return Ok(manager_tray::ExitCheck::Confirm(None));
                    };
                    Ok(match servers_not_stopped(&snapshot, |name| restarts_automatically(&check_root, name)) {
                        (0, _) => manager_tray::ExitCheck::Clear,
                        servers => manager_tray::ExitCheck::Confirm(Some(servers)),
                    })
                }),
                request_exit: Box::new(move || exit_app(&exit_state)),
                exiting: Box::new(move || exiting_state.lock().map_or(true, |state| state.exiting)),
            },
        )
    };
    let prefetch_root = root.to_path_buf();
    thread::spawn(move || core_prefetch::supervise(&prefetch_root));
    let management_root = root.to_path_buf();
    thread::spawn(move || {
        let mut cursor = 0usize;
        let mut last_error: Option<String> = None;
        loop {
            match batch_import::sync_management_folders(&management_root, cursor) {
                Err(error) if error != "ManagerBusy" => {
                    if last_error.as_deref() != Some(error.as_str()) {
                        eprintln!("Management folder scan: {error}");
                        last_error = Some(error);
                    }
                }
                Ok(()) => last_error = None,
                Err(_) => {}
            }
            cursor = cursor.wrapping_add(1);
            thread::sleep(Duration::from_secs(10));
        }
    });
    let core_state = Arc::clone(&state);
    let core_root = root.to_path_buf();
    thread::spawn(move || loop {
        if let Err(error) = advance_core_batches(&core_root, &core_state) {
            eprintln!("Core batch scheduling stopped: {error}");
            if let Ok(mut state) = core_state.lock() {
                state.core_batches = Err(error);
            }
            return;
        }
        thread::sleep(Duration::from_millis(250));
    });
    let stop_state = Arc::clone(&state);
    let (stop_root, stop_directory, stop_exe) =
        (root.to_path_buf(), directory.clone(), exe.clone());
    thread::spawn(move || {
        let (send, receive) = std::sync::mpsc::channel();
        let mut in_flight = 0;
        loop {
            if let Err(error) = advance_stops(
                &stop_root,
                &stop_directory,
                &stop_state,
                &stop_exe,
                &send,
                &receive,
                &mut in_flight,
            ) {
                if let Ok(mut state) = stop_state.lock() {
                    state.fault = true;
                }
                eprintln!("Batch stop scheduling stopped: {error}");
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
    });
    let mut recovering = true;
    let mut next_auto_restart = Instant::now();
    let mut next_deferred = Instant::now();
    loop {
        if let Ok(result) = receive.try_recv() {
            return Err(result
                .err()
                .map(fail)
                .unwrap_or_else(|| "ManagerPipeClosed".into()));
        }
        {
            let state = state.lock().map_err(fail)?;
            if state.exiting {
                // Give the bounded pipe reply time to reach its caller.
                drop(state);
                thread::sleep(Duration::from_millis(500));
                return Ok(json!({"exited":true}));
            }
        }
        if !state.lock().map_err(fail)?.fault {
            if let Err(error) = advance(root, &directory, &state, &exe, recovering) {
                state.lock().map_err(fail)?.fault = true;
                eprintln!("Batch scheduling stopped: {error}");
            }
        }
        if !recovering && Instant::now() >= next_auto_restart {
            if let Err(error) = process_auto_restart(root, &state) {
                eprintln!("Automatic restart check failed: {error}");
            }
            next_auto_restart = Instant::now() + Duration::from_secs(1);
        }
        if Instant::now() >= next_deferred {
            if let Err(error) = recover_deferred_settings(root) {
                eprintln!("Deferred settings sweep failed: {error}");
            }
            next_deferred = Instant::now() + Duration::from_secs(1);
        }
        recovering = false;
        thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod start_rejection_tests {
    use super::{queued_start_problem, start_rejection_reason};

    #[test]
    fn batch_start_keeps_explainable_codes_and_drops_raw_text() {
        assert_eq!(start_rejection_reason("JavaExecutableMissing"), "JavaExecutableMissing");
        assert_eq!(
            start_rejection_reason(r"JavaExecutableUnreadable: Access is denied. (os error 5) C:\Users\someone"),
            "JavaExecutableUnreadable"
        );
        assert_eq!(start_rejection_reason("PortOccupied"), "PortOccupied");
        assert_eq!(start_rejection_reason("StartIntentChanged"), "StartIntentChanged");
        assert_eq!(start_rejection_reason("系統找不到指定的路徑。 (os error 3)"), "StartRejected");
    }

    #[test]
    fn batch_start_checks_the_registered_java_before_queueing() {
        let parent = std::env::temp_dir().join(format!("ih-batch-java-{}", std::process::id()));
        std::fs::create_dir_all(&parent).unwrap();
        let java = parent.join("java.exe");
        assert_eq!(queued_start_problem(&java), Some("JavaExecutableMissing"));
        std::fs::write(&java, b"").unwrap();
        assert_eq!(queued_start_problem(&java), None);
        // An empty path means the instance inherits Java; that is checked at dispatch.
        assert_eq!(queued_start_problem(std::path::Path::new("")), None);
        std::fs::remove_dir_all(&parent).unwrap();
    }
}

#[cfg(test)]
mod tray_count_tests {
    use super::{running_servers, servers_not_stopped};
    use serde_json::json;

    #[test]
    fn tray_counts_only_servers_the_list_can_stop() {
        let snapshot = json!({"rows":[
            {"state":"Listening","run":{"id":"a"}},
            {"state":"Starting","run":{"id":"b"}},
            {"state":"Listening","run":null},
            {"state":"Stopped","run":{"id":"c"}},
        ]});
        assert_eq!(running_servers(&snapshot), 2);
        assert_eq!(running_servers(&json!({})), 0);
    }

    #[test]
    fn quit_counts_every_server_not_known_to_be_stopped() {
        // Anything short of a confirmed stop counts, including a run still being decided.
        let snapshot = json!({"rows":[
            {"name":"guarded","state":"Listening","run":{"id":"a"}},
            {"name":"launching","state":"Starting","run":null},
            {"name":"undecided","state":"RunUnconfirmed","run":null},
            {"name":"occupied","state":"PortOccupied","run":null},
            {"name":"unreadable"},
            {"name":"stopped","state":"Stopped","run":{"id":"d"}},
        ]});
        let policy = |name: &str| matches!(name, "guarded" | "undecided" | "stopped");
        assert_eq!(servers_not_stopped(&snapshot, policy), (5, 2));
        assert_eq!(servers_not_stopped(&json!({"rows":[{"state":"Stopped"}]}), |_| true), (0, 0));
        assert_eq!(servers_not_stopped(&json!({}), |_| true), (0, 0));
    }
}

#[cfg(test)]
mod gui_identity_tests {
    use super::*;

    fn gui(pid: u32, created: u64, image: &str) -> AttachedGui {
        AttachedGui { process: Identity { pid, created }, image: PathBuf::from(image) }
    }

    #[test]
    fn quit_closes_only_guis_that_are_still_the_attached_process() {
        let exe = r"X:\InstanceHarbor\instanceharbor.exe";
        let recorded = [gui(10, 100, exe), gui(11, 200, exe), gui(12, 300, exe), gui(13, 400, exe), gui(14, 500, exe)];
        let probe = |pid: u32| -> GuiProbe {
            match pid {
                // Still the attached GUI.
                10 => (Ok(Some(Identity { pid, created: 100 })), Some(PathBuf::from(exe))),
                // The PID was reused by a later process of the same executable name.
                11 => (Ok(Some(Identity { pid, created: 999 })), Some(PathBuf::from(exe))),
                // Same creation time reported, but another executable now owns the PID.
                12 => (Ok(Some(Identity { pid, created: 300 })), Some(PathBuf::from(r"X:\Other\editor.exe"))),
                // The GUI has ended.
                13 => (Ok(None), None),
                // Access denied: the PID now belongs to a process of another user or level.
                _ => (Err(io::Error::from_raw_os_error(5)), None),
            }
        };
        assert_eq!(verified_guis(&recorded, probe), vec![10]);
    }

    #[test]
    fn quit_sends_nothing_when_no_gui_is_verified() {
        let recorded = [gui(20, 1, r"X:\InstanceHarbor\instanceharbor.exe")];
        assert!(verified_guis(&recorded, |_| (Ok(None), None)).is_empty());
        assert!(verified_guis(&[], |_| unreachable!("no GUI is probed")).is_empty());
    }
}
