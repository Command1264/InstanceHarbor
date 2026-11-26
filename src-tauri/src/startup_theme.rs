//! The main window's background color before the page draws.
//!
//! The window is created hidden and shown once the page has applied the theme. When it
//! appears, WebView2 needs a moment to draw again and fills the window with its default
//! background until then. Without a set color that is white, which flashes under a dark
//! theme. The window and WebView2 therefore get the theme's page background (`--bg` in
//! `src/app.css`) when the window is built, and again from `present_window` just
//! before it is shown, when the page's theme is known.
//!
//! At build time the theme comes from the preferences read that `startup_prefetch`
//! starts before WebView2, if it has finished. Setup does not wait for it: without it,
//! or for "system", the Windows app color mode decides.
use serde_json::Value;
use std::sync::OnceLock;

/// `--bg` of the dark and light page themes.
pub(super) const DARK_BACKGROUND: (u8, u8, u8) = (0x12, 0x1c, 0x2b);
pub(super) const LIGHT_BACKGROUND: (u8, u8, u8) = (0xf4, 0xf6, 0xfa);

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
pub(super) struct Slot(OnceLock<Option<Preference>>);

impl Slot {
    pub(super) const fn new() -> Self {
        Self(OnceLock::new())
    }

    pub(super) fn publish(&self, preference: Option<Preference>) {
        let _ = self.0.set(preference);
    }

    /// The preference if the read has finished; never waits.
    pub(super) fn current(&self) -> Option<Preference> {
        self.0.get().copied().flatten()
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
    fn setup_reads_the_preference_only_once_it_is_published() {
        let slot = Slot::new();
        assert_eq!(slot.current(), None);
        slot.publish(Some(Preference::Dark));
        assert_eq!(slot.current(), Some(Preference::Dark));
        let failed = Slot::new();
        failed.publish(None);
        assert_eq!(failed.current(), None);
    }
}
