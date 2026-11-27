//! Keeps a GUI that never shows its main window from blocking later launches (#117).
//!
//! The GUI lock holder publishes who it is in a small named shared-memory record:
//! its process id, its creation time, which workspace it serves, and whether it has
//! shown its main window. A new GUI that cannot take the lock reads the record. A
//! holder that started more than `TAKEOVER_AGE` ago and never showed its window is
//! asked to quit, then ended after `TAKEOVER_GRACE`. The holder also ends itself when
//! it has not shown its window `STARTUP_DEADLINE` after it started.
use std::time::Duration;

/// A holder younger than this may still be starting normally.
pub const TAKEOVER_AGE: Duration = Duration::from_secs(15);
/// How long a holder asked to quit may take before it is ended.
pub const TAKEOVER_GRACE: Duration = Duration::from_secs(2);
/// A GUI that has not shown its main window this long after it started ends itself.
pub const STARTUP_DEADLINE: Duration = Duration::from_secs(20);

const MAGIC: u32 = 0x4948_4755; // "IHGU"
const RECORD_SIZE: usize = 24;
const REVEALED_OFFSET: usize = 20;

/// What the lock holder published about itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Owner {
    pub pid: u32,
    /// FILETIME ticks (100 ns since 1601) of the holder's process creation.
    pub created: u64,
    /// Hash of the workspace path the holder serves.
    pub workspace: u32,
    /// True once the holder has shown its main window. A window hidden later (for
    /// example in the notification area) still counts as shown.
    pub revealed: bool,
}

pub fn encode(owner: &Owner) -> [u8; RECORD_SIZE] {
    let mut bytes = [0u8; RECORD_SIZE];
    bytes[0..4].copy_from_slice(&MAGIC.to_le_bytes());
    bytes[4..8].copy_from_slice(&owner.pid.to_le_bytes());
    bytes[8..16].copy_from_slice(&owner.created.to_le_bytes());
    bytes[16..20].copy_from_slice(&owner.workspace.to_le_bytes());
    bytes[REVEALED_OFFSET..].copy_from_slice(&u32::from(owner.revealed).to_le_bytes());
    bytes
}

pub fn decode(bytes: &[u8]) -> Option<Owner> {
    if bytes.len() < RECORD_SIZE {
        return None;
    }
    let word =
        |at: usize| u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    let pid = word(4);
    let created = u64::from(word(8)) | (u64::from(word(12)) << 32);
    if word(0) != MAGIC || pid == 0 || created == 0 {
        return None;
    }
    Some(Owner {
        pid,
        created,
        workspace: word(16),
        revealed: word(REVEALED_OFFSET) != 0,
    })
}

/// What a GUI that found the lock taken does.
#[derive(Debug, PartialEq, Eq)]
pub enum Takeover {
    /// Show the holder's window, as before #117.
    Focus,
    /// The holder is stuck before its first window: end it and open normally.
    Replace(Owner),
}

/// `now` and `owner.created` are FILETIME ticks. `verified`: the holder's process is
/// this installation's executable and was created at `owner.created`.
pub fn decide(owner: Option<Owner>, now: u64, workspace: u32, verified: bool) -> Takeover {
    let Some(owner) = owner else {
        return Takeover::Focus;
    };
    let age = Duration::from_nanos(now.saturating_sub(owner.created).saturating_mul(100));
    if verified && owner.workspace == workspace && !owner.revealed && age > TAKEOVER_AGE {
        Takeover::Replace(owner)
    } else {
        Takeover::Focus
    }
}

/// The holder ends itself when this returns true.
pub fn deadline_passed(since_start: Duration, revealed: bool) -> bool {
    !revealed && since_start >= STARTUP_DEADLINE
}

/// How long the deadline thread sleeps before it checks.
pub fn deadline_wait(since_start: Duration) -> Duration {
    STARTUP_DEADLINE.saturating_sub(since_start)
}

