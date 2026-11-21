//! Bound automatic restart decisions to a drained same-run exit and durable budget.
use super::*;

const EXIT: &str = "auto-restart-exit.json";
const STATE: &str = "auto-restart-state.json";
const MAX_RECORD: u64 = 16 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Exit {
    run: Run,
    code: i32,
    pub(super) observed_at: u64,
    ready_at: Option<u64>,
    manual_stop: bool,
    native_console: bool,
    policy: String,
}

fn policy_revision(policy: &AutoRestart) -> Result<String> {
    content_revision::revision(&serde_json::to_vec(policy).map_err(fail)?)
}

pub(super) fn record_exit(
    folder: &Path,
    policy: &AutoRestart,
    run: &Run,
    code: i32,
    observed_at: u64,
    ready_at: Option<u64>,
    native_console: bool,
) -> Result<()> {
    let exit = Exit {
        run: run.clone(),
        code,
        observed_at,
        ready_at,
        native_console,
        manual_stop: manual_stop(folder)?,
        policy: policy_revision(policy)?,
    };
    core_maintenance::write_durable(&folder.join(EXIT), &exit, false)
}

fn optional<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(file_error("inspect-auto-restart", path, error)),
        Ok(_) => no_redirect(path)?,
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(fail)?
        .take(MAX_RECORD + 1)
        .read_to_end(&mut bytes)
        .map_err(fail)?;
    if bytes.len() as u64 > MAX_RECORD {
        return Err("InvalidAutoRestartRecord".into());
    }
    serde_json::from_slice(&bytes).map(Some).map_err(fail)
}

fn manual_stop(folder: &Path) -> Result<bool> {
    for marker in ["stop-queued", "stop-sent", "batch-stop-accepted.json"] {
        let path = folder.join(marker);
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                no_redirect(&path)?;
                return Ok(true);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(fail(error)),
        }
    }
    Ok(cancellation_token(folder)?.is_some())
}

impl Exit {
    fn reason(&self, policy: &AutoRestart) -> Option<&'static str> {
        match (
            self.code,
            self.ready_at,
            self.manual_stop,
            self.native_console,
        ) {
            (_, _, true, _) | (FORCE_EXIT, _, _, _) => Some("ManualStop"),
            (0, Some(_), false, true) => Some("NativeConsoleStopNotDistinguishable"),
            (0, Some(_), false, false) if !policy.on_unexpected_exit => {
                Some("UnexpectedExitDisabled")
            }
            (0, Some(_), false, false) => None,
            _ if !policy.on_crash => Some("CrashRestartDisabled"),
            _ => None,
        }
    }
}

