//! The main window's background color before the page draws.
//!
//! The window is created hidden and shown once the page has applied the theme. When it
//! appears, WebView2 needs a moment to draw again and fills the window with its default
//! background until then. Without a set color that is white, which flashes under a dark
//! theme. The window and WebView2 therefore get the theme's page background (`--bg` in
//! `src/app.css`) before the window is built.
//!
//! The theme comes from the preferences read that `startup_prefetch` starts before
//! WebView2. Setup waits for it only briefly; without it, or for "system", the Windows
//! app color mode decides.
use serde_json::Value;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

/// `--bg` of the dark and light page themes.
pub(super) const DARK_BACKGROUND: (u8, u8, u8) = (0x12, 0x1c, 0x2b);
pub(super) const LIGHT_BACKGROUND: (u8, u8, u8) = (0xf4, 0xf6, 0xfa);
/// How long setup waits for the prefetched preferences. The read usually finishes before
/// setup starts; a slower one must not delay the window.
pub(super) const WAIT: Duration = Duration::from_millis(50);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Preference {
    System,
    Light,
    Dark,
}

/// The theme preference in a `preferences-snapshot` reply; None when it cannot be read.
pub(super) fn preference_of(reply: &Result<Value, String>) -> Option<Preference> {
    match reply.as_ref().ok()?.pointer("/preferences/theme")?.as_str()? {
        "system" => Some(Preference::System),
        "light" => Some(Preference::Light),
        "dark" => Some(Preference::Dark),
        _ => None,
    }
}

/// Dark unless the preference or, failing that, Windows asks for light. An unknown
/// Windows mode falls back to light, the Windows default.
pub(super) fn dark(preference: Option<Preference>, windows_dark: impl FnOnce() -> Option<bool>) -> bool {
    match preference {
        Some(Preference::Dark) => true,
        Some(Preference::Light) => false,
        Some(Preference::System) | None => windows_dark().unwrap_or(false),
    }
}

pub(super) fn background(dark: bool) -> (u8, u8, u8) {
    if dark { DARK_BACKGROUND } else { LIGHT_BACKGROUND }
}

/// Hands the prefetched theme preference from the prefetch thread to setup.
pub(super) struct Slot {
    value: Mutex<Option<Option<Preference>>>,
    ready: Condvar,
}

impl Slot {
    pub(super) const fn new() -> Self {
        Self { value: Mutex::new(None), ready: Condvar::new() }
    }

    pub(super) fn publish(&self, preference: Option<Preference>) {
        if let Ok(mut value) = self.value.lock() {
            *value = Some(preference);
            self.ready.notify_all();
        }
    }

    /// The published preference, or None when none arrives within `limit`.
    pub(super) fn wait(&self, limit: Duration) -> Option<Preference> {
        let deadline = Instant::now() + limit;
        let mut value = self.value.lock().ok()?;
        loop {
            if let Some(preference) = *value {
                return preference;
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            value = self.ready.wait_timeout(value, left).ok()?.0;
        }
    }
}

/// Whether Windows apps use dark mode (`AppsUseLightTheme` is 0); None when unreadable.
#[cfg(windows)]
pub(super) fn windows_dark() -> Option<bool> {
    use std::ffi::c_void;
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegGetValueW(
            key: isize,
            sub_key: *const u16,
            value: *const u16,
            flags: u32,
            kind: *mut u32,
            data: *mut c_void,
            size: *mut u32,
        ) -> i32;
    }
    const HKEY_CURRENT_USER: isize = 0x8000_0001_u32 as i32 as isize;
    const RRF_RT_REG_DWORD: u32 = 0x10;
    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide("AppsUseLightTheme");
    let mut data = 0_u32;
    let mut size = 4_u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut data as *mut u32).cast(),
            &mut size,
        )
    };
    (status == 0).then_some(data == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_preference_is_read_from_a_preferences_snapshot() {
        let reply = |theme: &str| Ok(json!({"preferences":{"theme":theme,"pins":[]},"revision":"r"}));
        assert_eq!(preference_of(&reply("dark")), Some(Preference::Dark));
        assert_eq!(preference_of(&reply("light")), Some(Preference::Light));
        assert_eq!(preference_of(&reply("system")), Some(Preference::System));
        assert_eq!(preference_of(&reply("blue")), None);
        assert_eq!(preference_of(&Ok(json!({"revision":"r"}))), None);
        assert_eq!(preference_of(&Err("ManagerUnavailable".into())), None);
    }

    #[test]
    fn an_explicit_theme_does_not_ask_windows() {
        assert!(dark(Some(Preference::Dark), || panic!("Windows was asked")));
        assert!(!dark(Some(Preference::Light), || panic!("Windows was asked")));
    }

    #[test]
    fn system_or_an_unread_preference_follows_windows() {
        for preference in [Some(Preference::System), None] {
            assert!(dark(preference, || Some(true)));
            assert!(!dark(preference, || Some(false)));
            assert!(!dark(preference, || None));
        }
    }

    #[test]
    fn the_colors_match_the_page_background() {
        assert_eq!(background(true), (0x12, 0x1c, 0x2b));
        assert_eq!(background(false), (0xf4, 0xf6, 0xfa));
        let css = include_str!("../../src/app.css");
        assert!(css.contains(":root{color-scheme:light;--bg:#f4f6fa;"));
        assert!(css.contains("html[data-theme=dark]{color-scheme:dark;--bg:#121c2b;"));
    }

    #[test]
    fn setup_gets_a_published_preference_and_stops_waiting_without_one() {
        let slot = Slot::new();
        let started = Instant::now();
        assert_eq!(slot.wait(Duration::from_millis(20)), None);
        assert!(started.elapsed() >= Duration::from_millis(20));
        slot.publish(Some(Preference::Dark));
        assert_eq!(slot.wait(Duration::ZERO), Some(Preference::Dark));
        let failed = Slot::new();
        failed.publish(None);
        assert_eq!(failed.wait(Duration::from_secs(5)), None);
    }

    #[test]
    fn a_preference_published_while_waiting_is_received() {
        let slot = std::sync::Arc::new(Slot::new());
        let publisher = slot.clone();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(10));
            publisher.publish(Some(Preference::Light));
        });
        assert_eq!(slot.wait(Duration::from_secs(5)), Some(Preference::Light));
        thread.join().unwrap();
    }
}
