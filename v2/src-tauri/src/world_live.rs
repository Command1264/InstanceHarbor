//! Explicit live world changes bound to a verified managed run.
use super::*;
use std::{
    io::{Seek, SeekFrom},
    process::ChildStdin,
    sync::mpsc::{self, Receiver, SyncSender},
};

#[path = "world_live/first_ready.rs"]
pub(super) mod first_ready;
#[path = "world_live/queued.rs"]
pub(super) mod queued;

pub(super) const RULE_KEYS: &[(&str, &str, &str)] = &[
    ("gamerules.advance_time", "doDaylightCycle", "advance_time"),
    ("gamerules.spawn_mobs", "doMobSpawning", "spawn_mobs"),
    (
        "gamerules.command_block_output",
        "commandBlockOutput",
        "command_block_output",
    ),
    (
        "gamerules.max_entity_cramming",
        "maxEntityCramming",
        "max_entity_cramming",
    ),
    (
        "gamerules.allow_entering_nether_using_portals",
        "",
        "allow_entering_nether_using_portals",
    ),
    ("serverProperties.spawn-monsters", "", "spawn_monsters"),
    (
        "serverProperties.enable-command-block",
        "",
        "command_blocks_work",
    ),
    ("serverProperties.pvp", "", "pvp"),
];
const TIME: &str = "initialTime";
const FLOW_RULE: &str = "gamerules.advance_time";
const PLAN_LIFETIME: u64 = 120_000;
const REPLY_LIMIT: u64 = 256 * 1024;

pub(super) struct Command {
    line: String,
    deadline: Instant,
    reply: SyncSender<&'static str>,
}

fn integer(text: &str, max: u32) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse::<u32>().ok().filter(|number| *number <= max)
}

fn rule_value(name: &str, value: &str) -> bool {
    if matches!(name, "maxEntityCramming" | "max_entity_cramming") {
        integer(value, 9999).is_some()
    } else {
        matches!(value, "true" | "false")
    }
}

fn allowed_line(line: &str) -> bool {
    let fields: Vec<_> = line.split(' ').collect();
    match fields.as_slice() {
        ["time", "query", "time" | "daytime"] => true,
        ["time", "set", tick] => integer(tick, 23999).is_some(),
        ["gamerule", name] | ["gamerule", name, _] => {
            let key = name.strip_prefix("minecraft:").unwrap_or(name);
            RULE_KEYS.iter().any(|(_, legacy, modern)| {
                (*modern == key || (*legacy == key && name == &key && !legacy.is_empty()))
                    && fields.get(2).is_none_or(|value| rule_value(key, value))
            })
        }
        _ => false,
    }
}

pub(super) fn submit(send: &SyncSender<Command>, line: &str) -> &'static str {
    if !allowed_line(line) {
        return "InvalidWorldCommand";
    }
    let (reply, result) = mpsc::sync_channel(1);
    let wait = Duration::from_millis(235);
    let command = Command {
        line: format!("{line}\n"),
        deadline: Instant::now() + wait,
        reply,
    };
    if send.try_send(command).is_err() {
        return "WorldCommandUnavailable";
    }
    result
        .recv_timeout(wait)
        .unwrap_or("WorldCommandUnconfirmed")
}

#[derive(Default)]
pub(super) struct ConsoleWindow(Option<(native_console::ManagedInput, Instant)>);

impl ConsoleWindow {
    fn expire(&mut self) {
        if self.0.as_ref().is_some_and(|(_, expiry)| Instant::now() >= *expiry) {
            self.0 = None;
        }
    }

    fn write(&mut self, console: &native_console::Console, line: &str, deadline: Instant) -> &'static str {
        let until = Instant::now() + Duration::from_secs(6);
        let (input, expires) = match self.0.as_mut() {
            Some(prior) => prior,
            None => {
                let Ok(input) = console.begin_managed_input() else {
                    return "WorldConsoleInputNotVerified";
                };
                self.0.insert((input, until))
            }
        };
        *expires = until;
        match input.write_command(line.trim_end_matches('\n'), deadline) {
            Ok(()) => "WorldCommandWritten",
            // The user is typing a line in the console; nothing was written.
            Err(error) if error == "ConsoleManagedInputBusy" => "WorldConsoleInputBusy",
            Err(_) => "WorldCommandUnconfirmed",
        }
    }
}