/// A stable 32-bit key for a workspace path; case, separators and the `\\?\` prefix
/// do not matter.
pub fn workspace_key(path: &str) -> u32 {
    let text = path.replace('/', "\\");
    let text = text
        .strip_prefix(r"\\?\")
        .unwrap_or(&text)
        .trim_end_matches('\\')
        .to_lowercase();
    let mut hash = 0x811c_9dc5u32;
    for byte in text.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

#[cfg(windows)]
pub use platform::*;

#[cfg(windows)]
mod platform {
    use super::*;
    use serde_json::json;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};

    type Handle = *mut c_void;

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateFileMappingW(
            file: Handle,
            attributes: Handle,
            protect: u32,
            high: u32,
            low: u32,
            name: *const u16,
        ) -> Handle;
        fn OpenFileMappingW(access: u32, inherit: i32, name: *const u16) -> Handle;
        fn MapViewOfFile(
            mapping: Handle,
            access: u32,
            high: u32,
            low: u32,
            size: usize,
        ) -> *mut c_void;
        fn UnmapViewOfFile(view: *const c_void) -> i32;
        fn CreateEventW(attributes: Handle, manual: i32, initial: i32, name: *const u16) -> Handle;
        fn OpenEventW(access: u32, inherit: i32, name: *const u16) -> Handle;
        fn SetEvent(event: Handle) -> i32;
        fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn GetCurrentProcess() -> Handle;
        fn GetProcessTimes(
            process: Handle,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
        fn QueryFullProcessImageNameW(
            process: Handle,
            flags: u32,
            name: *mut u16,
            size: *mut u32,
        ) -> i32;
        fn TerminateProcess(process: Handle, code: u32) -> i32;
        fn GetSystemTimeAsFileTime(time: *mut FileTime);
        fn CloseHandle(handle: Handle) -> i32;
    }

    const PAGE_READWRITE: u32 = 0x04;
    const FILE_MAP_READ: u32 = 0x04;
    const FILE_MAP_WRITE: u32 = 0x02;
    const EVENT_MODIFY_STATE: u32 = 0x0002;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const PROCESS_TERMINATE: u32 = 0x0001;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const WAIT_OBJECT_0: u32 = 0;
    const INFINITE: u32 = u32::MAX;

    /// The published record; null until `publish` succeeded.
    static RECORD: AtomicPtr<u8> = AtomicPtr::new(std::ptr::null_mut());

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain([0]).collect()
    }
    fn record_name(lock: &str) -> String {
        format!("{lock}-Owner")
    }
    fn show_name(lock: &str) -> String {
        format!("{lock}-Show")
    }
    fn quit_name(lock: &str) -> String {
        format!("{lock}-Quit")
    }
    fn ticks(time: FileTime) -> u64 {
        (u64::from(time.high) << 32) | u64::from(time.low)
    }
    pub fn now() -> u64 {
        let mut time = FileTime::default();
        unsafe { GetSystemTimeAsFileTime(&mut time) };
        ticks(time)
    }
    fn creation(process: Handle) -> Option<u64> {
        let mut times = [FileTime::default(); 4];
        let [creation, exit, kernel, user] = &mut times;
        (unsafe { GetProcessTimes(process, creation, exit, kernel, user) } != 0)
            .then(|| ticks(times[0]))
    }
    fn image(process: Handle) -> Option<std::path::PathBuf> {
        let mut name = vec![0u16; 32768];
        let mut size = name.len() as u32;
        if unsafe { QueryFullProcessImageNameW(process, 0, name.as_mut_ptr(), &mut size) } == 0 {
            return None;
        }
        std::path::PathBuf::from(String::from_utf16(&name[..size as usize]).ok()?)
            .canonicalize()
            .ok()
    }

    /// The key of the workspace this GUI serves, from `IH_WORKSPACE` or the default.
    pub fn current_workspace(default: impl FnOnce() -> Option<std::path::PathBuf>) -> u32 {
        let path = std::env::var_os("IH_WORKSPACE")
            .map(std::path::PathBuf::from)
            .or_else(default);
        // The Manager and a launcher may spell the same folder differently.
        path.map(|path| workspace_key(&path.canonicalize().unwrap_or(path).to_string_lossy()))
            .unwrap_or(0)
    }

    /// Publishes this GUI as the lock holder and listens for a quit request.
    /// Called once, right after the lock was taken and the workspace bound.
    /// `listen` is false only for a development-build simulation of a GUI that does not
    /// answer the quit request.
    pub fn publish(lock: &str, workspace: u32, listen: bool) -> Result<(), String> {
        let created = creation(unsafe { GetCurrentProcess() }).ok_or("ProcessTimesUnavailable")?;
        let name = wide(&record_name(lock));
        // INVALID_HANDLE_VALUE: backed by the paging file, gone when the last handle closes.
        let mapping = unsafe {
            CreateFileMappingW(
                -1isize as Handle,
                std::ptr::null_mut(),
                PAGE_READWRITE,
                0,
                RECORD_SIZE as u32,
                name.as_ptr(),
            )
        };
        if mapping.is_null() {
            return Err(format!(
                "OwnerRecordCreate ({})",
                std::io::Error::last_os_error()
            ));
        }
        let view =
            unsafe { MapViewOfFile(mapping, FILE_MAP_READ | FILE_MAP_WRITE, 0, 0, RECORD_SIZE) }
                as *mut u8;
        if view.is_null() {
            let error = std::io::Error::last_os_error();
            unsafe { CloseHandle(mapping) };
            return Err(format!("OwnerRecordMap ({error})"));
        }
        let bytes = encode(&Owner {
            pid: std::process::id(),
            created,
            workspace,
            revealed: revealed(),
        });
        // The mapping and view stay open until the process ends.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), view, RECORD_SIZE) };
        RECORD.store(view, Ordering::SeqCst);
        if revealed() {
            mark_revealed();
        }
        listen_for_show(lock);
        if !listen {
            return Ok(());
        }
        let quit = wide(&quit_name(lock));
        let event = unsafe { CreateEventW(std::ptr::null_mut(), 1, 0, quit.as_ptr()) } as usize;
        if event == 0 {
            return Err(format!(
                "QuitEventCreate ({})",
                std::io::Error::last_os_error()
            ));
        }
        std::thread::Builder::new()
            .name("gui-takeover-listener".into())
            .spawn(move || {
                if unsafe { WaitForSingleObject(event as Handle, INFINITE) } != WAIT_OBJECT_0 {
                    return;
                }
                // A window shown in the meantime has content the user may be using; such a
                // GUI is never asked again (`decide`), so the listener ends.
                if !revealed() {
                    record_and_terminate(json!({"type":"gui-takeover-quit","stage":crate::startup_timing::last_mark()}));
                }
            })
            .map(|_| ())
            .map_err(|error| format!("QuitListener ({error})"))
    }

    /// A later launch asks this GUI to show its window, hidden in fast open mode (#56).
    /// Auto-reset: each request shows the window once.
    fn listen_for_show(lock: &str) {
        let name = wide(&show_name(lock));
        let event = unsafe { CreateEventW(std::ptr::null_mut(), 0, 0, name.as_ptr()) } as usize;
        if event == 0 {
            eprintln!(
                "GUI show event unavailable: {}",
                std::io::Error::last_os_error()
            );
            return;
        }
        let spawned = std::thread::Builder::new()
            .name("gui-show-listener".into())
            .spawn(move || {
                while unsafe { WaitForSingleObject(event as Handle, INFINITE) } == WAIT_OBJECT_0 {
                    crate::fast_open::show();
                }
            });
        if let Err(error) = spawned {
            eprintln!("GUI show listener could not start: {error}");
        }
    }

    /// Asks the lock holder to show its window; nothing happens when it is not hidden.
    pub fn request_show(lock: &str) {
        let name = wide(&show_name(lock));
        let event = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr()) };
        if !event.is_null() {
            unsafe {
                SetEvent(event);
                CloseHandle(event);
            }
        }
    }

    static REVEALED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    pub fn revealed() -> bool {
        REVEALED.load(Ordering::SeqCst)
    }

    /// Called when the main window is first shown.
    pub fn mark_revealed() {
        REVEALED.store(true, Ordering::SeqCst);
        let view = RECORD.load(Ordering::SeqCst);
        if !view.is_null() {
            let flag = unsafe { &*(view.add(REVEALED_OFFSET) as *const AtomicU32) };
            flag.store(1, Ordering::SeqCst);
        }
    }

    /// The record the lock holder published, if any.
    pub fn read(lock: &str) -> Option<Owner> {
        let name = wide(&record_name(lock));
        let mapping = unsafe { OpenFileMappingW(FILE_MAP_READ, 0, name.as_ptr()) };
        if mapping.is_null() {
            return None;
        }
        let view = unsafe { MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, RECORD_SIZE) } as *const u8;
        let owner = (!view.is_null()).then(|| {
            let mut bytes = [0u8; RECORD_SIZE];
            unsafe { std::ptr::copy_nonoverlapping(view, bytes.as_mut_ptr(), RECORD_SIZE) };
            unsafe { UnmapViewOfFile(view.cast()) };
            decode(&bytes)
        });
        unsafe { CloseHandle(mapping) };
        owner.flatten()
    }

    /// An open handle to the holder's process, after checking it is this installation's
    /// executable and was created when the record says. Otherwise None.
    pub struct Holder(Handle);
    impl Drop for Holder {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }
    pub fn open_verified(owner: &Owner) -> Option<Holder> {
        if owner.pid == std::process::id() {
            return None;
        }
        let process = unsafe {
            OpenProcess(
                SYNCHRONIZE | PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                owner.pid,
            )
        };
        if process.is_null() {
            return None;
        }
        let holder = Holder(process);
        let executable = std::env::current_exe()
            .and_then(|path| path.canonicalize())
            .ok()?;
        (creation(holder.0) == Some(owner.created) && image(holder.0) == Some(executable))
            .then_some(holder)
    }

    /// How the stuck holder ended.
    #[derive(Debug, PartialEq, Eq)]
    pub enum Ended {
        /// It quit when asked.
        Quit,
        /// It was terminated after the grace period.
        Terminated,
        /// It showed its window meanwhile and keeps running.
        Revealed,
        /// It is still running; the lock stays taken.
        Failed,
    }

    impl Ended {
        pub fn name(&self) -> &'static str {
            match self {
                Ended::Quit => "quit",
                Ended::Terminated => "terminated",
                Ended::Revealed => "revealed",
                Ended::Failed => "failed",
            }
        }
    }

    /// Asks the holder to quit, then ends it once `TAKEOVER_GRACE` has passed.
    pub fn end(lock: &str, holder: &Holder) -> Ended {
        let quit = wide(&quit_name(lock));
        let event = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, quit.as_ptr()) };
        if !event.is_null() {
            unsafe {
                SetEvent(event);
                CloseHandle(event);
            }
        }
        if unsafe { WaitForSingleObject(holder.0, TAKEOVER_GRACE.as_millis() as u32) }
            == WAIT_OBJECT_0
        {
            return Ended::Quit;
        }
        // Do not end a GUI that showed its window while it was asked.
        if read(lock).is_some_and(|owner| owner.revealed) {
            return Ended::Revealed;
        }
        unsafe { TerminateProcess(holder.0, 1) };
        if unsafe { WaitForSingleObject(holder.0, TAKEOVER_GRACE.as_millis() as u32) }
            == WAIT_OBJECT_0
        {
            Ended::Terminated
        } else {
            Ended::Failed
        }
    }

    /// Ends this process if it has not shown its main window by `STARTUP_DEADLINE`.
    pub fn start_deadline(since_start: impl Fn() -> Duration + Send + 'static) {
        let spawned = std::thread::Builder::new()
            .name("gui-startup-deadline".into())
            .spawn(move || loop {
                let wait = deadline_wait(since_start());
                if !wait.is_zero() {
                    std::thread::sleep(wait);
                    continue;
                }
                if deadline_passed(since_start(), revealed()) {
                    record_and_terminate(json!({
                        "type":"gui-startup-deadline",
                        "deadlineMs":STARTUP_DEADLINE.as_millis() as u64,
                        "stage":crate::startup_timing::last_mark(),
                        "webview":crate::webview_events::snapshot(),
                    }));
                }
                return;
            });
        if let Err(error) = spawned {
            eprintln!("GUI startup deadline could not start: {error}");
        }
    }

    fn record_and_terminate(event: serde_json::Value) -> ! {
        let (done, wait) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            if let Ok(log) = instanceharbor::diagnostics::global() {
                let _ = log.record(event);
            }
            let _ = done.send(());
        });
        // The diagnostic record must not keep a stuck process alive.
        let _ = wait.recv_timeout(Duration::from_secs(1));
        unsafe { TerminateProcess(GetCurrentProcess(), 1) };
        std::process::abort()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: u64 = 10_000_000;

    fn owner(revealed: bool) -> Owner {
        Owner {
            pid: 4242,
            created: 1_000 * SECOND,
            workspace: 7,
            revealed,
        }
    }

    #[test]
    fn the_record_round_trips_and_rejects_foreign_bytes() {
        for revealed in [false, true] {
            assert_eq!(decode(&encode(&owner(revealed))), Some(owner(revealed)));
        }
        assert_eq!(decode(&[0u8; RECORD_SIZE]), None);
        assert_eq!(decode(&encode(&owner(false))[..RECORD_SIZE - 1]), None);
        let mut zero_pid = encode(&owner(false));
        zero_pid[4..8].fill(0);
        assert_eq!(decode(&zero_pid), None);
    }

    #[test]
    fn a_verified_holder_without_a_window_after_fifteen_seconds_is_replaced() {
        let now = owner(false).created + 16 * SECOND;
        assert_eq!(
            decide(Some(owner(false)), now, 7, true),
            Takeover::Replace(owner(false))
        );
    }

    #[test]
    fn a_holder_still_inside_fifteen_seconds_is_left_starting() {
        let created = owner(false).created;
        assert_eq!(
            decide(Some(owner(false)), created + 15 * SECOND, 7, true),
            Takeover::Focus
        );
        assert_eq!(
            decide(Some(owner(false)), created + 3 * SECOND, 7, true),
            Takeover::Focus
        );
        // A clock that went backwards is not an old holder.
        assert_eq!(
            decide(Some(owner(false)), created - SECOND, 7, true),
            Takeover::Focus
        );
    }

    #[test]
    fn a_holder_that_showed_its_window_is_focused_even_when_hidden_later() {
        let now = owner(true).created + 3_600 * SECOND;
        assert_eq!(decide(Some(owner(true)), now, 7, true), Takeover::Focus);
    }

    #[test]
    fn only_this_installation_and_workspace_are_replaced() {
        let now = owner(false).created + 60 * SECOND;
        // Another executable, or a recycled process id with another creation time.
        assert_eq!(decide(Some(owner(false)), now, 7, false), Takeover::Focus);
        assert_eq!(decide(Some(owner(false)), now, 8, true), Takeover::Focus);
        // An older GUI that publishes no record keeps the old behavior.
        assert_eq!(decide(None, now, 7, true), Takeover::Focus);
    }

    #[test]
    fn the_startup_deadline_is_twenty_seconds_without_a_window() {
        assert!(!deadline_passed(Duration::from_millis(19_999), false));
        assert!(deadline_passed(STARTUP_DEADLINE, false));
        assert!(deadline_passed(Duration::from_secs(45), false));
        assert!(!deadline_passed(Duration::from_secs(45), true));
        assert_eq!(
            deadline_wait(Duration::from_secs(5)),
            Duration::from_secs(15)
        );
        assert_eq!(deadline_wait(Duration::from_secs(25)), Duration::ZERO);
    }

    #[test]
    fn the_deadline_leaves_room_for_a_takeover() {
        // A new GUI waits for a holder older than TAKEOVER_AGE; the holder ends itself
        // at STARTUP_DEADLINE. Both must fit, with the grace period, in a short wait.
        assert_eq!(TAKEOVER_AGE, Duration::from_secs(15));
        assert_eq!(TAKEOVER_GRACE, Duration::from_secs(2));
        assert!(TAKEOVER_AGE < STARTUP_DEADLINE);
    }

    #[test]
    fn workspace_keys_ignore_case_and_separators() {
        assert_eq!(
            workspace_key(r"C:\Work\Space"),
            workspace_key("c:/work/space/")
        );
        assert_eq!(
            workspace_key(r"C:\Work\Space"),
            workspace_key(r"\\?\C:\Work\Space")
        );
        assert_ne!(
            workspace_key(r"C:\Work\Space"),
            workspace_key(r"C:\Work\Other")
        );
    }
}
