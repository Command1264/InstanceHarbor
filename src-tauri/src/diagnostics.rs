//! Bounded, value-free GUI/IPC diagnostics. Never records command arguments or replies.
//!
//! One GUI launch writes `gui-<local start yyyyMMdd-HHmmss>-<pid>-<segment>.jsonl`. A full
//! segment continues in the next one. The folder keeps a total budget by removing the oldest
//! GUI logs of earlier launches; files of the current launch are never removed.
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
/// Room kept in every segment for the line that names the next segment.
const SEGMENT_RESERVE: u64 = 256;
pub struct Diagnostics {
    directory: PathBuf,
    /// The folder as shown to the user, without the user profile path.
    folder: String,
    state: Mutex<LogState>,
    limit: u64,
    budget: u64,
}
struct LogState {
    file: File,
    path: PathBuf,
    bytes: u64,
    reads: HashSet<String>,
    outcomes: HashMap<String, Value>,
    failure: Option<String>,
}
/// Segment numbers are shared by the whole process, so every file of one launch is distinct.
static SEGMENT: AtomicU64 = AtomicU64::new(1);
#[cfg(test)]
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
/// Calendar fields of a Unix time in seconds (proleptic Gregorian, no leap seconds).
#[cfg(any(test, not(windows)))]
fn civil(seconds: u64) -> (u64, u64, u64, u64, u64, u64) {
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year as u64, month as u64, day as u64, rest / 3600, rest / 60 % 60, rest % 60)
}
#[cfg(windows)]
fn local_stamp() -> String {
    #[repr(C)]
    #[derive(Default)]
    struct SystemTimeFields {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        milliseconds: u16,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLocalTime(time: *mut SystemTimeFields);
    }
    let mut t = SystemTimeFields::default();
    // SAFETY: GetLocalTime only fills the SYSTEMTIME structure it is given.
    unsafe { GetLocalTime(&mut t) };
    format!("{:04}{:02}{:02}-{:02}{:02}{:02}", t.year, t.month, t.day, t.hour, t.minute, t.second)
}
#[cfg(not(windows))]
fn local_stamp() -> String {
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let (y, mo, d, h, mi, s) = civil(seconds);
    format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}")
}
/// `gui-<local start time>-<pid>`, fixed for the whole launch so its segments sort together.
fn launch_prefix() -> &'static str {
    static PREFIX: OnceLock<String> = OnceLock::new();
    PREFIX.get_or_init(|| format!("gui-{}-{}", local_stamp(), std::process::id()))
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
fn create_segment(directory: &Path) -> Result<(File, PathBuf), String> {
    let mut attempts = 0;
    loop {
        let path = directory.join(format!(
            "{}-{}.jsonl",
            launch_prefix(),
            SEGMENT.fetch_add(1, Ordering::Relaxed)
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((file, path)),
            // A segment left by an earlier process with the same pid and second.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && attempts < 64 => {
                attempts += 1
            }
            Err(e) => return Err(io_trace("DiagnosticCreateSession", e)),
        }
    }
}
/// Background queries that the GUI repeats on a timer. Their unchanged results are written once.
fn periodic_read(action: &str) -> bool {
    matches!(
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
            | "core-batch-list"
            | "core-batch-status"
            | "core-state"
            | "core-check"
            | "core-catalog-versions"
            | "core-catalog-build"
            | "manager-connect"
            | "world-live-values"
            | "world-live-clock"
            | "recovery-key-status"
            | "scan-policy"
            | "validate-scan-policy"
    )
}
impl Diagnostics {
    /// Opens a new launch log in `directory`. `folder` is how messages name that folder.
    pub fn open(directory: &Path, folder: String) -> Result<Self, String> {
        let mut log = Self::with_limit(directory, FILE_LIMIT, DIRECTORY_LIMIT)?;
        log.folder = folder;
        Ok(log)
    }
    fn with_limit(directory: &Path, limit: u64, budget: u64) -> Result<Self, String> {
        plain_path(directory)?;
        let (file, path) = create_segment(directory)?;
        Ok(Self {
            directory: directory.to_path_buf(),
            folder: directory.to_string_lossy().into_owned(),
            state: Mutex::new(LogState {
                file,
                path,
                bytes: 0,
                reads: HashSet::new(),
                outcomes: HashMap::new(),
                failure: None,
            }),
            limit,
            budget,
        })
    }
    /// The segment written now.
    pub fn path(&self) -> PathBuf {
        match self.state.lock() {
            Ok(state) => state.path.clone(),
            Err(poisoned) => poisoned.into_inner().path.clone(),
        }
    }
    pub fn runtime_result(&self, result: &Result<(), String>) -> Result<(), String> {
        self.record(json!({"type":"gui-exit","ok":result.is_ok(),"error":result.as_ref().err().map(|e|safe_error(e))}))
    }
    pub fn begin(&self, id: u64, action: &str, origin: &str) -> Result<(), String> {
        let sample = periodic_read(action) && origin != "manual";
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
                json!({"path":s.path,"folder":self.folder,"available":s.failure.is_none(),"error":s.failure})
            }
            Err(_) => json!({"folder":self.folder,"available":false,"error":"DiagnosticLockPoisoned"}),
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
        let mut bytes = serde_json::to_vec(
            &json!({"schema":"ih-gui-diagnostic/v1","timeUnixMs":now(),"event":event}),
        )
        .map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        let lock_path = self.directory.join(".budget.lock");
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
        // An event larger than a whole segment still gets a segment of its own.
        let rotate = state.bytes > 0 && state.bytes + bytes.len() as u64 + SEGMENT_RESERVE > self.limit;
        let mut needed = bytes.len() as u64;
        if rotate {
            needed += SEGMENT_RESERVE
        }
        self.make_room(needed)?;
        if rotate {
            let (file, path) = create_segment(&self.directory)?;
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
            let marker = serde_json::to_vec(&json!({"schema":"ih-gui-diagnostic/v1","timeUnixMs":now(),"event":{"type":"segment-full","next":name}}))
                .map_err(|e| e.to_string())?;
            state.file.write_all(&marker).and_then(|()| state.file.write_all(b"\n"))
                .and_then(|()| state.file.flush())
                .map_err(|e| io_trace("DiagnosticWriteSession", e))?;
            state.file = file;
            state.path = path;
            state.bytes = 0;
        }
        state.file.write_all(&bytes).map_err(|e| io_trace("DiagnosticWriteSession", e))?;
        state.file.flush().map_err(|e| io_trace("DiagnosticFlushSession", e))?;
        state.bytes += bytes.len() as u64;
        Ok(())
    }
    /// Keeps the folder within its budget. Only GUI logs (`gui-*.jsonl`, both the current and
    /// the pre-1.1 name format) of other launches are removed, the least recently written first.
    fn make_room(&self, needed: u64) -> Result<(), String> {
        let own = format!("{}-", launch_prefix());
        let mut used = 0u64;
        let mut removable = Vec::new();
        for entry in fs::read_dir(&self.directory).map_err(|e| io_trace("DiagnosticReadDirectory", e))? {
            let entry = entry.map_err(|e| io_trace("DiagnosticReadEntry", e))?;
            let meta = match fs::symlink_metadata(entry.path()) {
                Ok(meta) => meta,
                // Another GUI of this folder removed it meanwhile.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(io_trace("DiagnosticInspectEntry", e)),
            };
            if !meta.is_file() {
                continue;
            }
            used = used.saturating_add(meta.len());
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("gui-") && name.ends_with(".jsonl") && !name.starts_with(&own) {
                let modified = meta.modified().unwrap_or(UNIX_EPOCH);
                removable.push((modified, name, meta.len()));
            }
        }
        removable.sort();
        let mut oldest = removable.into_iter();
        while used.saturating_add(needed) > self.budget {
            let Some((_, name, length)) = oldest.next() else {
                return Err("DiagnosticDirectoryBudgetReached".into());
            };
            match fs::remove_file(self.directory.join(&name)) {
                Ok(()) => used = used.saturating_sub(length),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => used = used.saturating_sub(length),
                // A file another GUI still holds stays; the next oldest one is tried.
                Err(_) => {}
            }
        }
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
        let read = periodic_read(action);
        let tracked_failure = matches!(action, "instance-snapshot" | "preferences-snapshot")
            || (action == "jobs" && result.as_ref().is_err_and(|error| {
                error.starts_with("InvalidInstanceRecord") || error.starts_with("FileAccess[")
            }));
        let sample = (result.is_ok() || tracked_failure) && read && origin == "background";
        let count = result.as_ref().ok().and_then(|v| {
            if matches!(action, "list" | "batch-list" | "jobs" | "core-batch-list") {
                v.as_array().map(Vec::len)
            } else if matches!(action, "presentation" | "presentation-metadata" | "instance-snapshot") {
                v.get("instances").and_then(Value::as_array).map(Vec::len)
            } else if action == "core-check" {
                v.get("rows").and_then(Value::as_array).map(Vec::len)
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
        | "core-install-version"
        | "core-velocity-versions"
        | "core-velocity-target"
        | "core-reconcile"
        | "manager-connect"
        | "import-preview"
        | "scan-policy"
        | "global-java-settings"
        | "save-global-java-settings"
        | "creation-defaults"
        | "save-creation-defaults"
        | "cache-preview"
        | "cache-clear"
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
/// The folders under LocalAppData that hold GUI logs. A development workspace gets its
/// own folder, so GUIs of different workspaces never wait for one another's budget lock.
fn log_folder_parts<'a>(app_directory: &'a str, workspace: Option<&'a str>) -> Vec<&'a str> {
    [app_directory, "logs"].into_iter().chain(workspace).collect()
}
/// How messages name a log folder: `%LOCALAPPDATA%/<parts>`, never the user profile path.
fn display_folder(parts: &[&str]) -> String {
    format!("%LOCALAPPDATA%/{}", parts.join("/"))
}
fn channel_log_parts() -> (&'static str, Option<String>) {
    #[cfg(windows)]
    {
        (crate::desktop_channel::APP_DIRECTORY, crate::desktop_channel::workspace_name())
    }
    #[cfg(not(windows))]
    {
        ("InstanceHarbor", None)
    }
}
/// The GUI log folder of this channel and workspace, as messages show it.
pub fn log_folder() -> String {
    let (app_directory, workspace) = channel_log_parts();
    display_folder(&log_folder_parts(app_directory, workspace.as_deref()))
}
static GLOBAL: OnceLock<Result<Diagnostics, String>> = OnceLock::new();
pub fn global() -> Result<&'static Diagnostics, String> {
    GLOBAL
        .get_or_init(|| {
            let mut root =
                PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("MissingLocalAppData")?);
            let (app_directory, workspace) = channel_log_parts();
            let parts = log_folder_parts(app_directory, workspace.as_deref());
            for part in &parts {
                root.push(part);
                match fs::create_dir(&root) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(io_trace("DiagnosticCreateDirectory", e)),
                }
                plain_path(&root)?;
            }
            Diagnostics::open(&root, display_folder(&parts))
        })
        .as_ref()
        .map_err(Clone::clone)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn each_development_workspace_writes_its_own_log_folder() {
        // Parallel development GUIs must not share one lock and one directory budget.
        assert_eq!(log_folder_parts("InstanceHarbor", None), ["InstanceHarbor", "logs"]);
        assert_eq!(
            log_folder_parts("InstanceHarbor Dev", Some("agent-73a")),
            ["InstanceHarbor Dev", "logs", "agent-73a"]
        );
    }
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
        let d = Diagnostics::open(&p, String::new()).unwrap();
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
        let file = d.path();
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
        let d = Diagnostics::open(&p, String::new()).unwrap();
        for _ in 0..5 {
            d.ipc(1, "list", "background", 1, &Ok(json!([]))).unwrap()
        }
        d.ipc(2, "list", "manual", 1, &Ok(json!([]))).unwrap();
        d.ipc(3, "list", "background", 1, &Err("ManagerBusy".into()))
            .unwrap();
        assert_eq!(fs::read_to_string(d.path()).unwrap().lines().count(), 3);
        let f = d.path();
        drop(d);
        cleanup(p, f);
    }
    #[test]
    fn background_row_count_changes_and_recovery_remain_visible() {
        let p = folder();
        let d = Diagnostics::open(&p, String::new()).unwrap();
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
        let f = d.path();
        drop(d);
        cleanup(p, f);
    }
    fn names(p: &Path) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(p)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".jsonl"))
            .collect();
        names.sort();
        names
    }
    fn remove_folder(p: PathBuf) {
        for entry in fs::read_dir(&p).unwrap() {
            fs::remove_file(entry.unwrap().path()).unwrap();
        }
        fs::remove_dir(p).unwrap()
    }
    fn aged(path: &Path, bytes: usize, seconds_ago: u64) {
        fs::write(path, vec![b'x'; bytes]).unwrap();
        let file = OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - std::time::Duration::from_secs(seconds_ago))
            .unwrap();
    }
    #[test]
    fn launch_prefix_names_the_local_start_second_and_process() {
        let prefix = launch_prefix();
        let parts: Vec<_> = prefix.split('-').collect();
        assert_eq!(parts[0], "gui");
        assert!(parts[1].len() == 8 && parts[1].chars().all(|c| c.is_ascii_digit()));
        assert!(parts[2].len() == 6 && parts[2].chars().all(|c| c.is_ascii_digit()));
        assert_eq!(parts[3], std::process::id().to_string());
        assert_eq!(parts.len(), 4);
        assert_eq!(civil(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil(1_795_586_655), (2026, 11, 25, 6, 4, 15));
    }
    #[test]
    fn full_segment_continues_in_the_next_numbered_file() {
        let p = folder();
        let d = Diagnostics::with_limit(&p, 1024, 1024 * 1024).unwrap();
        let first = d.path();
        for id in 0..20 {
            d.record(json!({"type":"test","id":id,"padding":"x".repeat(90)}))
                .unwrap();
        }
        let files = names(&p);
        assert!(files.len() >= 3, "{files:?}");
        let prefix = format!("{}-", launch_prefix());
        let mut segments: Vec<u64> = files
            .iter()
            .map(|n| {
                n.strip_prefix(&prefix).unwrap().strip_suffix(".jsonl").unwrap().parse().unwrap()
            })
            .collect();
        segments.sort();
        assert!(segments.windows(2).all(|w| w[0] < w[1]));
        assert_ne!(d.path(), first);
        let mut ids = Vec::new();
        for name in &files {
            let path = p.join(name);
            assert!(fs::metadata(&path).unwrap().len() <= 1024);
            for line in fs::read_to_string(path).unwrap().lines() {
                let event = &serde_json::from_str::<Value>(line).unwrap()["event"];
                if event["type"] == "test" {
                    ids.push(event["id"].as_u64().unwrap())
                }
            }
        }
        ids.sort();
        assert_eq!(ids, (0..20).collect::<Vec<_>>());
        assert_eq!(d.status()["available"], true);
        drop(d);
        remove_folder(p);
    }
    #[test]
    fn directory_budget_removes_the_oldest_logs_of_earlier_launches_only() {
        let p = folder();
        // An old-format log, a newer launch, and a file that is not a GUI log.
        let legacy = p.join("gui-4242-1795400000000000000-1.jsonl");
        let earlier = p.join("gui-20261124-210000-4343-1.jsonl");
        let notes = p.join("notes.txt");
        aged(&legacy, 1500, 7200);
        aged(&earlier, 1500, 3600);
        aged(&notes, 500, 10800);
        let d = Diagnostics::with_limit(&p, 1024, 4096).unwrap();
        for id in 0..8 {
            d.record(json!({"type":"test","id":id,"padding":"x".repeat(90)}))
                .unwrap();
        }
        assert!(!legacy.exists(), "the oldest earlier-launch log goes first");
        assert!(earlier.exists());
        assert!(notes.exists());
        for id in 8..20 {
            let _ = d.record(json!({"type":"test","id":id,"padding":"x".repeat(90)}));
        }
        assert!(!earlier.exists());
        assert!(notes.exists());
        // Files of the current launch are never removed, so every written event stays.
        let prefix = format!("{}-", launch_prefix());
        let own: Vec<_> = names(&p).into_iter().filter(|n| n.starts_with(&prefix)).collect();
        let written = own
            .iter()
            .flat_map(|n| fs::read_to_string(p.join(n)).unwrap().lines().map(String::from).collect::<Vec<_>>())
            .filter(|line| line.contains("\"type\":\"test\""))
            .count();
        let total: u64 = fs::read_dir(&p)
            .unwrap()
            .map(|e| e.unwrap().metadata().unwrap().len())
            .sum();
        assert!(total <= 4096, "{total}");
        assert!(written >= 8);
        if written < 20 {
            assert_eq!(d.status()["error"], "DiagnosticDirectoryBudgetReached");
        }
        drop(d);
        remove_folder(p);
    }
    #[test]
    fn periodic_background_queries_are_written_only_when_their_result_changes() {
        let p = folder();
        let d = Diagnostics::open(&p, "%LOCALAPPDATA%/InstanceHarbor/logs".into()).unwrap();
        let periodic = [
            ("core-batch-list", json!([])),
            ("core-state", json!({"reason":null})),
            ("core-check", json!({"rows":[],"checkedAt":1})),
            ("manager-connect", json!(null)),
            ("world-live-values", json!({})),
            ("world-live-clock", json!({"run":"r","value":"1"})),
            ("recovery-key-status", json!({})),
            ("core-catalog-versions", json!({"versions":[]})),
            ("core-catalog-build", json!({})),
            ("scan-policy", json!({"revision":"r"})),
        ];
        for id in 0..5 {
            for (action, reply) in &periodic {
                d.begin(id, action, "background").unwrap();
                d.ipc(id, action, "background", 1, &Ok(reply.clone())).unwrap();
            }
        }
        let lines = fs::read_to_string(d.path()).unwrap().lines().count();
        assert_eq!(lines, periodic.len() * 2);
        // A changed batch count is still evidence.
        d.ipc(9, "core-batch-list", "background", 1, &Ok(json!([{"state":"Running"}])))
            .unwrap();
        assert_eq!(fs::read_to_string(d.path()).unwrap().lines().count(), lines + 1);
        drop(d);
        remove_folder(p);
    }
    #[test]
    fn status_names_the_log_folder_of_this_channel_without_the_user_profile() {
        let p = folder();
        let d = Diagnostics::open(&p, display_folder(&["InstanceHarbor Dev", "logs", "agent-71"])).unwrap();
        assert_eq!(d.status()["folder"], "%LOCALAPPDATA%/InstanceHarbor Dev/logs/agent-71");
        assert_eq!(d.status()["available"], true);
        drop(d);
        remove_folder(p);
    }
    #[test]
    fn concurrent_sessions_keep_complete_json_and_distinct_files() {
        let p = folder();
        let sessions: Vec<_> = (0..4).map(|_| Diagnostics::open(&p, String::new()).unwrap()).collect();
        let paths: Vec<_> = sessions.iter().map(|d| d.path()).collect();
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
    fn files_that_are_not_gui_logs_are_never_removed_for_the_budget() {
        let p = folder();
        let old = p.join("old.jsonl");
        fs::write(&old, "prior evidence").unwrap();
        let d = Diagnostics::with_limit(&p, 1024, 2048).unwrap();
        let external = p.join("other-session.jsonl");
        fs::write(&external, vec![b'x'; 2000]).unwrap();
        assert!(d.begin(1, "list", "manual").is_err());
        assert_eq!(d.status()["error"], "DiagnosticDirectoryBudgetReached");
        assert_eq!(d.status()["available"], false);
        assert_eq!(fs::read_to_string(&old).unwrap(), "prior evidence");
        assert_eq!(fs::metadata(d.path()).unwrap().len(), 0);
        let f = d.path();
        drop(d);
        fs::remove_file(old).unwrap();
        fs::remove_file(external).unwrap();
        cleanup(p, f);
    }
    #[test]
    fn bounded_wait_keeps_contended_event_and_failed_sample_can_retry() {
        use std::sync::Arc;
        let p = folder();
        let d = Arc::new(Diagnostics::open(&p, String::new()).unwrap());
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
        let f = d.path();
        drop(d);
        cleanup(p, f);
    }
}