pub(super) fn deliver(
    receiver: &Receiver<Command>,
    ready: bool,
    stdin: Option<&mut ChildStdin>,
    console: Option<(&native_console::Console, &mut ConsoleWindow)>,
) {
    let mut console = console;
    if let Some((_, window)) = console.as_mut() {
        window.expire();
    }
    let Ok(command) = receiver.try_recv() else {
        return;
    };
    if Instant::now() >= command.deadline {
        return;
    }
    let reply = if ready {
        match stdin {
            Some(stdin) => match stdin.write_all(command.line.as_bytes()) {
                Ok(()) => "WorldCommandWritten",
                Err(_) => "WorldCommandUnconfirmed",
            },
            None => console.map_or("WorldConsoleUnavailable", |(console, window)|
                window.write(console, &command.line, command.deadline)),
        }
    } else {
        "WorldRunNotReady"
    };
    // The caller may have disconnected. It must consult the journal of any mutation.
    let _ = command.reply.send(reply);
}

struct Target {
    name: String,
    run: Run,
    output: PathBuf,
    registration: String,
    profile: CoreProfile,
}

impl Target {
    fn get(root: &Path, name: &str, run: &str) -> Result<Self> {
        if !token(name) || !token(run) || !admitted(root, name)? {
            return Err("WorldRunUnavailable".into());
        }
        let config = instance(root, name)?;
        let run = ready_run(root, &config, run)?;
        let (directory, profile) = queued::verified(root, &config)?;
        let logs = child_directory(&directory, "logs")?;
        Ok(Self {
            name: name.into(),
            run,
            output: logs.join("latest.log"),
            registration: registration_revision(root, name)?,
            profile,
        })
    }

    fn field(&self, key: &str) -> Result<Field> {
        Self::field_for(&self.profile.minecraft, key)
    }

    fn field_for(version: &str, key: &str) -> Result<Field> {
        let modern_core = version.starts_with("26.");
        if key == TIME {
            return Ok(Field::Time {
                modern: modern_core,
            });
        }
        let (logical, legacy, modern) = RULE_KEYS
            .iter()
            .copied()
            .find(|(logical, _, _)| *logical == key)
            .ok_or("WorldFieldUnavailable")?;
        if logical == "gamerules.max_entity_cramming" && version == "1.8.8" {
            return Err("WorldFieldUnavailable".into());
        }
        let name = if modern_core && !modern.is_empty() {
            format!("minecraft:{modern}")
        } else if !modern_core && !legacy.is_empty() {
            legacy.to_owned()
        } else {
            return Err("WorldFieldUnavailable".into());
        };
        Ok(Field::Rule {
            name,
            suffix: if modern_core { modern } else { legacy },
        })
    }

    fn assert_same_run(&self, root: &Path) -> Result<()> {
        if ready_run(root, &instance(root, &self.name)?, &self.run.id)? != self.run
            || registration_revision(root, &self.name)? != self.registration
        {
            return Err("WorldRunChanged".into());
        }
        Ok(())
    }

    fn command(
        &self,
        root: &Path,
        line: &str,
        parse: impl Fn(&str) -> Option<String>,
    ) -> Result<String> {
        let mut log = RunOutput::open(&self.output)?;
        self.assert_same_run(root)?;
        host_pipe::request_world_command(root, &self.name, &self.run, line)?;
        let expiry = Instant::now() + Duration::from_secs(5);
        while Instant::now() < expiry {
            self.assert_same_run(root)?;
            if let Some(reply) = log.reply(&parse)? {
                self.assert_same_run(root)?;
                return Ok(reply);
            }
            thread::sleep(Duration::from_millis(75));
        }
        Err("WorldReplyUnconfirmed".into())
    }

    fn query(&self, root: &Path, field: &Field) -> Result<String> {
        self.command(root, &field.query_line(), |text| field.query_reply(text))
    }

    fn set(&self, root: &Path, field: &Field, value: &str) -> Result<()> {
        self.command(root, &field.set_line(value), |text| {
            field.set_reply(text, value).then(String::new)
        })
        .map(|_| ())
    }

    fn clock_flow(&self, root: &Path, field: &Field) -> Result<Option<bool>> {
        if !field.is_time() {
            return Ok(None);
        }
        let flow = self.query(root, &self.field(FLOW_RULE)?)?;
        match flow.as_str() {
            "true" => Ok(Some(true)),
            "false" => Ok(Some(false)),
            _ => Err("WorldClockFlowUnconfirmed".into()),
        }
    }
}

