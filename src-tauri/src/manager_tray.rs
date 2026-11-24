//! Notification-area icon of the resident Manager.
//!
//! The icon exists exactly while the Manager runs: a Manager started by the GUI,
//! the CLI or an Agent shows it, and choosing Quit ends the Manager and the GUI
//! windows attached to it. Servers and their Hosts are separate processes and keep
//! running. The GUI no longer has an icon of its own.
#![cfg_attr(test, allow(dead_code))]
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
// muda imports comctl32 functions that only version 6 exports by name. The binaries
// carry that manifest; the library's unit-test executable does not, so the icon code
// is left out of test builds, which would otherwise fail to load.
#[cfg(not(test))]
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
#[cfg(not(test))]
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

#[cfg(not(test))]
const OPEN_ID: &str = "manager-open";
#[cfg(not(test))]
const QUIT_ID: &str = "manager-quit";
#[cfg(not(test))]
/// The application icon tauri-build embeds in every binary of the package.
const ICON_RESOURCE: u16 = 32512;
#[cfg(not(test))]
const REFRESH: Duration = Duration::from_secs(5);

/// What the tray asks of the Manager it belongs to.
pub(super) struct Hooks {
    pub running: Box<dyn Fn() -> Result<u32> + Send>,
    /// `Ok(true)` once the Manager will exit; `Ok(false)` while work blocks it.
    pub request_exit: Box<dyn Fn() -> Result<bool> + Send>,
    pub exiting: Box<dyn Fn() -> bool + Send>,
}

