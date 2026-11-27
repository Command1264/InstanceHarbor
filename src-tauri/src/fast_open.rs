//! Fast open (#56): with the preference on, X hides the main window instead of ending the
//! GUI, so the next open only shows it again.
//!
//! While hidden, WebView2 is made invisible and suspended, so the page's polling stops
//! (it also checks `document.visibilityState`). A GUI hidden for `HIDE_LIMIT` ends itself.
//! The Manager's Quit, `app-exit`, Setup and the in-app update all end a hidden GUI through
//! its exit event (`gui_singleton::exit_event`), since a hidden window gets no WM_CLOSE.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

/// How long a hidden GUI stays before it ends itself and frees its memory.
pub const HIDE_LIMIT: Duration = Duration::from_secs(30 * 60);

/// What a close request of the main window does.
#[derive(Debug, PartialEq, Eq)]
pub enum Close {
    Hide,
    Exit,
}

/// `revealed`: the window was shown at least once. A window that never showed, or a close
/// that comes with the Manager's Quit, always ends the GUI.
pub fn on_close(fast_open: bool, quitting: bool, revealed: bool) -> Close {
    if fast_open && !quitting && revealed {
        Close::Hide
    } else {
        Close::Exit
    }
}

/// Counts hides and shows, so a timer started by an earlier hide does nothing.
pub struct Hidden {
    hidden: AtomicBool,
    generation: AtomicU64,
}

impl Hidden {
    pub const fn new() -> Self {
        Self {
            hidden: AtomicBool::new(false),
            generation: AtomicU64::new(0),
        }
    }
    /// Marks the window hidden; returns the generation the hide timer belongs to.
    pub fn hide(&self) -> u64 {
        self.hidden.store(true, Ordering::SeqCst);
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }
    /// True when the window was hidden and is now shown.
    pub fn show(&self) -> bool {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.hidden.swap(false, Ordering::SeqCst)
    }
    pub fn is_hidden(&self) -> bool {
        self.hidden.load(Ordering::SeqCst)
    }
    /// The hide timer of `generation` ends the GUI only if nothing happened since.
    pub fn expired(&self, generation: u64) -> bool {
        self.is_hidden() && self.generation.load(Ordering::SeqCst) == generation
    }
}

/// Development builds may shorten the limit to test it (`IH_DEV_FAST_OPEN_HIDE_MS`).
pub fn hide_limit(development_override: Option<&str>) -> Duration {
    development_override
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|ms| *ms > 0)
        .map(Duration::from_millis)
        .unwrap_or(HIDE_LIMIT)
}

#[cfg(windows)]
pub use platform::*;

#[cfg(windows)]
mod platform {
    use super::*;
    use instanceharbor::gui_singleton;
    use serde_json::json;
    use std::sync::OnceLock;
    use tauri::Manager;

    static HIDDEN: Hidden = Hidden::new();
    static QUITTING: AtomicBool = AtomicBool::new(false);
    static LIMIT: OnceLock<Duration> = OnceLock::new();
    static WINDOW: OnceLock<tauri::WebviewWindow> = OnceLock::new();

    fn record(event: serde_json::Value) {
        if let Ok(log) = instanceharbor::diagnostics::global() {
            if let Err(error) = log.record(event) {
                eprintln!("GUI diagnostic write failed: {error}");
            }
        }
    }

    /// Reads the development override once, before threads start; release builds ignore it.
    pub fn configure() {
        #[cfg(feature = "development-channel")]
        let value = std::env::var("IH_DEV_FAST_OPEN_HIDE_MS").ok();
        #[cfg(feature = "development-channel")]
        std::env::remove_var("IH_DEV_FAST_OPEN_HIDE_MS");
        #[cfg(not(feature = "development-channel"))]
        let value: Option<String> = None;
        let _ = LIMIT.set(hide_limit(value.as_deref()));
    }

    /// True once the Manager asked this GUI to exit; a close then always ends it.
    pub fn quitting() -> bool {
        QUITTING.load(Ordering::SeqCst)
    }

    pub fn enabled() -> bool {
        std::env::var_os("IH_WORKSPACE")
            .is_some_and(|root| instanceharbor::fast_open_enabled(std::path::Path::new(&root)))
    }