fn ready_run(root: &Path, config: &Instance, run: &str) -> Result<Run> {
    if config.removed {
        return Err("WorldRunUnavailable".into());
    }
    let observation = status(root, config)?;
    if observation["state"] != "Listening"
        || observation["consoleReady"] != true
        || !observation["stopPhase"].is_null()
        || !observation["stopToken"].is_null()
    {
        return Err("WorldRunNotReady".into());
    }
    let current: Run = serde_json::from_value(observation["run"].clone()).map_err(fail)?;
    if current.id != run {
        return Err("WorldRunChanged".into());
    }
    Ok(current)
}

struct RunOutput {
    file: File,
    path: PathBuf,
    offset: u64,
    anchor: Vec<u8>,
}

impl RunOutput {
    fn open(path: &Path) -> Result<Self> {
        no_redirect(path)?;
        let mut file = File::open(path).map_err(fail)?;
        let offset = file.metadata().map_err(fail)?.len();
        let mut anchor = vec![0; offset.min(256) as usize];
        file.seek(SeekFrom::Start(offset - anchor.len() as u64))
            .map_err(fail)?;
        file.read_exact(&mut anchor).map_err(fail)?;
        Ok(Self {
            file,
            path: path.to_owned(),
            offset,
            anchor,
        })
    }

    fn reply(&mut self, parse: &impl Fn(&str) -> Option<String>) -> Result<Option<String>> {
        // Never accept bytes from a replacement log. The read handle also retains
        // the original object if Windows allows a concurrent rename.
        use std::os::windows::fs::MetadataExt;
        no_redirect(&self.path)?;
        let current = fs::metadata(&self.path).map_err(fail)?;
        let original = self.file.metadata().map_err(fail)?;
        if current.creation_time() != original.creation_time() {
            return Err("WorldOutputChanged".into());
        }
        let length = original.len();
        if length < self.offset || length - self.offset > REPLY_LIMIT {
            return Err("WorldOutputChanged".into());
        }
        self.file
            .seek(SeekFrom::Start(self.offset - self.anchor.len() as u64))
            .map_err(fail)?;
        let mut current = vec![0; self.anchor.len()];
        self.file.read_exact(&mut current).map_err(fail)?;
        if current != self.anchor {
            return Err("WorldOutputChanged".into());
        }
        let mut suffix = Vec::new();
        (&mut self.file)
            .take(REPLY_LIMIT + 1)
            .read_to_end(&mut suffix)
            .map_err(fail)?;
        if suffix.len() as u64 > REPLY_LIMIT {
            return Err("WorldOutputChanged".into());
        }
        let mut lines = suffix.split_inclusive(|byte| *byte == b'\n');
        if self.anchor.last().is_some_and(|byte| *byte != b'\n') {
            // A line that began before the request cannot be its reply.
            lines.next();
        }
        let mut matches = lines
            .filter(|line| line.ends_with(b"\n"))
            .filter_map(|line| std::str::from_utf8(line).ok())
            .filter_map(log_message)
            .filter_map(parse);
        let value = matches.next();
        if matches.next().is_some() {
            return Err("WorldReplyAmbiguous".into());
        }
        Ok(value)
    }
}

fn log_message(line: &str) -> Option<&str> {
    let (prefix, text) = line.split_once("]: ")?;
    let (_, thread) = prefix.strip_prefix('[')?.split_once("] [")?;
    let thread = thread.strip_suffix("/INFO")?;
    if thread != "Server thread" && !thread.starts_with("Folia Region Scheduler Thread #") {
        return None;
    }
    let text = text.trim_end_matches(['\r', '\n']);
    Some(text.strip_prefix("System chat: ").unwrap_or(text))
}

enum Field {
    Rule { name: String, suffix: &'static str },
    Time { modern: bool },
}

impl Field {
    fn is_time(&self) -> bool {
        matches!(self, Self::Time { .. })
    }

    fn valid(&self, value: &str) -> bool {
        match self {
            Self::Time { .. } => integer(value, 23999).is_some(),
            Self::Rule { suffix, .. } => rule_value(suffix, value),
        }
    }

    fn query_line(&self) -> String {
        match self {
            Self::Rule { name, .. } => format!("gamerule {name}"),
            Self::Time { modern: true } => "time query time".into(),
            Self::Time { modern: false } => "time query daytime".into(),
        }
    }

