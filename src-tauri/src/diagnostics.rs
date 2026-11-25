//! Bounded, value-free GUI/IPC diagnostics. Never records command arguments or replies.
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};

const FILE_LIMIT: u64 = 2 * 1024 * 1024;
const DIRECTORY_LIMIT: u64 = 16 * 1024 * 1024;
pub struct Diagnostics {
    path: PathBuf,
    state: Mutex<LogState>,
    limit: u64,
    budget: u64,
}
struct LogState {
    file: File,
    bytes: u64,
    capped: bool,
    reads: HashSet<String>,
    outcomes: HashMap<String, Value>,
    failure: Option<String>,
}
static FILE_ID: AtomicU64 = AtomicU64::new(1);
fn io_trace(stage: &str, error: std::io::Error) -> String {
    match error.raw_os_error() {
        Some(code) => format!("{stage} (os error {code})"),
        None => format!("{stage} ({:?})", error.kind()),
    }
}
fn now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
fn plain_path(path: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| io_trace("DiagnosticInspectPath", e))?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err("DiagnosticReparsePoint".into());
        }
    }
    if meta.file_type().is_symlink() {
        return Err("DiagnosticSymlink".into());
    }
    Ok(())
}
impl Diagnostics {
    pub fn open(directory: &Path) -> Result<Self, String> {
        Self::with_limit(directory, FILE_LIMIT, DIRECTORY_LIMIT)
    }
    fn with_limit(directory: &Path, limit: u64, budget: u64) -> Result<Self, String> {
        plain_path(directory)?;
        let used = fs::read_dir(directory)
            .map_err(|e| io_trace("DiagnosticReadDirectory", e))?
            .try_fold(0u64, |sum, entry| {
                let entry = entry.map_err(|e| io_trace("DiagnosticReadEntry", e))?;
                let meta = fs::symlink_metadata(entry.path()).map_err(|e| io_trace("DiagnosticInspectEntry", e))?;
                Ok::<_, String>(sum.saturating_add(if meta.is_file() { meta.len() } else { 0 }))
            })?;
        // Reserve one whole session without deleting any previous diagnostic evidence.
        if used.saturating_add(limit) > budget {
            return Err("DiagnosticDirectoryBudgetReached".into());
        }
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = directory.join(format!(
            "gui-{}-{stamp}-{}.jsonl",
            std::process::id(),
            FILE_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| io_trace("DiagnosticCreateSession", e))?;
        Ok(Self {
            path,
            state: Mutex::new(LogState {
                file,
                bytes: 0,
                capped: false,
                reads: HashSet::new(),
                outcomes: HashMap::new(),
                failure: None,
            }),
            limit,
            budget,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn runtime_result(&self, result: &Result<(), String>) -> Result<(), String> {
        self.record(json!({"type":"gui-exit","ok":result.is_ok(),"error":result.as_ref().err().map(|e|safe_error(e))}))
    }
    pub fn begin(&self, id: u64, action: &str, origin: &str) -> Result<(), String> {
        let read = matches!(
            action,
            "list"
                | "presentation"
                | "presentation-metadata"
                | "instance-snapshot"
                | "preferences-snapshot"
                | "status"
                | "jobs"
                | "batch-list"
                | "instance-settings"
                | "discover-java"
                | "preview-java-discovery"
                | "java-discovery-settings"
                | "validate-java-discovery-settings"
                | "world-settings"
                | "world-batch-preview"
                | "get-initialization"
                | "logs"
        );
        let sample = read && origin != "manual";
        let key = format!("begin:{action}");
        if sample
            && self
                .state
                .lock()
                .map_err(|_| "DiagnosticLockPoisoned")?
                .reads
                .contains(&key)
        {
            return Ok(());
        }
        self.record(json!({"type":"ipc-begin","id":id,"action":safe_action(action),"origin":if origin=="manual"{"manual"}else{"background"}}))?;
        if sample {
            self.state
                .lock()
                .map_err(|_| "DiagnosticLockPoisoned")?
                .reads
                .insert(key);
        }
        Ok(())
    }
    pub fn status(&self) -> Value {
        match self.state.lock() {
            Ok(s) => {
                json!({"path":self.path,"available":!s.capped&&s.failure.is_none(),"error":s.failure})
            }
            Err(_) => json!({"available":false,"error":"DiagnosticLockPoisoned"}),
        }
    }
    pub fn record(&self, event: Value) -> Result<(), String> {
        let result = self.write(event);
        if let Err(error) = &result {
            if let Ok(mut state) = self.state.lock() {
                state.failure = Some(error.clone())
            }
        }
        result
    }
    fn write(&self, event: Value) -> Result<(), String> {
        let mut state = self.state.lock().map_err(|_| "DiagnosticLockPoisoned")?;
        if state.capped {
            return Err("DiagnosticSessionLimitReached".into());
        }
        let mut bytes = serde_json::to_vec(
            &json!({"schema":"ih-gui-diagnostic/v1","timeUnixMs":now(),"event":event}),
        )
        .map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        let directory = self.path.parent().ok_or("InvalidDiagnosticDirectory")?;
        let lock_path = directory.join(".budget.lock");
        if lock_path.exists() {
            plain_path(&lock_path)?
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(lock_path)
            .map_err(|e| io_trace("DiagnosticOpenBudgetLock", e))?;
        let until = std::time::Instant::now() + std::time::Duration::from_millis(250);
        loop {
            match lock.try_lock() {
                Ok(()) => break,
                Err(_) => {
                    if std::time::Instant::now() >= until {
                        return Err("DiagnosticWriterBusy".into());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5))
                }
            }
        }
        let used = fs::read_dir(directory)
            .map_err(|e| io_trace("DiagnosticReadDirectory", e))?
            .try_fold(0u64, |sum, entry| {
                let meta = fs::symlink_metadata(entry.map_err(|e| io_trace("DiagnosticReadEntry", e))?.path())
                    .map_err(|e| io_trace("DiagnosticInspectEntry", e))?;
                Ok::<_, String>(sum.saturating_add(if meta.is_file() { meta.len() } else { 0 }))
            })?;
        if used.saturating_add(bytes.len() as u64 + 256) > self.budget {
            return Err("DiagnosticDirectoryBudgetReached".into());
        }
        if state.bytes + bytes.len() as u64 + 256 > self.limit {
            let marker=b"{\"schema\":\"ih-gui-diagnostic/v1\",\"event\":{\"type\":\"session-limit-reached\"}}\n";
            state.file.write_all(marker).map_err(|e| io_trace("DiagnosticWriteSession", e))?;
            state.file.flush().map_err(|e| io_trace("DiagnosticFlushSession", e))?;
            state.bytes += marker.len() as u64;
            state.capped = true;
            return Err("DiagnosticSessionLimitReached".into());
        }
        state.file.write_all(&bytes).map_err(|e| io_trace("DiagnosticWriteSession", e))?;
        state.file.flush().map_err(|e| io_trace("DiagnosticFlushSession", e))?;
        state.bytes += bytes.len() as u64;
        Ok(())
    }
    pub fn ipc(
        &self,
        id: u64,
        action: &str,
        origin: &str,
        elapsed: u128,
        result: &Result<Value, String>,
    ) -> Result<(), String> {
        let read = matches!(
            action,
            "list"
                | "presentation"
                | "presentation-metadata"
                | "instance-snapshot"
                | "preferences-snapshot"
                | "status"
                | "jobs"
                | "batch-list"
                | "instance-settings"
                | "discover-java"
                | "preview-java-discovery"
                | "java-discovery-settings"
                | "validate-java-discovery-settings"
                | "world-settings"
                | "get-initialization"
                | "logs"
        );
        let tracked_failure = matches!(action, "instance-snapshot" | "preferences-snapshot")
            || (action == "jobs" && result.as_ref().is_err_and(|error| {
                error.starts_with("InvalidInstanceRecord") || error.starts_with("FileAccess[")
            }));
        let sample = (result.is_ok() || tracked_failure) && read && origin == "background";
        let count = result.as_ref().ok().and_then(|v| {
            if action == "list" {
                v.as_array().map(Vec::len)
            } else if matches!(action, "presentation" | "presentation-metadata" | "instance-snapshot") {
                v.get("instances").and_then(Value::as_array).map(Vec::len)
            } else {
                None
            }
        });
        let error = result.as_ref().err().map(|error| safe_error(error));
        // Include reason/OS code changes without writing user-selected paths to logs.
        let issues: Vec<_> = result.as_ref().ok().filter(|_| action == "instance-snapshot")
            .and_then(|value| value["issues"].as_array())
            .into_iter().flatten()
            .map(|issue| json!({"stage":issue["stage"],"reason":issue["reason"],"code":issue["code"],"osError":issue["osError"]}))
            .collect();
        let outcome = json!({"rowCount":count,"error":error,"issues":issues});
        if sample
            && self
                .state
                .lock()
                .map_err(|_| "DiagnosticLockPoisoned")?
                .outcomes
                .get(action)
                == Some(&outcome)
        {
            return Ok(());
        }
        self.record(json!({"type":"ipc-result","id":id,"action":safe_action(action),"origin":if origin=="manual"{"manual"}else{"background"},"elapsedMs":elapsed,"ok":result.is_ok(),"rowCount":count,"error":error,"registryIssues":issues}))?;
        if read {
            let mut state = self.state.lock().map_err(|_| "DiagnosticLockPoisoned")?;
            if result.is_ok() || tracked_failure {
                state.outcomes.insert(action.into(), outcome);
            } else {
                // The first successful read after failure is diagnostic evidence too.
                state.outcomes.remove(action);
            }
        }
        Ok(())
    }
}
pub fn safe_action(action: &str) -> &str {
    match action {
        "list"
        | "presentation"
        | "presentation-metadata"
        | "preferences"
        | "preferences-snapshot"
        | "instance-snapshot"
        | "status"
        | "jobs"
        | "batch-list"
        | "batch-connect"
        | "console-open"
        | "console-plan"
        | "batch-start"
        | "batch-stop"
        | "batch-restart"
        | "batch-cancel"
        | "instance-settings"
        | "discover-java"
        | "preview-java-discovery"
        | "java-discovery-settings"
        | "validate-java-discovery-settings"
        | "save-java-discovery-settings"
        | "world-settings"
        | "get-initialization"
        | "logs"
        | "save-preferences"
        | "save-instance-settings"
        | "save-world-settings"
        | "save-initialization"
        | "start"
        | "start-if-current"
        | "stop"
        | "restart"
        | "force-stop"
        | "create"
        | "prepare"
        | "doctor"
        | "defaults-diff"
        | "admin-install"
        | "admin-verify"
        | "agent-list"
        | "compatibility-probe-check"
        | "import"
        | "v1-handoff-preview"
        | "v1-handoff"
        | "v1-handoff-rollback"
        | "adopt"
        | "legacy-preview"
        | "recovery-key-status"
        | "recovery-key-rotate"
        | "host-recover"
        | "core-state"
        | "core-check"
        | "core-catalog-versions"
        | "core-catalog-build"
        | "core-batch-list"
        | "core-batch-status"
        | "core-batch-submit"
        | "core-batch-cancel"
        | "world-batch-preview"
        | "import-batch-apply"
        | "world-live-preview"
        | "world-live-apply"
        | "world-live-result"
        | "world-live-values"
        | "world-live-clock"
        | "world-live-queue"
        | "world-live-queue-rules"
        | "world-live-save-rules"
        | "create-download"
        | "core-install"
        | "core-install-latest"
        | "core-restore"
        | "core-reconcile"
        | "manager-connect"
        | "import-preview"
        | "scan-policy"
        | "global-java-settings"
        | "save-global-java-settings"
        | "test-admin-settings"
        | "save-test-admin-settings"
        | "test-admin-fetch-luckperms"
        | "test-admin-lookup"
        | "validate-scan-policy"
        | "preview-management-scope"
        | "save-management-scope"
        | "gui-create-preview"
        | "gui-create-rescope"
        | "gui-create-confirm"
        | "gui-create-recover"
        | "management-folder-preview"
        | "management-folder-save"
        | "preview-scan-policy"
        | "save-scan-policy"
        | "remove-management"
        | "delete-server"
        | "removed-instances"
        | "open-server-folder"
        | "open-log-folder" => action,
        "present-window" => action,
        _ => "unrecognized",
    }
}
pub fn safe_error(error: &str) -> Value {
    // Only typed I/O contexts may supply OS codes. A user setting can itself
    // contain strings such as "os error 5" or "0xDEADBEEF".
    let trusted_io = error.starts_with("FileAccess[");
    let os = trusted_io
        .then_some(error)
        .unwrap_or("")
        .split("os error ")
        .nth(1)
        .and_then(|s| s.split(')').next())
        .and_then(|s| s.parse::<u32>().ok());
    let detail = if let Some(rest) = error.strip_prefix("FileAccess[") {
        let stage = rest.split(']').next().unwrap_or("");
        let stage = match stage {
            "read" | "inspect" | "resolve" | "create-temp" | "write-temp" | "sync-temp" | "replace"
            | "remove-temp" | "open-command-lock" | "open-receipt-lock" => stage,
            _ => "unknown",
        };
        let file = error.split("path=").nth(1).unwrap_or("");
        let file = if file.contains("preferences.json:") {
            "preferences.json"
        } else if file.contains("manager.lock:") {
            "manager.lock"
        } else if file.contains("receipts.lock:") {
            "receipts.lock"
        } else if file.contains("global-java.json:") {
            "global-java.json"
        } else if file.contains("scan-policy.json:") {
            "scan-policy.json"
        } else if file.contains("workspace.json:") {
            "workspace.json"
        } else if file.contains(".tmp:") {
            "temporary-json"
        } else {
            "managed-file"
        };
        Some(json!({"stage":stage,"file":file}))
    } else {
        None
    };
    let code = error
        .split([':', ' ', '(', '['])
        .next()
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 64
                && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
        .unwrap_or("OperationFailed");
    let hresult = error
        .strip_prefix("WebviewError:")
        .unwrap_or("")
        .split("0x")
        .nth(1)
        .map(|s| {
            s.chars()
                .take_while(|c| c.is_ascii_hexdigit())
                .take(8)
                .collect::<String>()
        })
        .filter(|s| !s.is_empty());
    json!({"code":code,"osCode":os,"hresult":hresult,"detail":detail})
}
/// Diagnostic correlation only, not a cryptographic identity or authorization check.
pub fn path_fingerprint(path: &Path) -> String {
    let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let text = path.to_string_lossy().replace('\\', "/");
    let mut hash = 0xcbf29ce484222325u64;
    for byte in text.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3)
    }
    format!("fnv1a64:{hash:016x}")
}
static GLOBAL: OnceLock<Result<Diagnostics, String>> = OnceLock::new();
pub fn global() -> Result<&'static Diagnostics, String> {
    GLOBAL
        .get_or_init(|| {
            let mut root =
                PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("MissingLocalAppData")?);
            #[cfg(windows)]
            let app_directory = crate::desktop_channel::APP_DIRECTORY;
            #[cfg(not(windows))]
            let app_directory = "InstanceHarbor";
            for part in [app_directory, "logs"] {
                root.push(part);
                match fs::create_dir(&root) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(io_trace("DiagnosticCreateDirectory", e)),
                }
                plain_path(&root)?;
            }
            Diagnostics::open(&root)
        })
        .as_ref()
        .map_err(Clone::clone)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn folder() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "ih-diagnostics-{}-{}-{}",
            std::process::id(),
            now(),
            FILE_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        p
    }
    fn cleanup(p: PathBuf, f: PathBuf) {
        fs::remove_file(f).unwrap();
        fs::remove_file(p.join(".budget.lock")).unwrap();
        fs::remove_dir(p).unwrap()
    }
    #[test]
    fn host_recovery_is_named_without_accepting_argument_text() {
        assert_eq!(safe_action("host-recover"), "host-recover");
        assert_eq!(safe_action("host-recover arbitrary-value"), "unrecognized");
    }

    #[test]
    fn records_os_code_and_context_without_argument_or_response_values() {
        let p = folder();
        let d = Diagnostics::open(&p).unwrap();
        d.ipc(1,"save-preferences","manual",5,&Err("FileAccess[replace] path=C:/SECRET_TEST_VALUE/preferences.json: Access denied (os error 5)".into())).unwrap();
        d.ipc(
            2,
            "save-world-settings",
            "manual",
            5,
            &Err("UnsupportedWorldKey: SECRET_TEST_VALUE".into()),
        )
        .unwrap();
        d.ipc(
            3,
            "list",
            "manual",
            5,
            &Ok(json!([{"stopToken":"SECRET_TEST_VALUE"}])),
        )
        .unwrap();
        d.runtime_result(&Err("WebviewError: SECRET_TEST_VALUE 0x80070005".into()))
            .unwrap();
        let s = fs::read_to_string(d.path()).unwrap();
        assert!(!s.contains("SECRET_TEST_VALUE"));
        assert!(s.contains("\"osCode\":5"));
        assert!(s.contains("preferences.json"));
        assert!(s.contains("\"rowCount\":1"));
        assert!(s.contains("\"hresult\":\"80070005\""));
        let file = d.path.clone();
        drop(d);
        cleanup(p, file);
    }
    #[test]
    fn setting_values_cannot_masquerade_as_system_error_codes() {
        let detail = safe_error("UnsupportedWorldKey: 0xDEADBEEF os error 123)");
        assert_eq!(detail["hresult"], Value::Null);
        assert_eq!(detail["osCode"], Value::Null);
        assert!(!detail.to_string().contains("DEADBEEF"));
    }
    #[test]
    fn background_success_is_sampled_but_errors_and_manual_results_are_kept() {
        let p = folder();
        let d = Diagnostics::open(&p).unwrap();
        for _ in 0..5 {
            d.ipc(1, "list", "background", 1, &Ok(json!([]))).unwrap()
        }
        d.ipc(2, "list", "manual", 1, &Ok(json!([]))).unwrap();
        d.ipc(3, "list", "background", 1, &Err("ManagerBusy".into()))
            .unwrap();
        assert_eq!(fs::read_to_string(d.path()).unwrap().lines().count(), 3);
        let f = d.path.clone();
        drop(d);
        cleanup(p, f);
    }
    #[test]
    fn background_row_count_changes_and_recovery_remain_visible() {
        let p = folder();
        let d = Diagnostics::open(&p).unwrap();
        for (id, count) in [(1, 5), (2, 5), (3, 0), (4, 0), (5, 5)] {
            d.ipc(
                id,
                "list",
                "background",
                1,
                &Ok(json!(vec![json!({}); count])),
            )
            .unwrap();
        }
        d.ipc(6, "list", "background", 1, &Err("ManagerBusy".into()))
            .unwrap();
        d.ipc(7, "list", "background", 1, &Ok(json!(vec![json!({}); 5])))
            .unwrap();
        let events: Vec<Value> = fs::read_to_string(d.path())
            .unwrap()
            .lines()
            .map(|s| serde_json::from_str::<Value>(s).unwrap()["event"].clone())
            .collect();
        assert_eq!(events.len(), 5);
        assert_eq!(events[1]["rowCount"], 0);
        assert_eq!(events[2]["rowCount"], 5);
        assert_eq!(events[3]["ok"], false);
        assert_eq!(events[4]["ok"], true);
        let f = d.path.clone();
        drop(d);
        cleanup(p, f);
    }
    #[test]
    fn budgets_preserve_old_logs_and_cap_current_session() {
        let p = folder();
        let d = Diagnostics::with_limit(&p, 1024, 2048).unwrap();
        for _ in 0..20 {
            let _ = d.record(json!({"type":"test","padding":"x".repeat(90)}));
        }
        assert!(fs::metadata(d.path()).unwrap().len() <= 1024);
        assert!(fs::read_to_string(d.path())
            .unwrap()
            .contains("session-limit-reached"));
        assert!(Diagnostics::with_limit(&p, 2048, 2048).is_err());
        assert_eq!(d.status()["available"], false);
        let f = d.path.clone();
        drop(d);
        cleanup(p, f);
    }
    #[test]
    fn concurrent_sessions_keep_complete_json_and_distinct_files() {
        let p = folder();
        let sessions: Vec<_> = (0..4).map(|_| Diagnostics::open(&p).unwrap()).collect();
        let paths: Vec<_> = sessions.iter().map(|d| d.path.clone()).collect();
        assert_eq!(paths.iter().collect::<HashSet<_>>().len(), 4);
        let threads: Vec<_> = sessions
            .into_iter()
            .map(|d| {
                std::thread::spawn(move || {
                    for id in 0..40 {
                        d.begin(id, "list", "manual").unwrap();
                        d.ipc(id, "list", "manual", 1, &Ok(json!([]))).unwrap();
                    }
                    d.runtime_result(&Ok(())).unwrap();
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        for path in paths {
            let text = fs::read_to_string(&path).unwrap();
            assert_eq!(text.lines().count(), 81);
            for line in text.lines() {
                assert_eq!(
                    serde_json::from_str::<Value>(line).unwrap()["schema"],
                    "ih-gui-diagnostic/v1"
                );
            }
            fs::remove_file(path).unwrap();
        }
        fs::remove_file(p.join(".budget.lock")).unwrap();
        fs::remove_dir(p).unwrap();
    }
    #[test]
    fn shared_directory_budget_does_not_overwrite_prior_evidence() {
        let p = folder();
        let old = p.join("old.jsonl");
        fs::write(&old, "prior evidence").unwrap();
        let d = Diagnostics::with_limit(&p, 1024, 2048).unwrap();
        let external = p.join("other-session.jsonl");
        fs::write(&external, vec![b'x'; 1800]).unwrap();
        assert!(d.begin(1, "list", "manual").is_err());
        assert_eq!(d.status()["available"], false);
        assert_eq!(fs::read_to_string(&old).unwrap(), "prior evidence");
        assert_eq!(fs::metadata(d.path()).unwrap().len(), 0);
        let f = d.path.clone();
        drop(d);
        fs::remove_file(old).unwrap();
        fs::remove_file(external).unwrap();
        cleanup(p, f);
    }
    #[test]
    fn bounded_wait_keeps_contended_event_and_failed_sample_can_retry() {
        use std::sync::Arc;
        let p = folder();
        let d = Arc::new(Diagnostics::open(&p).unwrap());
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(p.join(".budget.lock"))
            .unwrap();
        lock.lock().unwrap();
        let copy = d.clone();
        let pending =
            std::thread::spawn(move || copy.ipc(1, "list", "background", 1, &Ok(json!([]))));
        std::thread::sleep(std::time::Duration::from_millis(30));
        drop(lock);
        pending.join().unwrap().unwrap();
        assert_eq!(fs::read_to_string(d.path()).unwrap().lines().count(), 1);
        let lock = OpenOptions::new()
            .write(true)
            .open(p.join(".budget.lock"))
            .unwrap();
        lock.lock().unwrap();
        assert!(d
            .ipc(
                2,
                "presentation",
                "background",
                1,
                &Ok(json!({"instances":[]}))
            )
            .is_err());
        drop(lock);
        d.ipc(
            3,
            "presentation",
            "background",
            1,
            &Ok(json!({"instances":[]})),
        )
        .unwrap();
        assert_eq!(fs::read_to_string(d.path()).unwrap().lines().count(), 2);
        assert_eq!(d.status()["available"], false); // a lost event is never hidden by a later success
        let f = d.path.clone();
        drop(d);
        cleanup(p, f);
    }
}
