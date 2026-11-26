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

/// Whether Quit may end the Manager right away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ExitCheck {
    /// Batch or core-update work would be cut off: refuse.
    Busy,
    /// Servers not known to be stopped, and how many of them rely on the Manager for
    /// automatic restart: ask first. `None` means their states could not be read.
    Confirm(Option<(u32, u32)>),
    Clear,
}

/// What the tray asks of the Manager it belongs to.
pub(super) struct Hooks {
    pub running: Box<dyn Fn() -> Result<u32> + Send>,
    pub check_exit: Box<dyn Fn() -> Result<ExitCheck> + Send>,
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

/// A development workspace is named, so several icons stay apart.
pub(super) fn tooltip(workspace: Option<&str>, running: Option<u32>, chinese: bool) -> String {
    let title = crate::desktop_channel::product_title(workspace);
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

/// Asked before Quit while any server is not known to be stopped (starting, running or
/// undecided). Automatic restart is performed by the Manager, so it pauses until the
/// Manager starts again. `None`: the states could not be read, so ask as well.
pub(super) fn exit_confirmation(servers: Option<(u32, u32)>, chinese: bool) -> String {
    let body = match (servers, chinese) {
        (None, true) => "無法確認伺服器狀態，可能還有伺服器在執行。".to_string(),
        (None, false) => "Server states could not be checked, so some may still be running.".to_string(),
        (Some((count, guarded)), true) => {
            let mut text = format!("還有 {count} 台伺服器沒有停止（包含啟動中或狀態未定）。");
            if guarded > 0 {
                text.push_str(&format!(
                    "\n\n其中 {guarded} 台開啟了自動重啟：當機或意外結束時不會自動重啟，要等下次開啟 InstanceHarbor 才會恢復。"
                ));
            }
            text
        }
        (Some((count, guarded)), false) => {
            let mut text = if count == 1 {
                "1 server is not stopped (including starting or undecided).".to_string()
            } else {
                format!("{count} servers are not stopped (including starting or undecided).")
            };
            if guarded > 0 {
                let has = if guarded == 1 { "has" } else { "have" };
                text.push_str(&format!(
                    "\n\n{guarded} of them {has} automatic restart on: a crash or unexpected exit will not restart them until InstanceHarbor opens again."
                ));
            }
            text
        }
    };
    if chinese {
        format!("{body}\n\n結束後伺服器會繼續執行，只會關閉 GUI 與系統匣圖示。\n\n仍要結束嗎？")
    } else {
        format!("{body}\n\nServers keep running after you quit; only the GUI and the tray icon close.\n\nQuit anyway?")
    }
}

#[cfg(not(test))]
fn handle_quit(hooks: &Hooks, chinese: bool) {
    let confirmed = match (hooks.check_exit)() {
        Ok(ExitCheck::Busy) => return message_box(active_work_message(chinese), false),
        Ok(ExitCheck::Confirm(servers)) => confirm(&exit_confirmation(servers, chinese)),
        Ok(ExitCheck::Clear) => true,
        Err(error) => return quit_failed(&error, chinese),
    };
    if !confirmed {
        return;
    }
    // Checked again: work may have started while the question was open.
    match (hooks.request_exit)() {
        Ok(true) => {}
        Ok(false) => message_box(active_work_message(chinese), false),
        Err(error) => quit_failed(&error, chinese),
    }
}

/// The raw error may name user paths: it goes to the diagnostic log as a code only.
#[cfg(not(test))]
fn quit_failed(error: &str, chinese: bool) {
    if let Ok(log) = crate::diagnostics::global() {
        let _ = log.record(quit_failure_event(error));
    }
    message_box(&quit_failed_message(chinese, &crate::diagnostics::log_folder()), true);
}

pub(super) fn quit_failure_event(error: &str) -> Value {
    json!({"type":"manager-quit-failed","error":crate::diagnostics::safe_error(error)})
}

/// `folder` is the log folder as messages show it, without the user profile path.
pub(super) fn quit_failed_message(chinese: bool, folder: &str) -> String {
    if chinese {
        format!("Manager 無法結束，GUI 與系統匣圖示維持開啟。伺服器不受影響，會繼續執行。\n\n請再試一次。如果持續發生，請回報診斷紀錄（{folder}）。")
    } else {
        format!("The Manager could not quit; the GUI and the tray icon stay open. Servers are not affected and keep running.\n\nTry again. If this keeps happening, report the diagnostic log ({folder}).")
    }
}

#[cfg(not(test))]
pub(super) fn start(root: &Path, gui: PathBuf, hooks: Hooks) -> Guard {
    // The Manager serves one workspace; its message boxes name it.
    crate::desktop_channel::bind_workspace(root);
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
    let workspace = crate::desktop_channel::workspace_of(root);
    // No tooltip at registration: Windows 11 keeps the first tooltip as the icon's name and
    // reads "<first> <current>" once the text changes. The loop below sets it right away.
    let mut builder = TrayIconBuilder::new()
        .with_id(format!("instanceharbor-manager-{}", std::process::id()))
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
                QUIT_ID => handle_quit(hooks, presentation::chinese_ui(root)),
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

/// The Manager's message box caption names its workspace in the development channel.
#[cfg(not(test))]
fn caption() -> Vec<u16> {
    let workspace = crate::desktop_channel::workspace_name();
    crate::desktop_channel::product_title(workspace.as_deref()).encode_utf16().chain([0]).collect()
}

#[cfg(not(test))]
fn message_box(text: &str, error: bool) {
    let text: Vec<u16> = text.encode_utf16().chain([0]).collect();
    let caption: Vec<u16> = caption();
    // MB_OK | MB_SETFOREGROUND | MB_TOPMOST, with an error or information icon.
    let flags = 0x0001_0000 | 0x0004_0000 | if error { 0x10 } else { 0x40 };
    unsafe {
        MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), flags);
    }
}

#[cfg(not(test))]
/// A Yes/No warning whose default answer is No.
fn confirm(text: &str) -> bool {
    let text: Vec<u16> = text.encode_utf16().chain([0]).collect();
    let caption: Vec<u16> = caption();
    // MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2 | MB_SETFOREGROUND | MB_TOPMOST
    let flags = 0x0004 | 0x0030 | 0x0100 | 0x0001_0000 | 0x0004_0000;
    // IDYES
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), flags) == 6 }
}