/// Removes the icon before the Manager process ends. Without this, Windows keeps a
/// stale icon until the pointer passes over it.
pub(super) struct Guard {
    stop: Arc<AtomicBool>,
    done: Option<mpsc::Receiver<()>>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(done) = self.done.take() {
            let _ = done.recv_timeout(Duration::from_secs(2));
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Labels {
    pub open: &'static str,
    pub quit: &'static str,
}

pub(super) fn labels(chinese: bool) -> Labels {
    if chinese {
        Labels { open: "開啟 InstanceHarbor", quit: "結束" }
    } else {
        Labels { open: "Open InstanceHarbor", quit: "Quit" }
    }
}

/// The disabled first menu line: what the Manager is doing right now.
pub(super) fn status_line(running: Option<u32>, chinese: bool) -> String {
    match (running, chinese) {
        (None, true) => "Manager 執行中".into(),
        (None, false) => "Manager running".into(),
        (Some(count), true) => format!("Manager 執行中 · 伺服器 {count} 台"),
        (Some(1), false) => "Manager running · 1 server".into(),
        (Some(count), false) => format!("Manager running · {count} servers"),
    }
}

/// A workspace other than the default one is named, so several icons stay apart.
pub(super) fn tooltip(workspace: Option<&str>, running: Option<u32>, chinese: bool) -> String {
    let title = match workspace {
        Some(name) => format!("{} ({name})", crate::desktop_channel::TITLE),
        None => crate::desktop_channel::TITLE.into(),
    };
    match (running, chinese) {
        (None | Some(0), _) => title,
        (Some(count), true) => format!("{title} · 執行中 {count} 台"),
        (Some(1), false) => format!("{title} · 1 server running"),
        (Some(count), false) => format!("{title} · {count} servers running"),
    }
}

pub(super) fn active_work_message(chinese: bool) -> &'static str {
    if chinese {
        "還有批次工作或核心更新正在進行，Manager 現在不能結束。\n\n請等它們完成後再結束。伺服器不受影響，會繼續執行。"
    } else {
        "A batch or a core update is still in progress, so the Manager cannot quit now.\n\nQuit again once it finishes. Servers are not affected and keep running."
    }
}

#[cfg(not(test))]
fn workspace_label(root: &Path) -> Option<String> {
    // The development channel uses many workspaces; name each one.
    crate::desktop_channel::DEVELOPMENT
        .then(|| root.file_name().map(|name| name.to_string_lossy().into_owned()))
        .flatten()
}

#[cfg(not(test))]
pub(super) fn start(root: &Path, gui: PathBuf, hooks: Hooks) -> Guard {
    let stop = Arc::new(AtomicBool::new(false));
    let (done_send, done) = mpsc::channel();
    let thread_stop = Arc::clone(&stop);
    let root = root.to_path_buf();
    let spawned = thread::Builder::new().name("manager-tray".into()).spawn(move || {
        if let Err(error) = run(&root, &gui, &hooks, &thread_stop) {
            eprintln!("Manager tray icon unavailable: {error}");
        }
        let _ = done_send.send(());
    });
    match spawned {
        Ok(_) => Guard { stop, done: Some(done) },
        Err(error) => {
            eprintln!("Manager tray thread failed to start: {error}");
            Guard { stop, done: None }
        }
    }
}

#[cfg(not(test))]
fn run(root: &Path, gui: &Path, hooks: &Hooks, stop: &AtomicBool) -> Result<()> {
    let chinese = presentation::chinese_ui(root);
    let text = labels(chinese);
    let status = MenuItem::with_id("manager-status", status_line(None, chinese), false, None);
    let open = MenuItem::with_id(OPEN_ID, text.open, true, None);
    let quit = MenuItem::with_id(QUIT_ID, text.quit, true, None);
    let menu = Menu::new();
    menu.append_items(&[&status, &PredefinedMenuItem::separator(), &open, &quit])
        .map_err(|error| error.to_string())?;
    let workspace = workspace_label(root);
    let mut builder = TrayIconBuilder::new()
        .with_id(format!("instanceharbor-manager-{}", std::process::id()))
        .with_tooltip(tooltip(workspace.as_deref(), None, chinese))
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false);
    match Icon::from_resource(ICON_RESOURCE, None) {
        Ok(icon) => builder = builder.with_icon(icon),
        Err(error) => eprintln!("Manager tray icon resource unavailable: {error}"),
    }
    let tray: TrayIcon = builder.build().map_err(|error| error.to_string())?;
    let menus = MenuEvent::receiver();
    let clicks = TrayIconEvent::receiver();
    let mut next_refresh = Instant::now();
    let mut shown: Option<(Option<u32>, bool)> = None;
    while !stop.load(Ordering::SeqCst) && !(hooks.exiting)() {
        pump_messages();
        while let Ok(event) = menus.try_recv() {
            match event.id().as_ref() {
                OPEN_ID => open_gui(root, gui, chinese),
                QUIT_ID => match (hooks.request_exit)() {
                    Ok(true) => {}
                    Ok(false) => message_box(active_work_message(chinese), false),
                    Err(error) => message_box(&error, true),
                },
                _ => {}
            }
        }
        while let Ok(event) = clicks.try_recv() {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                open_gui(root, gui, chinese);
            }
        }
        if Instant::now() >= next_refresh {
            next_refresh = Instant::now() + REFRESH;
            // The display language can change while the Manager runs.
            let chinese = presentation::chinese_ui(root);
            let running = (hooks.running)().map_err(|error| eprintln!("Manager tray count: {error}")).ok();
            if shown != Some((running, chinese)) {
                let text = labels(chinese);
                let _ = tray.set_tooltip(Some(tooltip(workspace.as_deref(), running, chinese)));
                status.set_text(status_line(running, chinese));
                open.set_text(text.open);
                quit.set_text(text.quit);
                shown = Some((running, chinese));
            }
        }
        thread::sleep(Duration::from_millis(30));
    }
    drop(tray);
    pump_messages();
    Ok(())
}

#[cfg(not(test))]
/// Opens the GUI bound to this workspace. A GUI that already runs brings its window forward.
fn open_gui(root: &Path, gui: &Path, chinese: bool) {
    unsafe {
        AllowSetForegroundWindow(u32::MAX);
    }
    if let Err(error) = std::process::Command::new(gui).env("IH_WORKSPACE", root).spawn() {
        let text = if chinese {
            format!("無法開啟 InstanceHarbor：{error}")
        } else {
            format!("Could not open InstanceHarbor: {error}")
        };
        message_box(&text, true);
    }
}

