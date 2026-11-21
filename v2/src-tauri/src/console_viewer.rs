//! One independent console window per workspace. Hosts still own each run's I/O.
use super::*;
use std::collections::HashMap;
use std::os::windows::io::OwnedHandle;
use std::sync::{mpsc, Arc, Mutex};

const TAB_LIMIT: usize = 64;
static OPEN: Mutex<()> = Mutex::new(());

fn endpoint(root: &Path) -> Result<String> {
    let scope = content_revision::revision(root.to_string_lossy().to_lowercase().as_bytes())?;
    Ok(format!(r"\\.\pipe\instanceharbor-v2-viewer-{scope}"))
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
        if !token(&tab.name) || !token(&tab.run) || tab.title.trim().is_empty()
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
}

pub struct ConsoleViewer {
    root: PathBuf,
    tabs: Arc<Mutex<Tabs>>,
    _lease: OwnedHandle,
}

impl ConsoleViewer {
    pub fn start(workspace: &str) -> Result<Option<Self>> {
        let root = safe_directory(Path::new(workspace))?;
        let pipe = endpoint(&root)?;
        let Some(lease) = gui_singleton::acquire_named(&format!(
            r"Local\{}",
            pipe.rsplit('\\').next().ok_or("InvalidConsolePipe")?
        ))
        .map_err(fail)?
        else {
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
        thread::spawn(move || {
            let result = host_pipe::serve_local(
                &pipe,
                &expected,
                || ready.send(()).map_err(io::Error::other),
                |request| {
                    let result = (|| {
                        let tab = serde_json::from_str(request).map_err(fail)?;
                        receiver.lock().map_err(fail)?.open(tab)
                    })();
                    match result {
                        Ok(()) => json!({"ok":true}).to_string(),
                        Err(error) => json!({"error":error}).to_string(),
                    }
                },
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
            _lease: lease,
        }))
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

    // A later open request wins over a close sent by the renderer before it saw
    // that request. The final close and acceptance of new opens share this lock.
    pub fn close(&self, run: &str, version: u64) -> Result<(bool, bool)> {
        let mut tabs = self.tabs.lock().map_err(fail)?;
        if tabs.open.get(run).is_none_or(|tab| tab.version != version) {
            return Ok((false, false));
        }
        tabs.open.remove(run);
        tabs.closing = tabs.open.is_empty();
        Ok((true, tabs.closing))
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
