//! Ends a GUI process whose window is gone but which did not exit.
//!
//! Once seen in acceptance: the window was destroyed, no exit was logged, and the
//! process kept running. Its Manager attach loop then restarted the Manager the user
//! had just quit, so the tray icon came back. This guards the outcome: after a close,
//! a process whose main window is gone is ended, and it stops attaching to the Manager
//! while it closes.
//!
//! The cause found later (#92): tray Quit and `app-exit` also sent WM_CLOSE to Tao's
//! visible tool window that carries the event loop's messages, so the loop never saw
//! the exit request. `manager_tray::close_gui_windows` now skips tool windows. The
//! watchdog stays for a close that still gets stuck.
use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

/// How long a normal close may take before the process is treated as stuck.
const GRACE: Duration = Duration::from_secs(10);
/// The diagnostic record must not keep a stuck process alive.
const RECORD_LIMIT: Duration = Duration::from_secs(1);

static CLOSING: AtomicBool = AtomicBool::new(false);

/// True while the main window is closing; the Manager attach loop pauses then.
pub fn closing() -> bool {
    CLOSING.load(Ordering::SeqCst)
}

/// Called on every close signal of the main window. Starts at most one watchdog.
pub fn arm() {
    if !begin(&CLOSING) {
        return;
    }
    let spawned = std::thread::Builder::new().name("gui-exit-watchdog".into()).spawn(|| {
        std::thread::sleep(GRACE);
        if settle(&CLOSING, has_visible_window) {
            record_and_terminate();
        }
    });
    if let Err(error) = spawned {
        eprintln!("GUI exit watchdog could not start: {error}");
    }
}

fn begin(closing: &AtomicBool) -> bool {
    !closing.swap(true, Ordering::SeqCst)
}

/// `true`: end the process. A window still shown means the close did not happen,
/// so the watchdog stands down and attaching resumes.
fn settle(closing: &AtomicBool, has_window: impl Fn() -> bool) -> bool {
    if has_window() {
        closing.store(false, Ordering::SeqCst);
        false
    } else {
        true
    }
}

fn record_and_terminate() -> ! {
    let (done, wait) = mpsc::channel();
    std::thread::spawn(move || {
        if let Ok(log) = instanceharbor::diagnostics::global() {
            let _ = log.record(json!({"type":"gui-exit-watchdog","graceMs":GRACE.as_millis() as u64}));
        }
        let _ = done.send(());
    });
    let _ = wait.recv_timeout(RECORD_LIMIT);
    // TerminateProcess, not process::exit: a stuck process may hold the locks that
    // exit handlers would wait for.
    unsafe {
        TerminateProcess(GetCurrentProcess(), 0);
    }
    std::process::abort()
}

/// Remembers the main window, the only window whose presence means the GUI is open.
/// The process also owns helper windows (Tao's event target reports itself visible),
/// so they cannot be used to tell whether the close happened.
pub fn watch(window: isize) {
    MAIN_WINDOW.store(window, Ordering::SeqCst);
}

static MAIN_WINDOW: AtomicIsize = AtomicIsize::new(0);

fn has_visible_window() -> bool {
    let window = MAIN_WINDOW.load(Ordering::SeqCst) as RawWindow;
    !window.is_null() && unsafe { IsWindow(window) != 0 && IsWindowVisible(window) != 0 }
}

type RawWindow = *mut std::ffi::c_void;

#[link(name = "user32")]
unsafe extern "system" {
    fn IsWindow(window: RawWindow) -> i32;
    fn IsWindowVisible(window: RawWindow) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcess() -> *mut std::ffi::c_void;
    fn TerminateProcess(process: *mut std::ffi::c_void, code: u32) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn only_the_first_close_request_starts_a_watchdog() {
        let closing = AtomicBool::new(false);
        assert!(begin(&closing));
        assert!(!begin(&closing));
        assert!(closing.load(Ordering::SeqCst));
    }

    #[test]
    fn a_window_still_shown_cancels_the_forced_exit() {
        let closing = AtomicBool::new(true);
        assert!(!settle(&closing, || true));
        // Attaching to the Manager resumes and a later close can arm a new watchdog.
        assert!(!closing.load(Ordering::SeqCst));
        assert!(begin(&closing));
    }

    #[test]
    fn a_process_left_without_windows_is_ended() {
        let closing = AtomicBool::new(true);
        assert!(settle(&closing, || false));
        assert!(closing.load(Ordering::SeqCst));
    }
}