    fn set_line(&self, value: &str) -> String {
        match self {
            Self::Rule { name, .. } => format!("gamerule {name} {value}"),
            Self::Time { .. } => format!("time set {value}"),
        }
    }

    fn query_reply(&self, text: &str) -> Option<String> {
        match self {
            Self::Rule { name, suffix } => {
                let value = rule_reply(text, name, suffix, false)?;
                self.valid(value).then(|| value.to_owned())
            }
            Self::Time { modern: true } => text
                .strip_prefix("Clock minecraft:overworld is at ")?
                .strip_suffix(" tick(s)")
                .and_then(canonical_ticks),
            Self::Time { modern: false } => text
                .strip_prefix("The time is ")
                .or_else(|| text.strip_prefix("Time is "))
                .and_then(canonical_ticks),
        }
    }

    fn set_reply(&self, text: &str, target: &str) -> bool {
        match self {
            Self::Rule { name, suffix } => {
                let legacy =
                    name == suffix && text.trim_end_matches('.') == "Game rule has been updated";
                let modern = rule_reply(text, name, suffix, true) == Some(target);
                legacy || modern
            }
            Self::Time { modern: true } => [
                "Set minecraft:overworld to ",
                "Clock minecraft:overworld is already set to ",
            ]
            .iter()
            .any(|prefix| {
                text.strip_prefix(prefix)
                    .and_then(|rest| rest.strip_suffix(" tick(s)"))
                    == Some(target)
            }),
            Self::Time { modern: false } => text.strip_prefix("Set the time to ") == Some(target),
        }
    }
}

fn matches_readback(actual: &str, expected: &str, moving_clock: bool) -> bool {
    if !moving_clock {
        return actual == expected;
    }
    let (Ok(actual), Ok(expected)) = (actual.parse::<u64>(), expected.parse::<u64>()) else {
        return false;
    };
    // Match forward ticks within the readback window, including a day rollover.
    (24_000 + actual % 24_000 - expected % 24_000) % 24_000 <= 140
}

fn canonical_ticks(value: &str) -> Option<String> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse::<u64>().ok().map(|tick| tick.to_string())
}

fn rule_reply<'a>(text: &'a str, name: &str, suffix: &str, setter: bool) -> Option<&'a str> {
    let payload = text
        .strip_prefix("Game rule ")
        .or_else(|| text.strip_prefix("Gamerule "));
    if let Some(payload) = payload {
        let value = [name, suffix].into_iter().find_map(|expected| {
            payload
                .strip_prefix(expected)
                .and_then(|rest| rest.strip_prefix(' '))
        })?;
        let marker = if setter {
            "is now set to"
        } else {
            "is currently set to"
        };
        return value
            .strip_prefix(marker)
            .and_then(|rest| rest.strip_prefix(": ").or_else(|| rest.strip_prefix(' ')));
    }
    if setter {
        return None;
    }
    text.strip_prefix(name)?.strip_prefix(" = ")
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Plan {
    run: String,
    key: String,
    target: String,
    before: String,
    flowing: Option<bool>,
    registration: String,
    sha256: String,
    created_at: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Journal {
    name: String,
    request: String,
    plan: Plan,
    state: String,
    before_at_apply: String,
    observed: Option<String>,
    error: Option<String>,
}

fn journal_directory(root: &Path, name: &str) -> Result<PathBuf> {
    if !token(name) {
        return Err("WorldRunUnavailable".into());
    }
    Ok(child_directory(root, "history")?.join(format!("world-live-{name}")))
}

fn optional_receipt(path: &Path) -> Result<Option<Journal>> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            no_redirect(path)?;
            let mut bytes = Vec::new();
            File::open(path)
                .map_err(fail)?
                .take(16 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(fail)?;
            if bytes.len() > 16 * 1024 {
                return Err("WorldJournalTooLarge".into());
            }
            serde_json::from_slice(&bytes).map(Some).map_err(fail)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(fail(error)),
    }
}

fn receipt(root: &Path, name: &str, request: &str) -> Result<Journal> {
    if !token(request) {
        return Err("InvalidWorldRequest".into());
    }
    let directory = journal_directory(root, name)?;
    no_redirect(&directory)?;
    let latest = optional_receipt(&directory.join(format!("request-{request}.json")))?;
    let journal = match latest {
        Some(journal) => journal,
        None => optional_receipt(&directory.join("pending.json"))?.ok_or("UnknownWorldRequest")?,
    };
    if journal.name != name || journal.request != request {
        return Err("UnknownWorldRequest".into());
    }
    Ok(journal)
}

