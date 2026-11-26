//! One independent console window per workspace. Hosts still own each run's I/O.
use super::*;
use std::collections::HashMap;
use std::os::windows::io::OwnedHandle;
use std::sync::{mpsc, Arc, Mutex};

const TAB_LIMIT: usize = 64;
static OPEN: Mutex<()> = Mutex::new(());
/// Sent by the Manager on tray Quit and `app-exit` (#114). Anything else on the pipe is a tab.
const QUIT_REQUEST: &str = r#"{"quit":true}"#;

fn endpoint(root: &Path) -> Result<String> {
    let scope = content_revision::revision(root.to_string_lossy().to_lowercase().as_bytes())?;
    Ok(format!(r"\\.\pipe\instanceharbor-viewer-{scope}"))
}

/// The session mutex a running viewer holds for its workspace.
fn lease_name(pipe: &str) -> Result<String> {
    Ok(format!(r"Local\{}", pipe.rsplit('\\').next().ok_or("InvalidConsolePipe")?))
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Tab {
    name: String,
    run: String,
    title: String,
    language: presentation::DisplayLanguage,
    #[serde(default)]
    version: u64,
}

#[derive(Default)]
struct Tabs {
    open: HashMap<String, Tab>,
    pending: Vec<Tab>,
    version: u64,
    closing: bool,
}

impl Tabs {
    fn open(&mut self, mut tab: Tab) -> Result<()> {
        if self.closing {
            return Err("ConsoleViewerClosing".into());
        }
        if !instance_name(&tab.name) || !token(&tab.run) || tab.title.trim().is_empty()
            || tab.title.chars().count() > 1024 || tab.title.chars().any(char::is_control) {
            return Err("InvalidConsoleRequest".into());
        }
        if self.open.len() >= TAB_LIMIT && !self.open.contains_key(&tab.run) {
            return Err("ConsoleTabLimit".into());
        }
        self.version += 1;
        tab.version = self.version;
        self.open.insert(tab.run.clone(), tab.clone());
        self.pending.retain(|pending| pending.run != tab.run);
        self.pending.push(tab);
        Ok(())
    }

    // A later open request wins over a close sent by the renderer before it saw
    // that request. `(removed, viewer exits)`; once the last tab is gone, no tab opens.
    fn close(&mut self, run: &str, version: u64) -> (bool, bool) {
        if self.open.get(run).is_none_or(|tab| tab.version != version) {
            return (false, false);
        }
        self.open.remove(run);
        self.closing = self.open.is_empty();
        (true, self.closing)
    }

    /// Tray Quit: the viewer ends with its tabs. Their servers keep running in their Hosts.
    fn quit(&mut self) {
        self.closing = true;
        self.open.clear();
        self.pending.clear();
    }
}

/// One request on the viewer pipe. The pipe only accepts `ihctl.exe` of this installation.
fn respond(tabs: &Mutex<Tabs>, exit: &mpsc::Sender<()>, request: &str) -> String {
    let result = (|| {
        let mut tabs = tabs.lock().map_err(fail)?;
        if request == QUIT_REQUEST {
            tabs.quit();
            return exit.send(()).map_err(fail);
        }
        tabs.open(serde_json::from_str(request).map_err(|_| "InvalidConsoleRequest")?)
    })();
    match result {
        Ok(()) => json!({"ok":true}).to_string(),
        Err(error) => json!({"error":error}).to_string(),
    }
}

pub struct ConsoleViewer {
    root: PathBuf,
    tabs: Arc<Mutex<Tabs>>,
    quit: Mutex<Option<mpsc::Receiver<()>>>,
    _lease: OwnedHandle,
}

impl ConsoleViewer {
    pub fn start(workspace: &str) -> Result<Option<Self>> {
        let root = safe_directory(Path::new(workspace))?;
        let pipe = endpoint(&root)?;
        let Some(lease) = gui_singleton::acquire_named(&lease_name(&pipe)?).map_err(fail)? else {
            return Ok(None);
        };
        let expected = [std::env::current_exe()
            .map_err(fail)?
            .with_file_name("ihctl.exe")
            .canonicalize()
            .map_err(fail)?];
        let tabs = Arc::new(Mutex::new(Tabs::default()));
        let receiver = Arc::clone(&tabs);
        let (ready, started) = mpsc::sync_channel(1);
        let (quit, quit_requested) = mpsc::channel();
        thread::spawn(move || {
            let result = host_pipe::serve_local(
                &pipe,
                &expected,
                || ready.send(()).map_err(io::Error::other),
                |request| respond(&receiver, &quit, request),
            );
            if let Err(error) = result {
                eprintln!("Native console viewer pipe: {error}");
            }
        });
        started
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "ConsoleViewerUnavailable")?;
        Ok(Some(Self {
            root,
            tabs,
            quit: Mutex::new(Some(quit_requested)),
            _lease: lease,
        }))
    }

    /// Receives once when the Manager quits (tray Quit or `app-exit`); the viewer then exits.
    pub fn quit_requests(&self) -> Result<mpsc::Receiver<()>> {
        self.quit.lock().map_err(fail)?.take().ok_or_else(|| "ConsoleViewerQuitTaken".into())
    }

    pub fn pending(&self) -> Result<Vec<Tab>> {
        Ok(std::mem::take(&mut self.tabs.lock().map_err(fail)?.pending))
    }

    pub fn title(&self, run: &str) -> Result<String> {
        self.tabs.lock().map_err(fail)?.open.get(run)
            .map(|tab| tab.title.clone()).ok_or_else(|| "ConsoleTabClosed".into())
    }

    pub fn request(&self, run: &str, request: native_console::Request) -> Result<Value> {
        if matches!(request, native_console::Request::Probe) {
            return Err("InvalidConsoleRequest".into());
        }
        let tab = self
            .tabs
            .lock()
            .map_err(fail)?
            .open
            .get(run)
            .cloned()
            .ok_or("ConsoleTabClosed")?;
        native_console::call(&self.root, &tab.name, run, request)
    }

    /// `(removed, viewer exits)`. The final close and acceptance of new opens share this lock.
    pub fn close(&self, run: &str, version: u64) -> Result<(bool, bool)> {
        Ok(self.tabs.lock().map_err(fail)?.close(run, version))
    }
}