/// Messages one pass handles at most, so the loop always reaches its sleep and its
/// stop and exit checks.
const PUMP_LIMIT: usize = 64;

/// Calls `next` until it reports an empty queue or `limit` handled messages.
fn drain(limit: usize, mut next: impl FnMut() -> bool) -> usize {
    let mut handled = 0;
    while handled < limit && next() {
        handled += 1;
    }
    handled
}

/// Handles the messages waiting for this thread, at most `PUMP_LIMIT` of them.
fn pump_messages() -> usize {
    let mut message: Msg = unsafe { std::mem::zeroed() };
    drain(PUMP_LIMIT, || unsafe {
        if PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, 1) == 0 {
            return false;
        }
        TranslateMessage(&message);
        DispatchMessageW(&message);
        true
    })
}

/// Asks the windows of attached GUI processes to close, as their X button would.
/// Only their application windows: see `closes_on_quit`.
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
    if context.0.contains(&pid)
        && closes_on_quit(IsWindowVisible(window) != 0, GetWindowLongPtrW(window, GWL_EXSTYLE))
    {
        context.1.push(window);
    }
    1
}

const GWL_EXSTYLE: i32 = -20;
const WS_EX_TOOLWINDOW: isize = 0x80;

/// A shown window that is not a tool window. The GUI's event loop keeps a helper window
/// (Tao's "Thread Event Target") that is visible but a tool window; a WM_CLOSE destroys
/// it, after which Tauri can no longer end the event loop and the process only ended
/// through the exit watchdog, 10 s later (#92). Tool windows never show on the taskbar
/// and have no X button, so they are not what Quit means to close.
fn closes_on_quit(visible: bool, extended_style: isize) -> bool {
    visible && extended_style & WS_EX_TOOLWINDOW == 0
}

type RawWindow = *mut std::ffi::c_void;

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
    fn PeekMessageW(message: *mut Msg, window: RawWindow, min: u32, max: u32, remove: u32) -> i32;
    fn TranslateMessage(message: *const Msg) -> i32;
    fn DispatchMessageW(message: *const Msg) -> isize;
    #[cfg(not(test))]
    fn MessageBoxW(window: RawWindow, text: *const u16, caption: *const u16, flags: u32) -> i32;
    #[cfg(not(test))]
    fn AllowSetForegroundWindow(pid: u32) -> i32;
    fn EnumWindows(callback: unsafe extern "system" fn(RawWindow, isize) -> i32, context: isize) -> i32;
    fn GetWindowThreadProcessId(window: RawWindow, pid: *mut u32) -> u32;
    fn IsWindowVisible(window: RawWindow) -> i32;
    fn GetWindowLongPtrW(window: RawWindow, index: i32) -> isize;
    fn PostMessageW(window: RawWindow, message: u32, wparam: usize, lparam: isize) -> i32;
    #[cfg(test)]
    fn PostThreadMessageW(thread: u32, message: u32, wparam: usize, lparam: isize) -> i32;
}