fn confirm_saved(root: &Path, name: &str, request: &str) -> Result<Journal> {
    let initial = receipt(root, name, request)?;
    if initial.state != "AwaitingNormalStop" {
        return Ok(initial);
    }
    let _writer = instance_writer_lock(root, name)?;
    confirm_saved_locked(root, name, request)
}

fn confirm_saved_locked(root: &Path, name: &str, request: &str) -> Result<Journal> {
    let mut journal = receipt(root, name, request)?;
    let config = instance(root, name)?;
    let directory = instance_path(root, name)?;
    let snapshot = status(root, &config)?;
    if journal.state != "AwaitingNormalStop" {
        return Ok(journal);
    }
    let path = journal_directory(root, name)?.join(format!("request-{request}.json"));
    let uncertain = |mut journal: Journal, reason: &str| -> Result<Journal> {
        journal.state = "Unconfirmed".into();
        journal.error = Some(reason.into());
        core_maintenance::write_durable(&path, &journal, true)?;
        Ok(journal)
    };
    if !admitted(root, name)?
        || config.removed
        || snapshot["run"]["id"] != journal.plan.run
        || registration_revision(root, name)? != journal.plan.registration
    {
        return uncertain(journal, "WorldPersistenceRunChanged");
    }
    if snapshot["state"] != "Stopped" {
        return Ok(journal);
    }
    if snapshot["stopPhase"] != "Sent" {
        return uncertain(journal, "WorldPersistenceStopUnconfirmed");
    }
    if journal.plan.flowing == Some(true) {
        return uncertain(journal, "WorldPersistenceClockMoving");
    }
    let _lease = claim_runtime_directory(&directory)?;
    let run_directory = child_directory(&child_directory(root, "runs")?, name)?;
    let release = run_directory.join("run-release.json");
    if !release.is_file() {
        return uncertain(journal, "WorldPersistenceStopUnconfirmed");
    }
    no_redirect(&release)?;
    let settled: Value = read(&release)?;
    if settled["run"] != snapshot["run"] || settled["exit"] != 0 {
        return uncertain(journal, "WorldPersistenceStopUnconfirmed");
    }
    let core = match queued::verified(root, &config) {
        Ok((_, core)) if core.sha256 == journal.plan.sha256 => core,
        Ok(_) => return uncertain(journal, "WorldPersistenceCoreChanged"),
        Err(error) if matches!(error.as_str(), "WorldProfileUnverified" | "CoreIdentityChanged") => {
            return uncertain(journal, "WorldPersistenceCoreChanged");
        }
        Err(error) => return Err(error),
    };
    let saved = prepare_diagnostics::observe_saved_world(&directory, &core, &[&journal.plan.key]);
    if saved.get(&journal.plan.key) != Some(&journal.plan.target) {
        return uncertain(journal, "WorldPersistedValueUnconfirmed");
    }
    journal.state = "Saved".into();
    core_maintenance::write_durable(&path, &journal, true)?;
    Ok(journal)
}

pub(super) fn activity(root: &Path, name: &str) -> Result<Vec<Value>> {
    let mut rows = live_activity(root, name)?;
    for row in &mut rows {
        if row["state"] == "AwaitingNormalStop" {
            row["state"] = json!("VerifiedLive");
            row["message"] = json!("正常停止後自動核對存檔");
        }
    }
    rows.extend(queued::activity(root, name)?);
    Ok(rows)
}

fn live_activity(root: &Path, name: &str) -> Result<Vec<Value>> {
    let path = journal_directory(root, name)?;
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(fail(error)),
        Ok(_) => no_redirect(&path)?,
    }
    let mut entries = std::collections::BTreeMap::new();
    if let Some(pending) = optional_receipt(&path.join("pending.json"))? {
        if pending.name != name || !token(&pending.request) {
            return Err("InvalidWorldJournal".into());
        }
        entries.insert(pending.request.clone(), activity_row(&pending));
    }
    for entry in fs::read_dir(path).map_err(fail)? {
        let entry = entry.map_err(fail)?;
        let filename = entry.file_name();
        let request = filename
            .to_str()
            .and_then(|filename| filename.strip_prefix("request-")?.strip_suffix(".json"));
        let Some(request) = request.filter(|request| token(request)) else {
            continue;
        };
        let journal = optional_receipt(&entry.path())?.ok_or("InvalidWorldJournal")?;
        if journal.name != name || journal.request != request {
            return Err("InvalidWorldJournal".into());
        }
        entries.insert(journal.request.clone(), activity_row(&journal));
    }
    Ok(entries.into_values().collect())
}