#[cfg(not(test))]
fn message_box(text: &str, error: bool) {
    let text: Vec<u16> = text.encode_utf16().chain([0]).collect();
    let caption: Vec<u16> = crate::desktop_channel::TITLE.encode_utf16().chain([0]).collect();
    // MB_OK | MB_SETFOREGROUND | MB_TOPMOST, with an error or information icon.
    let flags = 0x0001_0000 | 0x0004_0000 | if error { 0x10 } else { 0x40 };
    unsafe {
        MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), flags);
    }
}

#[cfg(not(test))]
fn pump_messages() {
    let mut message: Msg = unsafe { std::mem::zeroed() };
    unsafe {
        while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, 1) != 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

/// Asks the windows of attached GUI processes to close, as their X button would.
pub(super) fn close_gui_windows(pids: &[u32]) {
    if pids.is_empty() {
        return;
    }
    let mut context: (&[u32], Vec<RawWindow>) = (pids, Vec::new());
    unsafe {
        EnumWindows(collect_windows, (&mut context as *mut (&[u32], Vec<RawWindow>)) as isize);
        for window in context.1 {
            PostMessageW(window, 0x0010, 0, 0);
        }
    }
}

unsafe extern "system" fn collect_windows(window: RawWindow, context: isize) -> i32 {
    let context = &mut *(context as *mut (&[u32], Vec<RawWindow>));
    let mut pid = 0;
    GetWindowThreadProcessId(window, &mut pid);
    if context.0.contains(&pid) && IsWindowVisible(window) != 0 {
        context.1.push(window);
    }
    1
}

type RawWindow = *mut std::ffi::c_void;

#[cfg(not(test))]
#[repr(C)]
struct Msg {
    window: RawWindow,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    point: [i32; 2],
    private: u32,
}

#[link(name = "user32")]
unsafe extern "system" {
    #[cfg(not(test))]
    fn PeekMessageW(message: *mut Msg, window: RawWindow, min: u32, max: u32, remove: u32) -> i32;
    #[cfg(not(test))]
    fn TranslateMessage(message: *const Msg) -> i32;
    #[cfg(not(test))]
    fn DispatchMessageW(message: *const Msg) -> isize;
    #[cfg(not(test))]
    fn MessageBoxW(window: RawWindow, text: *const u16, caption: *const u16, flags: u32) -> i32;
    #[cfg(not(test))]
    fn AllowSetForegroundWindow(pid: u32) -> i32;
    fn EnumWindows(callback: unsafe extern "system" fn(RawWindow, isize) -> i32, context: isize) -> i32;
    fn GetWindowThreadProcessId(window: RawWindow, pid: *mut u32) -> u32;
    fn IsWindowVisible(window: RawWindow) -> i32;
    fn PostMessageW(window: RawWindow, message: u32, wparam: usize, lparam: isize) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quit_label_is_short_and_refusal_explains_servers_keep_running() {
        assert_eq!(labels(true).quit, "結束");
        assert_eq!(labels(false).quit, "Quit");
        assert!(active_work_message(true).contains("繼續執行"));
        assert!(active_work_message(false).contains("keep running"));
    }

    #[test]
    fn status_and_tooltip_count_running_servers() {
        assert_eq!(status_line(Some(2), true), "Manager 執行中 · 伺服器 2 台");
        assert_eq!(status_line(Some(1), false), "Manager running · 1 server");
        assert_eq!(status_line(None, false), "Manager running");
        let title = crate::desktop_channel::TITLE;
        assert_eq!(tooltip(None, Some(0), true), title);
        assert_eq!(tooltip(None, None, false), title);
        assert_eq!(tooltip(None, Some(3), true), format!("{title} · 執行中 3 台"));
        assert_eq!(tooltip(Some("agent-a"), Some(1), false), format!("{title} (agent-a) · 1 server running"));
    }
}
