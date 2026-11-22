//! Notification-area icon for the main InstanceHarbor window.
//!
//! Closing the window can hide it here instead of ending the GUI. Servers and
//! their Hosts never depend on the GUI process, so ending the GUI leaves them running.
use instanceharbor_v2::desktop_channel::TITLE;
use instanceharbor_v2::diagnostics::{self, safe_error};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{
    menu::{Menu, MenuEvent, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Window, WindowEvent, Wry,
};

const MAIN_WINDOW: &str = "main";
const SHOW_ID: &str = "tray-show";
const QUIT_ID: &str = "tray-quit";
static QUITTING: AtomicBool = AtomicBool::new(false);

pub struct Tray {
    icon: TrayIcon<Wry>,
    show: MenuItem<Wry>,
    quit: MenuItem<Wry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    TraditionalChinese,
    English,
}

impl Language {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "zh-TW" => Ok(Self::TraditionalChinese),
            "en" => Ok(Self::English),
            _ => Err("InvalidTrayLanguage".into()),
        }
    }
}

// Follow the platform convention: short verbs, no product name. The settings help
// text explains that servers keep running after the app quits.
pub fn show_label(language: Language) -> &'static str {
    match language {
        Language::TraditionalChinese => "顯示應用程式",
        Language::English => "Show app",
    }
}

pub fn quit_label(language: Language) -> &'static str {
    match language {
        Language::TraditionalChinese => "結束",
        Language::English => "Quit",
    }
}

pub fn tooltip(running: u32, language: Language) -> String {
    match (running, language) {
        (0, _) => TITLE.into(),
        (count, Language::TraditionalChinese) => format!("{TITLE} · 執行中 {count} 台"),
        (1, Language::English) => format!("{TITLE} · 1 server running"),
        (count, Language::English) => format!("{TITLE} · {count} servers running"),
    }
}

pub fn install(app: &AppHandle) -> tauri::Result<()> {
    let language = Language::TraditionalChinese;
    let show = MenuItem::with_id(app, SHOW_ID, show_label(language), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT_ID, quit_label(language), true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut builder = TrayIconBuilder::with_id("instanceharbor")
        .tooltip(tooltip(0, language))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(menu_event)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    let icon = builder.build(app)?;
    app.manage(Tray { icon, show, quit });
    Ok(())
}

fn menu_event(app: &AppHandle, event: MenuEvent) {
    match event.id().as_ref() {
        SHOW_ID => show_main(app),
        QUIT_ID => {
            record(json!({"type": "tray-quit"}));
            QUITTING.store(true, Ordering::SeqCst);
            app.exit(0);
        }
        _ => {}
    }
}

fn show_main(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        eprintln!("Could not find the main window for the tray icon");
        record(json!({"type": "tray-show", "windowFound": false}));
        return;
    };
    let visible_before = window.is_visible().ok();
    let result = window
        .show()
        .and_then(|_| window.unminimize())
        .and_then(|_| window.set_focus());
    if let Err(error) = &result {
        eprintln!("Could not show the main window from the tray icon: {error}");
    }
    record(json!({
        "type": "tray-show",
        "windowFound": true,
        "visibleBefore": visible_before,
        "error": result.err().map(|error| safe_error(&error.to_string())),
        "visibleAfter": window.is_visible().ok(),
    }));
}

/// What happened when the main window was asked to close. Written to the GUI
/// diagnostic log so a window that will not close can be traced afterwards.
#[derive(Default)]
struct CloseOutcome {
    keep_in_tray: bool,
    preference_error: Option<String>,
    visible_before: Option<bool>,
    hide_error: Option<String>,
    visible_after: Option<bool>,
}

fn close_event(outcome: &CloseOutcome) -> Value {
    json!({
        "type": "window-close",
        "keepInTray": outcome.keep_in_tray,
        "preferenceError": outcome.preference_error.as_deref().map(safe_error),
        "visibleBefore": outcome.visible_before,
        "hideError": outcome.hide_error.as_deref().map(safe_error),
        "visibleAfter": outcome.visible_after,
    })
}

fn record(event: Value) {
    match diagnostics::global() {
        Ok(log) => {
            if let Err(error) = log.record(event) {
                eprintln!("GUI diagnostic write failed: {error}");
            }
        }
        Err(error) => eprintln!("GUI diagnostics unavailable: {error}"),
    }
}

/// Hide the main window on close when the saved preference asks for it.
pub fn window_event(window: &Window, event: &WindowEvent) {
    let WindowEvent::CloseRequested { api, .. } = event else {
        return;
    };
    if window.label() != MAIN_WINDOW {
        return;
    }
    if QUITTING.load(Ordering::SeqCst) {
        record(json!({"type": "window-close", "quitting": true}));
        return;
    }
    let mut outcome = CloseOutcome { visible_before: window.is_visible().ok(), ..CloseOutcome::default() };
    outcome.keep_in_tray = match std::env::var_os("IH_V2_WORKSPACE") {
        Some(root) => instanceharbor_v2::close_to_tray(std::path::Path::new(&root)).unwrap_or_else(|error| {
            // An unreadable preference keeps the safer default: the GUI stays reachable.
            eprintln!("Could not read the close-to-tray preference: {error}");
            outcome.preference_error = Some(error.to_string());
            true
        }),
        None => true,
    };
    if outcome.keep_in_tray {
        api.prevent_close();
        if let Err(error) = window.hide() {
            eprintln!("Could not hide the main window: {error}");
            outcome.hide_error = Some(error.to_string());
        }
        outcome.visible_after = window.is_visible().ok();
    }
    record(close_event(&outcome));
}

impl Tray {
    pub fn update(&self, running: u32, language: Language) -> tauri::Result<()> {
        self.icon.set_tooltip(Some(tooltip(running, language)))?;
        self.show.set_text(show_label(language))?;
        self.quit.set_text(quit_label(language))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_event_records_each_step_without_raw_error_text() {
        let event = close_event(&CloseOutcome {
            keep_in_tray: true,
            preference_error: None,
            visible_before: Some(true),
            hide_error: Some(r"C:\Users\someone\secret failed".into()),
            visible_after: Some(true),
        });
        assert_eq!(event["type"], "window-close");
        assert_eq!(event["keepInTray"], true);
        assert_eq!(event["visibleBefore"], true);
        assert_eq!(event["visibleAfter"], true);
        assert!(event["preferenceError"].is_null());
        assert!(!event["hideError"].is_null());
        assert!(!event.to_string().contains("someone"), "{event}");
    }

    #[test]
    fn tooltip_names_running_servers_in_the_display_language() {
        assert_eq!(tooltip(0, Language::TraditionalChinese), TITLE);
        assert_eq!(tooltip(0, Language::English), TITLE);
        assert_eq!(tooltip(3, Language::TraditionalChinese), format!("{TITLE} · 執行中 3 台"));
        assert_eq!(tooltip(1, Language::English), format!("{TITLE} · 1 server running"));
        assert_eq!(tooltip(2, Language::English), format!("{TITLE} · 2 servers running"));
    }

    #[test]
    fn tray_language_accepts_only_resolved_locales() {
        assert_eq!(Language::parse("zh-TW"), Ok(Language::TraditionalChinese));
        assert_eq!(Language::parse("en"), Ok(Language::English));
        assert!(Language::parse("system").is_err());
        for language in [Language::TraditionalChinese, Language::English] {
            assert!(!show_label(language).contains(TITLE));
            assert!(quit_label(language).chars().count() <= 4);
        }
    }
}