/// Called by the Manager on tray Quit and `app-exit`: asks this workspace's console viewer
/// to exit. `Ok(false)` when no viewer runs. Only the viewer process ends; each server's
/// Host owns its console and keeps running. The console-window fallback tier belongs to the
/// Host and is never touched: closing it would end the server.
pub(super) fn quit(root: &Path) -> Result<bool> {
    let pipe = endpoint(root)?;
    let lease = lease_name(&pipe)?;
    if !gui_singleton::exists_named(&lease).map_err(fail)? {
        return Ok(false);
    }
    let gui = std::env::current_exe()
        .map_err(fail)?
        .with_file_name(crate::desktop_channel::GUI_EXECUTABLE)
        .canonicalize()
        .map_err(fail)?;
    // The pipe has one instance; it is briefly busy while it serves another request.
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match host_pipe::local_request_executable(&pipe, &gui, QUIT_REQUEST) {
            Ok((_, reply)) => {
                let reply: Value = serde_json::from_str(&reply).map_err(fail)?;
                return if reply["ok"] == true {
                    Ok(true)
                } else {
                    Err(reply["error"].as_str().unwrap_or("ConsoleResultUnconfirmed").into())
                };
            }
            Err(error) if error == "ManagerPipeUnavailable" && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(20));
            }
            // The viewer ended between the lease check and the request.
            Err(error) if error == "ManagerPipeUnavailable" => {
                return if gui_singleton::exists_named(&lease).map_err(fail)? {
                    Err("ConsoleViewerUnavailable".into())
                } else {
                    Ok(false)
                };
            }
            Err(error) => return Err(error),
        }
    }
}