pub(super) fn recover_saved(root: &Path, name: &str) -> Result<()> {
    let _writer = instance_writer_lock(root, name)?;
    recover_saved_locked(root, name)
}

pub(super) fn recover_pending_rules(root: &Path, name: &str) -> Result<bool> {
    queued::recover_live_rules(root, name)
}

pub(super) fn has_queued(root: &Path, name: &str) -> Result<bool> {
    Ok(queued::needs_reconcile(root, name)?
        || live_activity(root, name)?.iter().any(|row| row["state"] == "AwaitingNormalStop"))
}

pub(super) fn recover_saved_locked(root: &Path, name: &str) -> Result<()> {
    queued::reconcile(root, name)?;
    for row in live_activity(root, name)? {
        if row["state"] == "AwaitingNormalStop" {
            let request = row["id"].as_str().ok_or("InvalidWorldJournal")?;
            confirm_saved_locked(root, name, request)?;
        }
    }
    Ok(())
}

fn activity_row(journal: &Journal) -> Value {
    let state = match journal.state.as_str() {
        "NoChange" | "Saved" => "Completed",
        "Applied" => "VerifiedLive",
        "AwaitingNormalStop" => "AwaitingNormalStop",
        // A process can exit between writing Sending and updating the receipt.
        _ => "Unconfirmed",
    };
    let action = if journal.plan.key == TIME {
        "world-live-time"
    } else {
        "world-live-rule"
    };
    let mut row = json!({
        "id": journal.request,
        "action": action,
        "instance": journal.name,
        "createdAt": journal.plan.created_at,
        "state": state,
    });
    if journal.state == "Saved" {
        row["message"] = json!("已核對世界存檔");
    } else if journal.state == "Unconfirmed" && journal.observed.is_some() {
        row["message"] = json!("已核對生效，存檔結果未確認");
    }
    row
}

fn current_clock(root: &Path, name: &str, run: &str) -> Result<Value> {
    if !token(name) || !token(run) {
        return Err("WorldRunUnavailable".into());
    }
    let _lock = instance_writer_lock(root, name)?;
    let target = Target::get(root, name, run)?;
    let field = target.field(TIME)?;
    let ticks = target
        .query(root, &field)?
        .parse::<u64>()
        .map_err(|_| "InvalidWorldClock")?;
    let value = (ticks % 24_000).to_string();
    Ok(json!({"run": target.run.id, "value": value}))
}

fn preview(root: &Path, name: &str, run: &str, key: &str, value: &str) -> Result<Value> {
    if !token(name) {
        return Err("WorldRunUnavailable".into());
    }
    let _lock = instance_writer_lock(root, name)?;
    let target = Target::get(root, name, run)?;
    let field = target.field(key)?;
    if !field.valid(value) {
        return Err("InvalidWorldValue".into());
    }
    let canonical =
        integer(value, 23999).map_or_else(|| value.to_owned(), |number| number.to_string());
    let before = target.query(root, &field)?;
    let flowing = target.clock_flow(root, &field)?;
    Ok(json!(Plan {
        run: target.run.id,
        key: key.to_owned(),
        target: canonical,
        before,
        flowing,
        registration: target.registration,
        sha256: target.profile.sha256,
        created_at: epoch_ms(),
    }))
}

