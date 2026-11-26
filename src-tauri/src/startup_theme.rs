//! The main window's theme and background color before the page draws.
//!
//! The window is created hidden and shown once the page has applied the theme. While it
//! is hidden, WebView2 already renders the page once, and when the window appears it
//! shows that frame before it draws again. The page's first theme comes from
//! `prefers-color-scheme` (its preferences are not read yet), which WebView2 takes from
//! the window's theme, so a window built with the Windows mode drew a light first frame
//! under a dark preference on light Windows; it flashed white for about ten frames when
//! the window opened (#94). Setup therefore gives the window the saved theme, read
//! directly from the workspace's `preferences.json` so it never waits for the Manager.
//!
//! Until WebView2 has drawn at all it fills the window with its default background,
//! which is white unless set. The window and WebView2 therefore also get the theme's
//! page background (`--bg` in `src/app.css`) when the window is built, and again from
//! `present_window` just before it is shown, when the page's theme is known.
//!
//! Without a readable saved theme, or for "system", the Windows app color mode decides.
use serde_json::Value;
use std::path::Path;

/// `--bg` of the dark and light page themes.
pub(super) const DARK_BACKGROUND: (u8, u8, u8) = (0x12, 0x1c, 0x2b);
pub(super) const LIGHT_BACKGROUND: (u8, u8, u8) = (0xf4, 0xf6, 0xfa);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Preference {
    System,
    Light,
    Dark,
}

/// The theme preference in a parsed `preferences.json`; None when it names none.
fn preference_in(preferences: &Value) -> Option<Preference> {
    match preferences.get("theme")?.as_str()? {
        "system" => Some(Preference::System),
        "light" => Some(Preference::Light),
        "dark" => Some(Preference::Dark),
        _ => None,
    }
}

/// The theme saved in the workspace's `preferences.json`, read only. Any problem
/// (no file, a link, too large, not JSON) means "no saved theme".
#[cfg(windows)]
pub(super) fn saved(workspace: &Path) -> Option<Preference> {
    use std::os::windows::fs::MetadataExt;
    const REPARSE_POINT: u32 = 0x400;
    let path = workspace.join("preferences.json");
    let metadata = std::fs::symlink_metadata(&path).ok()?;
    if !metadata.is_file() || metadata.file_attributes() & REPARSE_POINT != 0 || metadata.len() > 1 << 20 {
        return None;
    }
    preference_in(&serde_json::from_slice(&std::fs::read(path).ok()?).ok()?)
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
    fn the_preference_is_read_from_preferences_json() {
        let preferences = |theme: &str| json!({"theme":theme,"pins":[],"language":"system"});
        assert_eq!(preference_in(&preferences("dark")), Some(Preference::Dark));
        assert_eq!(preference_in(&preferences("light")), Some(Preference::Light));
        assert_eq!(preference_in(&preferences("system")), Some(Preference::System));
        assert_eq!(preference_in(&preferences("blue")), None);
        assert_eq!(preference_in(&json!({"pins":[]})), None);
        assert_eq!(preference_in(&json!({"theme":1})), None);
    }

    #[cfg(windows)]
    #[test]
    fn a_saved_theme_is_read_and_anything_unreadable_is_none() {
        let workspace = std::env::temp_dir().join(format!("ih-startup-theme-{}", std::process::id()));
        std::fs::create_dir_all(&workspace).unwrap();
        let file = workspace.join("preferences.json");
        assert_eq!(saved(&workspace), None);
        std::fs::write(&file, r#"{"theme":"dark","pins":[]}"#).unwrap();
        assert_eq!(saved(&workspace), Some(Preference::Dark));
        std::fs::write(&file, r#"{"theme":"light"}"#).unwrap();
        assert_eq!(saved(&workspace), Some(Preference::Light));
        std::fs::write(&file, "not json").unwrap();
        assert_eq!(saved(&workspace), None);
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        assert_eq!(saved(&workspace), None);
        std::fs::remove_dir_all(&workspace).unwrap();
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
}