pub(super) fn open(root: &Path, name: &str, run: &str) -> Result<Value> {
    native_console::call(root, name, run, native_console::Request::Probe)?;
    let (title, language) = match presentation::console_appearance(root, name, run) {
        Ok(appearance) => appearance,
        Err(error) if error == "ConsoleRunChanged" => return Err(error),
        Err(error) => {
            eprintln!("Console title unavailable: {error}");
            (name.into(), presentation::DisplayLanguage::default())
        }
    };
    let gui = std::env::current_exe()
        .map_err(fail)?
        .with_file_name(crate::desktop_channel::GUI_EXECUTABLE)
        .canonicalize()
        .map_err(fail)?;
    let pipe = endpoint(root)?;
    let frame = serde_json::to_string(&Tab {
        name: name.into(),
        run: run.into(),
        title,
        language,
        version: 0,
    })
    .map_err(fail)?;
    let _opening = OPEN.lock().map_err(fail)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut next_launch = Instant::now();
    loop {
        match host_pipe::local_request_executable(&pipe, &gui, &frame) {
            Ok((pid, reply)) => {
                let reply: Value = serde_json::from_str(&reply).map_err(fail)?;
                if reply["ok"] == true {
                    return Ok(json!({"run":run, "viewerPid":pid}));
                }
                let error = reply["error"]
                    .as_str()
                    .unwrap_or("ConsoleResultUnconfirmed");
                if error != "ConsoleViewerClosing" {
                    return Err(error.into());
                }
            }
            Err(error) if error == "ManagerPipeUnavailable" => {
                if Instant::now() >= next_launch {
                    launch_detached(&gui, root, "native-console")?;
                    next_launch = Instant::now() + Duration::from_secs(2);
                }
            }
            Err(error) => return Err(error),
        }
        if Instant::now() >= deadline {
            return Err("ConsoleViewerUnavailable".into());
        }
        thread::sleep(Duration::from_millis(35));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(run: &str) -> Tab {
        Tab {
            name: "alpha".into(),
            run: run.into(),
            title: "alpha".into(),
            language: presentation::DisplayLanguage::default(),
            version: 0,
        }
    }

    fn opened(tabs: &mut Tabs, run: &str) -> u64 {
        tabs.open(tab(run)).unwrap();
        tabs.open[run].version
    }

    #[test]
    fn closing_the_last_tab_ends_the_viewer_and_refuses_later_opens() {
        let mut tabs = Tabs::default();
        let first = opened(&mut tabs, "run-1");
        let second = opened(&mut tabs, "run-2");
        assert_eq!(tabs.close("run-1", first), (true, false), "another tab is still open");
        assert_eq!(tabs.close("run-2", second), (true, true), "the last tab ends the viewer");
        // console_viewer::open launches a new viewer when it sees this error.
        assert_eq!(tabs.open(tab("run-3")).unwrap_err(), "ConsoleViewerClosing");
    }

    #[test]
    fn a_close_sent_before_the_tab_was_opened_again_keeps_it() {
        let mut tabs = Tabs::default();
        let old = opened(&mut tabs, "run-1");
        let reopened = opened(&mut tabs, "run-1");
        assert_eq!(tabs.close("run-1", old), (false, false));
        assert!(!tabs.closing);
        assert_eq!(tabs.close("run-unknown", 1), (false, false));
        assert_eq!(tabs.close("run-1", reopened), (true, true));
    }

    #[test]
    fn quit_request_ends_the_viewer_even_with_tabs_open() {
        let tabs = Mutex::new(Tabs::default());
        let (exit, requested) = mpsc::channel();
        let open = serde_json::to_string(&tab("run-1")).unwrap();
        assert_eq!(respond(&tabs, &exit, &open), json!({"ok":true}).to_string());
        assert!(requested.try_recv().is_err(), "opening a tab does not end the viewer");
        assert_eq!(respond(&tabs, &exit, QUIT_REQUEST), json!({"ok":true}).to_string());
        assert!(requested.try_recv().is_ok(), "the viewer was asked to exit");
        // A console-open racing the quit gets the error that makes it start a new viewer.
        assert_eq!(respond(&tabs, &exit, &open), json!({"error":"ConsoleViewerClosing"}).to_string());
        assert!(tabs.lock().unwrap().open.is_empty());
    }

    #[test]
    fn malformed_requests_are_rejected_without_ending_the_viewer() {
        let tabs = Mutex::new(Tabs::default());
        let (exit, requested) = mpsc::channel();
        for request in [r#"{"quit":false}"#, r#""quit""#, "{", r#"{"quit":true,"x":1}"#] {
            assert!(respond(&tabs, &exit, request).contains("error"), "{request}");
        }
        assert!(requested.try_recv().is_err());
        assert!(!tabs.lock().unwrap().closing);
    }
}