    /// Remembers the main window and waits for the Manager's exit request.
    pub fn install(window: &tauri::WebviewWindow) {
        let _ = WINDOW.set(window.clone());
        let event = match gui_singleton::exit_event() {
            Ok(event) => event,
            Err(error) => {
                eprintln!("GUI exit event unavailable: {error}");
                record(
                    json!({"type":"gui-exit-event","error":instanceharbor::diagnostics::safe_error(&error.to_string())}),
                );
                return;
            }
        };
        let app = window.app_handle().clone();
        let spawned = std::thread::Builder::new()
            .name("gui-exit-request".into())
            .spawn(move || {
                if gui_singleton::wait(&event, u32::MAX) {
                    QUITTING.store(true, Ordering::SeqCst);
                    record(json!({"type":"gui-exit-requested","hidden":HIDDEN.is_hidden()}));
                    crate::exit_watchdog::arm();
                    app.exit(0);
                }
            });
        if let Err(error) = spawned {
            eprintln!("GUI exit request listener could not start: {error}");
        }
    }

    /// X in fast open mode: hide, suspend WebView2 and start the hide timer.
    pub fn hide(window: &tauri::WebviewWindow) {
        let generation = HIDDEN.hide();
        if let Err(error) = window.hide() {
            eprintln!("Could not hide the main window: {error}");
        }
        if let Err(error) =
            window.with_webview(|webview| crate::webview_start::suspend(webview, true))
        {
            eprintln!("Could not suspend WebView2: {error}");
        }
        let limit = *LIMIT.get_or_init(|| HIDE_LIMIT);
        record(json!({"type":"gui-hidden","limitMs":limit.as_millis() as u64}));
        let app = window.app_handle().clone();
        let spawned = std::thread::Builder::new()
            .name("gui-hide-limit".into())
            .spawn(move || {
                std::thread::sleep(limit);
                if HIDDEN.expired(generation) {
                    record(json!({"type":"gui-hidden-exit","limitMs":limit.as_millis() as u64}));
                    QUITTING.store(true, Ordering::SeqCst);
                    crate::exit_watchdog::arm();
                    app.exit(0);
                }
            });
        if let Err(error) = spawned {
            eprintln!("GUI hide timer could not start: {error}");
        }
    }

    /// Another launch asked for the window (startup_guard's show event).
    pub fn show() {
        let Some(window) = WINDOW.get() else { return };
        if !HIDDEN.show() {
            return;
        }
        if let Err(error) =
            window.with_webview(|webview| crate::webview_start::suspend(webview, false))
        {
            eprintln!("Could not resume WebView2: {error}");
        }
        let shown = window
            .show()
            .and_then(|_| window.unminimize())
            .and_then(|_| window.set_focus());
        if let Err(error) = shown {
            eprintln!("Could not show the main window: {error}");
        }
        record(json!({"type":"gui-shown"}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x_hides_only_in_fast_open_mode_after_the_window_was_shown() {
        assert_eq!(on_close(true, false, true), Close::Hide);
        assert_eq!(on_close(false, false, true), Close::Exit);
        // A window that never showed has nothing to keep (#117 handles a stuck start).
        assert_eq!(on_close(true, false, false), Close::Exit);
        // The Manager's Quit ends the GUI even in fast open mode.
        assert_eq!(on_close(true, true, true), Close::Exit);
    }

    #[test]
    fn a_hidden_gui_ends_after_the_limit_unless_shown_or_hidden_again() {
        let hidden = Hidden::new();
        let first = hidden.hide();
        assert!(hidden.expired(first));
        assert!(hidden.show());
        assert!(!hidden.expired(first));
        // Showing a window that is not hidden changes nothing.
        assert!(!hidden.show());
        let second = hidden.hide();
        let third = hidden.hide();
        assert!(!hidden.expired(second));
        assert!(hidden.expired(third));
    }

    #[test]
    fn the_hide_limit_is_thirty_minutes_unless_a_test_shortens_it() {
        assert_eq!(hide_limit(None), Duration::from_secs(1800));
        assert_eq!(hide_limit(Some("5000")), Duration::from_secs(5));
        assert_eq!(hide_limit(Some("0")), HIDE_LIMIT);
        assert_eq!(hide_limit(Some("soon")), HIDE_LIMIT);
    }
}