fn apply(root: &Path, name: &str, request: &str, raw: &str, confirmation: &str) -> Result<Value> {
    if !token(name) || !token(request) || raw.len() > 4096 {
        return Err("InvalidWorldRequest".into());
    }
    let plan: Plan = serde_json::from_str(raw).map_err(fail)?;
    if confirmation != if plan.key == TIME { "time" } else { "rules" } {
        return Err("WorldConfirmationRequired".into());
    }
    let _lock = instance_writer_lock(root, name)?;
    let directory = journal_directory(root, name)?;
    let path = directory.join(format!("request-{request}.json"));
    let pending_path = directory.join("pending.json");
    if directory.exists() {
        no_redirect(&directory)?;
    }
    let pending = optional_receipt(&pending_path)?;
    if let Some(existing) = optional_receipt(&path)?
        .or_else(|| pending.clone().filter(|record| record.request == request))
    {
        if existing.name != name || existing.request != request || existing.plan != plan {
            return Err("RequestConflict".into());
        }
        return Ok(json!(existing));
    }
    let now = epoch_ms();
    if now < plan.created_at || now - plan.created_at > PLAN_LIFETIME {
        return Err("WorldPreviewExpired".into());
    }
    let target = Target::get(root, name, &plan.run)?;
    let field = target.field(&plan.key)?;
    if target.registration != plan.registration
        || target.profile.sha256 != plan.sha256
        || !field.valid(&plan.target)
        || target.clock_flow(root, &field)? != plan.flowing
    {
        return Err("WorldPreviewChanged".into());
    }
    if let Some(pending) = pending {
        clear_previous_pending(&directory, name, &plan.run, &pending)?;
    }
    let before = target.query(root, &field)?;
    if plan.flowing != Some(true) && before != plan.before {
        return Err("WorldPreviewChanged".into());
    }
    let mut journal = Journal {
        name: name.into(),
        request: request.into(),
        state: "Sending".into(),
        before_at_apply: before.clone(),
        plan,
        observed: None,
        error: None,
    };
    if !directory.exists() {
        fs::create_dir(&directory).map_err(fail)?;
    }
    no_redirect(&directory)?;
    if before == journal.plan.target {
        journal.state = "NoChange".into();
        journal.observed = Some(before);
        core_maintenance::write_durable(&path, &journal, false)?;
        return Ok(json!(journal));
    }
    core_maintenance::write_durable(&pending_path, &journal, false)?;
    core_maintenance::write_durable(&path, &journal, false)?;
    let outcome = (|| -> Result<String> {
        target.set(root, &field, &journal.plan.target)?;
        let value = target.query(root, &field)?;
        if !matches_readback(&value, &journal.plan.target, journal.plan.flowing == Some(true)) {
            return Err("WorldReadbackMismatch".into());
        }
        Ok(value)
    })();
    match outcome {
        Ok(observed) => {
            // A running clock cannot be expected to contain this same tick at shutdown.
            journal.state = if field.is_time() && journal.plan.flowing == Some(true) {
                "Applied"
            } else {
                "AwaitingNormalStop"
            }
            .into();
            journal.observed = Some(observed);
        }
        Err(error) => {
            journal.state = "Unconfirmed".into();
            journal.error = Some(error);
        }
    }
    core_maintenance::write_durable(&path, &journal, true)?;
    if matches!(journal.state.as_str(), "AwaitingNormalStop" | "Applied") {
        fs::remove_file(pending_path).map_err(fail)?;
    }
    Ok(json!(journal))
}

fn clear_previous_pending(
    directory: &Path,
    name: &str,
    run: &str,
    pending: &Journal,
) -> Result<()> {
    if pending.name != name || !token(&pending.request) {
        return Err("InvalidWorldJournal".into());
    }
    let receipt_path = directory.join(format!("request-{}.json", pending.request));
    let previous = optional_receipt(&receipt_path)?;
    if previous.as_ref().is_some_and(|receipt| {
        receipt.name != name || receipt.request != pending.request || receipt.plan != pending.plan
    }) {
        return Err("InvalidWorldJournal".into());
    }
    let settled = previous.as_ref().is_some_and(|receipt| {
        matches!(receipt.state.as_str(), "Applied" | "AwaitingNormalStop" | "Saved")
    });
    if !settled && pending.plan.run == run {
        return Err("WorldOutcomeUnconfirmed".into());
    }
    if previous.is_none() {
        // A previous run might have ended before its uncertain receipt was published.
        core_maintenance::write_durable(&receipt_path, pending, false)?;
    }
    fs::remove_file(directory.join("pending.json")).map_err(fail)
}