pub(super) fn eligible_exit(
    folder: &Path,
    run: &Run,
    policy: &AutoRestart,
    clock: u64,
) -> Result<Option<Exit>> {
    if manual_stop(folder)? {
        return Ok(None);
    }
    let Some(exit): Option<Exit> = optional(&folder.join(EXIT))? else {
        return Ok(None);
    };
    if exit.run != *run
        || exit.policy != policy_revision(policy)?
        || exit.observed_at > clock
        || exit.ready_at.is_some_and(|ready| ready > exit.observed_at)
        || clock.saturating_sub(exit.observed_at) > 24 * 60 * 60 * 1000
        || optional::<Option<i32>>(&folder.join("exit.json"))? != Some(Some(exit.code))
        || optional::<u64>(&folder.join("exit-observed-at.json"))? != Some(exit.observed_at)
        || exit.reason(policy).is_some()
    {
        return Ok(None);
    }
    Ok(Some(exit))
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
enum Phase {
    Pending { registration: String, due_at: u64 },
    Paused,
    Suppressed,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct State {
    attempts: Vec<u64>,
    sequence: u8,
    phase: Option<Phase>,
}

fn read_state(folder: &Path) -> Result<Option<State>> {
    let state: Option<State> = optional(&folder.join(STATE))?;
    if state.as_ref().is_some_and(|item| {
        item.sequence > 10 || item.attempts.len() > 10 || !item.attempts.is_sorted()
    }) {
        return Err("InvalidAutoRestartRecord".into());
    }
    Ok(state)
}

fn persist_state(folder: &Path, state: &State) -> Result<()> {
    let path = folder.join(STATE);
    core_maintenance::write_durable(&path, state, path.exists())
}

impl State {
    fn recent(&mut self, policy: &AutoRestart, clock: u64) {
        let cutoff = clock.saturating_sub(policy.window_seconds.unwrap_or(600) * 1000);
        self.attempts.retain(|at| *at >= cutoff);
    }

    fn consider(
        &mut self,
        exit: &Exit,
        policy: &AutoRestart,
        clock: u64,
        buffer: u64,
        registration: &str,
    ) -> (Option<u8>, bool) {
        match &self.phase {
            Some(Phase::Paused | Phase::Suppressed) => return (None, false),
            Some(Phase::Pending {
                registration: expected,
                ..
            }) if registration != expected => {
                self.phase = Some(Phase::Suppressed);
                return (None, true);
            }
            _ => (),
        }
        self.recent(policy, clock);
        if self.attempts.len() >= usize::from(policy.max_attempts) {
            self.phase = Some(Phase::Paused);
            return (None, true);
        }
        if exit.ready_at.is_some_and(|ready| {
            exit.observed_at.saturating_sub(ready) >= policy.stable_seconds.unwrap_or(300) * 1000
        }) {
            self.sequence = 0;
        }
        let attempt = self.sequence.saturating_add(1).min(policy.max_attempts);
        if let Some(Phase::Pending { due_at, .. }) = &self.phase {
            return ((clock >= *due_at).then_some(attempt), false);
        }
        let delay = policy
            .retry_delays_seconds
            .as_ref()
            .map_or(policy.delay_seconds, |seconds| {
                seconds[usize::from(attempt - 1)]
            });
        self.phase = Some(Phase::Pending {
            registration: registration.into(),
            due_at: clock.saturating_add(delay * 1000).saturating_add(buffer),
        });
        (None, true)
    }
}

pub(super) fn next_attempt(
    folder: &Path,
    exit: &Exit,
    policy: &AutoRestart,
    clock: u64,
    buffer: u64,
    registration: &str,
) -> Result<Option<u8>> {
    if !policy.advanced() {
        let count = optional::<u8>(&folder.join("auto-restart-attempt.json"))?.unwrap_or_default();
        return Ok(count.checked_add(1).filter(|attempt| {
            *attempt <= policy.max_attempts
                && clock
                    >= exit
                        .observed_at
                        .saturating_add(policy.delay_seconds * 1000)
                        .saturating_add(buffer)
        }));
    }
    let mut state = read_state(folder)?.unwrap_or_default();
    if state.attempts.last().is_some_and(|last| *last > clock) {
        return Err("InvalidAutoRestartClock".into());
    }
    let (attempt, changed) = state.consider(exit, policy, clock, buffer, registration);
    if changed {
        persist_state(folder, &state)?;
    }
    Ok(attempt)
}

pub(super) fn pause_after_dispatch(folder: &Path, policy: &AutoRestart) -> Result<()> {
    if policy.advanced() {
        let mut state = read_state(folder)?.ok_or("InvalidAutoRestartRecord")?;
        state.phase = Some(Phase::Paused);
        persist_state(folder, &state)?;
    }
    Ok(())
}

pub(super) fn inherit(
    previous_folder: &Path,
    policy: &AutoRestart,
    attempt: u8,
    clock: u64,
) -> Result<impl FnOnce(&Path) -> Result<()>> {
    let mut state = read_state(previous_folder)?.ok_or("InvalidAutoRestartRecord")?;
    state.recent(policy, clock);
    if state.attempts.len() >= usize::from(policy.max_attempts)
        || state.attempts.last().is_some_and(|last| *last > clock)
        || !matches!(state.phase, Some(Phase::Pending { due_at, .. }) if clock >= due_at)
    {
        return Err("InvalidAutoRestartRecord".into());
    }
    state.phase = None;
    state.sequence = attempt;
    state.attempts.push(clock);
    Ok(move |folder: &Path| persist_state(folder, &state))
}

pub(super) fn state_for_status(
    root: &Path,
    folder: &Path,
    policy: &AutoRestart,
    run: Option<&Run>,
    stopped: bool,
) -> Value {
    let observer = resident_manager::observing(root);
    let unavailable = || json!({"phase":"Unconfirmed","paused":false,"managerObserved":observer});
    if !policy.enabled() {
        return json!({"phase":"Disabled","paused":false});
    }
    if !policy.valid() {
        return unavailable();
    }
    let status = (|| {
        let mut state = read_state(folder)?.unwrap_or_default();
        state.recent(policy, epoch_ms());
        let attempt_count = if policy.advanced() {
            state.attempts.len()
        } else {
            usize::from(
                optional::<u8>(&folder.join("auto-restart-attempt.json"))?.unwrap_or_default(),
            )
        };
        let exit: Option<Exit> = optional(&folder.join(EXIT))?;
        let revision = policy_revision(policy)?;
        let reason = exit.as_ref().and_then(|exit| {
            if exit.policy != revision {
                Some("PolicyChanged")
            } else {
                exit.reason(policy)
            }
        });
        let phase = match &state.phase {
            Some(Phase::Paused) => "Paused",
            _ if stopped && manual_stop(folder)? => "Suppressed",
            _ if stopped && reason.is_some() => "Suppressed",
            None if !policy.advanced()
                && stopped
                && exit.is_some()
                && attempt_count >= usize::from(policy.max_attempts) =>
            {
                "Paused"
            }
            Some(Phase::Suppressed) => "Suppressed",
            Some(Phase::Pending { .. }) => "Scheduled",
            None if stopped && run.is_some() && exit.is_none() => "Unconfirmed",
            None => "Watching",
        };
        Ok::<_, String>(json!({
            "phase":phase, "paused":phase=="Paused","attemptCount":attempt_count,
            "managerObserved":observer,
            "dueAt": match state.phase { Some(Phase::Pending { due_at, .. }) if phase == "Scheduled" => Some(due_at), _ => None },
            "reason":reason,
        }))
    })();
    status.unwrap_or_else(|_| unavailable())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> AutoRestart {
        serde_json::from_value(json!({
            "onCrash":true, "onUnexpectedExit":true, "delaySeconds":2, "maxAttempts":3,
            "retryDelaysSeconds":[2,5,10], "windowSeconds":60, "stableSeconds":120,
        }))
        .unwrap()
    }

    fn exit(code: i32, observed_at: u64, ready_at: Option<u64>) -> Exit {
        Exit {
            run: Run {
                id: "probe".into(),
                host: Identity { pid: 1, created: 1 },
                java: Identity { pid: 2, created: 2 },
            },
            code,
            observed_at,
            ready_at,
            native_console: false,
            manual_stop: false,
            policy: String::new(),
        }
    }

    #[test]
    fn budget_pause_survives_window_expiry_and_serialization() {
        let policy = policy();
        let mut state = State {
            attempts: vec![1000, 2000, 3000],
            sequence: 3,
            phase: None,
        };
        let exit = exit(123, 4000, None);
        assert_eq!(state.consider(&exit, &policy, 4000, 0, "old"), (None, true));
        assert!(matches!(state.phase, Some(Phase::Paused)));
        let mut restored: State =
            serde_json::from_value(serde_json::to_value(&state).unwrap()).unwrap();
        assert_eq!(
            restored.consider(&exit, &policy, 100_000, 0, "old"),
            (None, false)
        );
    }

    #[test]
    fn elapsed_window_does_not_clear_sequence_and_stability_does_not_clear_window() {
        let policy = policy();
        let mut state = State {
            attempts: vec![1000, 2000],
            sequence: 2,
            phase: None,
        };
        let crashed = exit(123, 65_000, None);
        assert_eq!(
            state.consider(&crashed, &policy, 65_000, 0, "old"),
            (None, true)
        );
        assert!(state.attempts.is_empty());
        assert_eq!(
            state.consider(&crashed, &policy, 74_999, 0, "old"),
            (None, false)
        );
        assert_eq!(
            state.consider(&crashed, &policy, 75_000, 0, "old"),
            (Some(3), false)
        );
        let mut state = State {
            attempts: vec![186_000, 187_000],
            sequence: 2,
            phase: None,
        };
        let stable = exit(123, 200_000, Some(79_000));
        assert_eq!(
            state.consider(&stable, &policy, 200_000, 300, "old"),
            (None, true)
        );
        assert_eq!(state.attempts.len(), 2);
        assert_eq!(
            state.consider(&stable, &policy, 202_299, 300, "old"),
            (None, false)
        );
        assert_eq!(
            state.consider(&stable, &policy, 202_300, 300, "old"),
            (Some(1), false)
        );
        assert_eq!(
            state.consider(&stable, &policy, 202_300, 300, "new"),
            (None, true)
        );
        assert!(matches!(state.phase, Some(Phase::Suppressed)));
    }

    #[test]
    fn manual_stop_and_ambiguous_native_exit_never_restart() {
        let mut policy = policy();
        let mut stopped = exit(0, 4000, Some(2000));
        assert_eq!(stopped.reason(&policy), None);
        stopped.manual_stop = true;
        assert_eq!(stopped.reason(&policy), Some("ManualStop"));
        stopped.manual_stop = false;
        stopped.native_console = true;
        assert_eq!(
            stopped.reason(&policy),
            Some("NativeConsoleStopNotDistinguishable")
        );
        stopped.ready_at = None;
        policy.on_crash = false;
        assert_eq!(stopped.reason(&policy), Some("CrashRestartDisabled"));
    }

    #[test]
    fn original_fixed_delay_settings_deserialize_without_new_fields() {
        let raw =
            json!({"onCrash":true, "onUnexpectedExit":false, "delaySeconds":20, "maxAttempts":3});
        let legacy: AutoRestart = serde_json::from_value(raw.clone()).unwrap();
        assert!(legacy.valid() && !legacy.advanced());
        assert_eq!(serde_json::to_value(legacy).unwrap(), raw);
        let mut invalid = policy();
        invalid.retry_delays_seconds = Some(vec![3]);
        assert!(!invalid.valid());
    }
}