#[cfg(test)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentThreadId() -> u32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quit_closes_shown_application_windows_but_not_tool_windows() {
        // WS_EX_APPWINDOW | WS_EX_WINDOWEDGE: a normal top-level window such as the GUI's.
        assert!(closes_on_quit(true, 0x40000 | 0x100));
        assert!(!closes_on_quit(false, 0));
        // Tao's event target: WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_LAYERED | WS_EX_TOOLWINDOW.
        assert!(!closes_on_quit(true, 0x0800_0000 | 0x20 | 0x80000 | 0x80));
    }

    #[test]
    fn one_pump_pass_handles_a_bounded_number_of_messages() {
        // A message that is always pending again (an unvalidated WM_PAINT, a window that
        // reposts to itself) used to keep the tray thread inside one pass forever (#118).
        let mut pending = PUMP_LIMIT * 10;
        let mut next = || pending.checked_sub(1).map(|left| pending = left).is_some();
        assert_eq!(drain(PUMP_LIMIT, &mut next), PUMP_LIMIT);
        assert_eq!(pending, PUMP_LIMIT * 9);
        assert_eq!(drain(PUMP_LIMIT, || false), 0);
    }

    #[test]
    fn pump_leaves_messages_beyond_the_limit_for_the_next_pass() {
        let thread = unsafe { GetCurrentThreadId() };
        // WM_APP, posted to this thread's own queue; PeekMessage creates the queue.
        let mut message: Msg = unsafe { std::mem::zeroed() };
        unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, 0) };
        for _ in 0..PUMP_LIMIT * 3 {
            assert_ne!(unsafe { PostThreadMessageW(thread, 0x8000, 0, 0) }, 0);
        }
        assert_eq!(pump_messages(), PUMP_LIMIT);
        assert_eq!(pump_messages(), PUMP_LIMIT);
        assert_eq!(pump_messages(), PUMP_LIMIT);
        assert_eq!(pump_messages(), 0);
    }

    #[test]
    fn quit_label_is_short_and_refusal_explains_servers_keep_running() {
        assert_eq!(labels(true).quit, "結束");
        assert_eq!(labels(false).quit, "Quit");
        assert!(active_work_message(true).contains("繼續執行"));
        assert!(active_work_message(false).contains("keep running"));
    }

    #[test]
    fn quit_confirmation_counts_servers_not_stopped_and_explains_auto_restart() {
        let plain = exit_confirmation(Some((2, 0)), true);
        assert!(plain.contains("2 台伺服器沒有停止") && plain.contains("仍要結束") && !plain.contains("自動重啟"));
        let guarded = exit_confirmation(Some((3, 1)), true);
        assert!(guarded.contains("3 台") && guarded.contains("其中 1 台開啟了自動重啟"));
        assert!(exit_confirmation(Some((1, 0)), false).starts_with("1 server is not stopped"));
        let english = exit_confirmation(Some((3, 2)), false);
        assert!(english.contains("3 servers are not stopped") && english.contains("2 of them") && english.contains("Quit anyway?"));
    }

    #[test]
    fn quit_confirmation_when_server_states_cannot_be_read() {
        assert!(exit_confirmation(None, true).contains("無法確認伺服器狀態"));
        assert!(exit_confirmation(None, false).contains("could not be checked"));
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
        let named = crate::desktop_channel::product_title(Some("agent-a"));
        assert_eq!(tooltip(Some("agent-a"), Some(1), false), format!("{named} · 1 server running"));
    }

    #[test]
    fn tray_icon_is_registered_without_a_tooltip() {
        // Windows 11 names the icon "<tooltip at registration> <current tooltip>" once the
        // tooltip changes, which repeated the product title when a server started.
        let source = include_str!("manager_tray.rs");
        let builder = source.split("TrayIconBuilder::new()").nth(1).and_then(|rest| rest.split(".build()").next());
        let builder = builder.expect("run() builds the tray icon");
        assert!(!builder.contains(".with_tooltip("), "the refresh loop sets the tooltip after registration");
        assert!(source.contains("tray.set_tooltip(Some(tooltip("));
    }
}

#[cfg(test)]
mod quit_failure_tests {
    use super::*;

    #[test]
    fn quit_failure_message_is_translated_and_omits_the_raw_error() {
        let folder = "%LOCALAPPDATA%/InstanceHarbor/logs";
        let chinese = quit_failed_message(true, folder);
        assert!(chinese.contains("無法結束") && chinese.contains(folder));
        let english = quit_failed_message(false, folder);
        assert!(english.contains("could not quit") && english.contains(folder));
    }

    #[test]
    fn quit_failure_is_logged_as_a_code_without_paths() {
        let event = quit_failure_event(r"FileAccess[read] path=C:\Users\someone\workspace\preferences.json: denied (os error 5)");
        assert_eq!(event["type"], "manager-quit-failed");
        assert_eq!(event["error"]["code"], "FileAccess");
        assert!(!event.to_string().contains("someone"), "{event}");
    }
}