pub(super) fn dispatch(root: &Path, verb: &str, args: &[String]) -> Option<Result<Value>> {
    match (verb, args) {
        ("world-live-values", [name]) => Some(queued::values(root, name)),
        ("world-live-clock", [name, run]) => Some(current_clock(root, name, run)),
        ("world-live-queue", [name, request, key, value, confirmation]) => {
            Some(queued::submit_edit(root, name, request, key, value, confirmation))
        }
        ("world-live-queue-rules", [name, request, raw]) => {
            Some(queued::submit_rules(root, name, request, raw))
        }
        ("world-live-save-rules", [name, request, raw]) => {
            Some(queued::save_rules(root, name, request, raw))
        }
        ("world-live-preview", [name, run, key, value]) => {
            Some(preview(root, name, run, key, value))
        }
        ("world-live-apply", [name, request, plan, confirmation]) => {
            Some(apply(root, name, request, plan, confirmation))
        }
        ("world-live-result", [name, request]) => {
            Some(confirm_saved(root, name, request).map(|journal| json!(journal)))
        }
        ("world-live-clock" | "world-live-preview" | "world-live-apply" | "world-live-result" | "world-live-values" | "world-live-queue" | "world-live-queue-rules" | "world-live-save-rules", _) => {
            Some(Err("InvalidWorldRequest".into()))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_accepts_only_canonical_command_shapes() {
        assert!(allowed_line("gamerule minecraft:spawn_mobs false"));
        assert!(allowed_line("gamerule doMobSpawning"));
        assert!(allowed_line("gamerule minecraft:max_entity_cramming 9999"));
        assert!(allowed_line("time query time"));
        assert!(allowed_line("time set 23999"));
        for invalid in [
            "stop",
            "time set day",
            "time set 24000",
            "gamerule minecraft:spawn_mobs false\nstop",
            "gamerule  doMobSpawning false",
            "gamerule minecraft:doMobSpawning",
            "gamerule minecraft:max_entity_cramming -1",
            "gamerule minecraft:unknown true",
        ] {
            assert!(!allowed_line(invalid), "{invalid}");
        }
    }

    #[test]
    fn validates_observed_reply_formats_from_isolated_cores() {
        let modern = Field::Rule {
            name: "minecraft:spawn_mobs".into(),
            suffix: "spawn_mobs",
        };
        let legacy = Field::Rule {
            name: "doMobSpawning".into(),
            suffix: "doMobSpawning",
        };
        let folia = log_message("[12:18:55] [Folia Region Scheduler Thread #0/INFO]: Gamerule spawn_mobs is currently set to: false\n").unwrap();
        let paper = log_message("[12:18:34] [Server thread/INFO]: System chat: Game rule spawn_mobs is currently set to false\r\n").unwrap();
        for message in [folia, paper] {
            assert_eq!(modern.query_reply(message).as_deref(), Some("false"));
            assert!(!modern.set_reply(message, "false"));
        }
        assert!(modern.set_reply("Game rule spawn_mobs is now set to true", "true"));
        assert!(legacy.set_reply("Game rule has been updated", "true"));
        assert_eq!(
            legacy.query_reply("doMobSpawning = true").as_deref(),
            Some("true")
        );
        assert_eq!(legacy.query_reply("Game rule has been updated"), None);
        assert!(!modern.set_reply("Game rule spawn_monsters is now set to true", "true"));
        assert_eq!(log_message("[12:18:34] [Async Chat Thread/INFO]: Game rule spawn_mobs is currently set to false\n"), None);

        let clock = Field::Time { modern: true };
        assert!(clock.set_reply(
            "Clock minecraft:overworld is already set to 6000 tick(s)",
            "6000"
        ));
        assert_eq!(
            clock
                .query_reply("Clock minecraft:overworld is at 6002 tick(s)")
                .as_deref(),
            Some("6002")
        );
        assert_eq!(
            clock.query_reply("Clock minecraft:the_end is at 6002 tick(s)"),
            None
        );
    }

    #[test]
    fn activity_omits_values_and_keeps_persistence_separate_from_live_readback() {
        let mut journal = Journal {
            name: "fixture".into(),
            request: "work".into(),
            plan: Plan {
                run: "test-run".into(),
                key: TIME.into(),
                target: "12000".into(),
                before: "13000".into(),
                flowing: Some(false),
                registration: "registration".into(),
                sha256: "hash".into(),
                created_at: 789,
            },
            state: "Sending".into(),
            before_at_apply: "13000".into(),
            observed: None,
            error: Some("private raw output".into()),
        };
        assert_eq!(
            activity_row(&journal),
            json!({"id":"work","instance":"fixture","createdAt":789,"state":"Unconfirmed","action":"world-live-time"})
        );
        journal.state = "AwaitingNormalStop".into();
        journal.observed = Some("12000".into());
        assert_eq!(
            activity_row(&journal),
            json!({"id":"work","instance":"fixture","createdAt":789,"state":"AwaitingNormalStop","action":"world-live-time"})
        );
    }
}
