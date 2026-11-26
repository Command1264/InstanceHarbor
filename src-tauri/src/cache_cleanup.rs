//! "Clear cache" in the app settings.
//!
//! Only these exact locations are listed and deleted:
//! - core download leftovers directly in the workspace's `staging` folder, named
//!   `download-<SHA-256>`, `pending-download-<PID>-<clock>` or `core-download-<PID>-<clock>`;
//! - the WebView2 folder that versions before 1.0 used
//!   (`%LOCALAPPDATA%\local.instanceharbor.v2`, development `local.instanceharbor.v2.dev`);
//! - the current channel's WebView2 folder, which the running GUI holds open, so
//!   clearing it only schedules the deletion for the next GUI start;
//! - shared cores (#62), `shared-cores/<SHA-256>.jar`, that no instance names any
//!   more: the file has only its own name and no process has it open.
//!
//! Each entry is checked again when it is deleted. Entries that a running job
//! uses are skipped, and one failure does not stop the others.
use super::*;

const MARKER_FILE: &str = "webview-cache-clear.json";
const MAX_REQUEST_BYTES: usize = 4096;
const MAX_MARKER_BYTES: u64 = 8192;
/// Stops a size walk early; the size is then reported as incomplete.
const MAX_WALK_ENTRIES: usize = 200_000;
const REPARSE_POINT: u32 = 0x400;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
enum Category {
    Download,
    LegacyWebView,
    WebView,
    SharedCore,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
enum State {
    Ready,
    /// A running job uses it.
    InUse,
    /// Already scheduled for the next GUI start.
    Scheduled,
    /// Cannot be cleared from here, for example a custom WebView2 folder.
    Unavailable,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Item {
    category: Category,
    #[serde(skip)]
    location: PathBuf,
    path: String,
    bytes: u64,
    complete: bool,
    state: State,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Marker {
    folder: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    revision: String,
}

/// Where the WebView2 folders are. Tests pass a fake LocalAppData.
struct Places {
    legacy: PathBuf,
    /// The channel's standard WebView2 folder, or why this process cannot clear it.
    webview: std::result::Result<PathBuf, &'static str>,
}

impl Places {
    fn current(root: &Path) -> Result<Self> {
        let local = crate::desktop_channel::local_app_data()?;
        let configured = std::env::var_os("WEBVIEW2_USER_DATA_FOLDER").filter(|value| !value.is_empty()).map(PathBuf::from);
        Ok(Self::new(&local, crate::desktop_channel::workspace_of(root).as_deref(), configured.as_deref()))
    }

    fn new(local: &Path, workspace: Option<&str>, configured: Option<&Path>) -> Self {
        let legacy = local.join(if crate::desktop_channel::DEVELOPMENT {
            "local.instanceharbor.v2.dev"
        } else {
            "local.instanceharbor.v2"
        });
        let standard = crate::desktop_channel::webview_profile(local, workspace);
        // A launcher may point the stable GUI at its own folder; that folder is not ours to delete.
        let webview = match configured {
            Some(folder) if !same_location(folder, &standard) => Err("WebViewCacheCustomFolder"),
            _ => Ok(standard),
        };
        Self { legacy, webview }
    }
}

fn same_location(left: &Path, right: &Path) -> bool {
    path_format::display(left).trim_end_matches('\\').eq_ignore_ascii_case(path_format::display(right).trim_end_matches('\\'))
}

fn digits(value: &str) -> bool {
    !value.is_empty() && value.len() <= 20 && value.bytes().all(|byte| byte.is_ascii_digit())
}

/// The kind of download leftover a `staging` entry name denotes, with the PID
/// that wrote a scratch folder. Any other name is not a cache entry.
enum Leftover {
    /// `download-<SHA-256>`: a verified download kept for the next creation.
    Verified(String),
    /// `pending-download-<PID>-<clock>` or `core-download-<PID>-<clock>`.
    Scratch(u32),
}

fn leftover(name: &str) -> Option<Leftover> {
    if let Some(hash) = name.strip_prefix("download-") {
        return (hash.len() == 64 && hash.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')))
            .then(|| Leftover::Verified(hash.to_owned()));
    }
    let rest = name.strip_prefix("pending-download-").or_else(|| name.strip_prefix("core-download-"))?;
    let (pid, clock) = rest.split_once('-')?;
    if !digits(pid) || !digits(clock) {
        return None;
    }
    pid.parse().ok().map(Leftover::Scratch)
}

/// A plain directory: not a link or junction.
fn plain_directory(path: &Path) -> io::Result<bool> {
    use std::os::windows::fs::MetadataExt;
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.is_dir() && metadata.file_attributes() & REPARSE_POINT == 0),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Total file length below `path`, without following links.
fn tree_size(path: &Path) -> (u64, bool) {
    use std::os::windows::fs::MetadataExt;
    let mut bytes = 0u64;
    let mut seen = 0usize;
    let mut pending = vec![path.to_path_buf()];
    let mut complete = true;
    while let Some(folder) = pending.pop() {
        let Ok(entries) = fs::read_dir(&folder) else {
            complete = false;
            continue;
        };
        for entry in entries {
            seen += 1;
            if seen > MAX_WALK_ENTRIES {
                return (bytes, false);
            }
            let Ok(entry) = entry else {
                complete = false;
                continue;
            };
            let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
                complete = false;
                continue;
            };
            if metadata.file_attributes() & REPARSE_POINT != 0 {
                continue;
            }
            if metadata.is_dir() {
                pending.push(entry.path());
            } else {
                bytes = bytes.saturating_add(metadata.len());
            }
        }
    }
    (bytes, complete)
}

/// Whether another holder has this workspace command lock. Never creates the lock file.
fn lock_held(root: &Path, name: &str) -> bool {
    let path = root.join(name);
    match OpenOptions::new().write(true).open(&path) {
        Ok(file) => file.try_lock().is_err(),
        Err(error) => error.kind() != io::ErrorKind::NotFound,
    }
}

/// A scratch folder belongs to a running process while its writer PID lives.
/// An unreadable process counts as running.
fn writer_running(pid: u32) -> bool {
    !matches!(identity(pid), Ok(None))
}

fn download_items(root: &Path) -> Result<Vec<Item>> {
    let staging = child_directory(root, "staging")?;
    let mut items = Vec::new();
    for entry in fs::read_dir(&staging).map_err(fail)? {
        let entry = entry.map_err(fail)?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else { continue };
        let Some(kind) = leftover(&name) else { continue };
        let location = staging.join(&name);
        if !plain_directory(&location).map_err(fail)? {
            continue;
        }
        let in_use = match &kind {
            Leftover::Verified(hash) => lock_held(root, &format!("download-{hash}.lock")),
            Leftover::Scratch(pid) => writer_running(*pid),
        };
        let (bytes, complete) = tree_size(&location);
        items.push(Item {
            category: Category::Download,
            path: path_format::display(&location),
            location,
            bytes,
            complete,
            state: if in_use { State::InUse } else { State::Ready },
            reason: None,
        });
    }
    items.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(items)
}

fn read_marker(root: &Path) -> Result<Option<Marker>> {
    let path = root.join(MARKER_FILE);
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(file_error("read", &path, error)),
    };
    no_redirect(&path)?;
    let mut bytes = Vec::new();
    file.take(MAX_MARKER_BYTES + 1).read_to_end(&mut bytes).map_err(|error| file_error("read", &path, error))?;
    if bytes.len() as u64 > MAX_MARKER_BYTES {
        return Err("InvalidWebViewCacheMarker".into());
    }
    serde_json::from_slice(&bytes).map(Some).map_err(|_| "InvalidWebViewCacheMarker".into())
}

/// Shared cores with only their own name (#62). One opened by a process is in use.
fn shared_core_items(root: &Path) -> Result<Vec<Item>> {
    Ok(shared_core::unused(root)?.into_iter().map(|core| Item {
        category: Category::SharedCore,
        path: path_format::display(&core.path),
        location: core.path,
        bytes: core.bytes,
        complete: true,
        state: if core.open { State::InUse } else { State::Ready },
        reason: None,
    }).collect())
}

fn preview_items(root: &Path, places: &Places) -> Result<Vec<Item>> {
    let mut items = download_items(root)?;
    items.extend(shared_core_items(root)?);
    if plain_directory(&places.legacy).map_err(fail)? {
        let (bytes, complete) = tree_size(&places.legacy);
        items.push(Item {
            category: Category::LegacyWebView,
            path: path_format::display(&places.legacy),
            location: places.legacy.clone(),
            bytes,
            complete,
            state: State::Ready,
            reason: None,
        });
    }
    match &places.webview {
        Ok(folder) => {
            let (bytes, complete) = if plain_directory(folder).map_err(fail)? { tree_size(folder) } else { (0, true) };
            let marker = read_marker(root)?.filter(|marker| same_location(Path::new(&marker.folder), folder));
            items.push(Item {
                category: Category::WebView,
                path: path_format::display(folder),
                location: folder.clone(),
                bytes,
                complete,
                state: if marker.is_some() { State::Scheduled } else { State::Ready },
                reason: marker.and_then(|marker| marker.error),
            });
        }
        Err(reason) => items.push(Item {
            category: Category::WebView,
            path: String::new(),
            location: PathBuf::new(),
            bytes: 0,
            complete: true,
            state: State::Unavailable,
            reason: Some((*reason).into()),
        }),
    }
    Ok(items)
}

/// Sizes change while the GUI runs, so the revision covers only what would be deleted.
fn revision(items: &[Item]) -> Result<String> {
    let identity: Vec<_> = items.iter().map(|item| json!([item.category, item.path, item.state])).collect();
    content_revision::revision(&serde_json::to_vec(&identity).map_err(fail)?)
}

fn preview_value(items: &[Item]) -> Result<Value> {
    Ok(json!({"items":items,"revision":revision(items)?}))
}

fn reason(error: &io::Error) -> String {
    match error.raw_os_error() {
        Some(5) => "AccessDenied".into(),
        Some(32) | Some(33) => "FileInUse".into(),
        Some(code) => format!("os error {code}"),
        None => error.kind().to_string(),
    }
}

/// Deletes one listed folder after checking it is still a plain directory.
fn remove_folder(path: &Path) -> std::result::Result<(), String> {
    match plain_directory(path) {
        Ok(true) => fs::remove_dir_all(path).map_err(|error| reason(&error)),
        Ok(false) if !path.exists() => Ok(()),
        Ok(false) => Err("CacheEntryChanged".into()),
        Err(error) => Err(reason(&error)),
    }
}

fn clear_item(root: &Path, item: &Item) -> std::result::Result<&'static str, String> {
    match item.category {
        Category::Download => {
            let name = item.location.file_name().and_then(|name| name.to_str()).unwrap_or("");
            match leftover(name) {
                Some(Leftover::Verified(hash)) => {
                    // Holding the lock keeps a creation from reusing the folder while it is deleted.
                    let _lock = command_lock(root, &format!("download-{hash}.lock"), false).map_err(|_| "InUse".to_owned())?;
                    remove_folder(&item.location)?;
                }
                Some(Leftover::Scratch(pid)) if !writer_running(pid) => remove_folder(&item.location)?,
                Some(Leftover::Scratch(_)) => return Err("InUse".into()),
                None => return Err("CacheEntryChanged".into()),
            }
            Ok("deleted")
        }
        Category::LegacyWebView => remove_folder(&item.location).map(|()| "deleted"),
        Category::SharedCore => {
            // Checked again under its lock: an instance may have named it since the preview.
            let hash = item.location.file_stem().and_then(|name| name.to_str()).unwrap_or("");
            shared_core::remove_unused(root, hash).map(|()| "deleted")
        }
        Category::WebView => {
            let marker = Marker { folder: item.path.clone(), error: None };
            let path = root.join(MARKER_FILE);
            let present = fs::symlink_metadata(&path).is_ok();
            core_maintenance::write_durable(&path, &marker, present)?;
            Ok("scheduled")
        }
    }
}

fn clear(root: &Path, places: &Places, raw: &str) -> Result<Value> {
    if raw.len() > MAX_REQUEST_BYTES {
        return Err("InvalidCacheRequest".into());
    }
    let request: Request = serde_json::from_str(raw).map_err(|_| "InvalidCacheRequest")?;
    let _guard = command_lock(root, "cache-cleanup.lock", false)?;
    let items = preview_items(root, places)?;
    if revision(&items)? != request.revision {
        return Err("CachePreviewChanged".into());
    }
    let mut results = Vec::new();
    for item in items.iter().filter(|item| item.state == State::Ready) {
        let outcome = clear_item(root, item);
        results.push(match outcome {
            Ok(outcome) => json!({"category":item.category,"path":item.path,"bytes":item.bytes,"outcome":outcome}),
            Err(reason) => json!({"category":item.category,"path":item.path,"bytes":item.bytes,"outcome":"failed","reason":reason}),
        });
    }
    let after = preview_items(root, places)?;
    Ok(json!({"results":results,"preview":preview_value(&after)?}))
}

pub(super) fn dispatch(root: &Path, verb: &str, args: &[String]) -> Option<Result<Value>> {
    Some(match (verb, args) {
        ("cache-preview", []) => Places::current(root).and_then(|places| preview_items(root, &places)).and_then(|items| preview_value(&items)),
        ("cache-clear", [raw]) => Places::current(root).and_then(|places| clear(root, &places, raw)),
        ("cache-preview" | "cache-clear", _) => Err("InvalidCacheRequest".into()),
        _ => return None,
    })
}

/// Runs once at GUI start, before WebView2 opens its folder: deletes the WebView2
/// folder that settings scheduled for this workspace. Returns None when nothing was scheduled.
pub fn clear_scheduled_webview() -> Option<Result<()>> {
    let root = PathBuf::from(std::env::var_os("IH_WORKSPACE")?);
    let root = safe_directory(&root).ok()?;
    let marker = read_marker(&root).ok()??;
    let places = match Places::current(&root) {
        Ok(places) => places,
        Err(error) => return Some(Err(error)),
    };
    Some(run_scheduled(&root, &places, marker))
}

fn run_scheduled(root: &Path, places: &Places, marker: Marker) -> Result<()> {
    let path = root.join(MARKER_FILE);
    let Ok(folder) = &places.webview else {
        // This process uses another folder; keep the request for a matching start.
        return Ok(());
    };
    if !same_location(Path::new(&marker.folder), folder) {
        return Ok(());
    }
    // The previous GUI's WebView2 processes can hold files for a moment after it exits.
    let mut outcome = remove_folder(folder);
    for _ in 0..10 {
        if outcome.is_ok() {
            break;
        }
        thread::sleep(Duration::from_millis(300));
        outcome = remove_folder(folder);
    }
    match outcome {
        Ok(()) => {
            crate::desktop_channel::create_plain_directory(folder)?;
            fs::remove_file(&path).map_err(|error| file_error("replace", &path, error))
        }
        Err(reason) => {
            // Keep the request, with the reason, so settings can show it and the next start retries.
            let failed = Marker { folder: marker.folder, error: Some(reason.clone()) };
            core_maintenance::write_durable(&path, &failed, true)?;
            Err(format!("WebViewCacheClearFailed: {reason}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        base: PathBuf,
        root: PathBuf,
        local: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let base = std::env::temp_dir().join(format!("ih-cache-{}-{}", std::process::id(), clock()));
            fs::create_dir(&base).unwrap();
            let base = fs::canonicalize(base).unwrap();
            let root = base.join("workspace");
            let local = base.join("local");
            for folder in [&root, &root.join("staging"), &root.join("instances"), &local] {
                fs::create_dir_all(folder).unwrap();
            }
            Self { base, root, local }
        }

        fn places(&self) -> Places {
            Places::new(&self.local, Some("agent-29"), None)
        }

        fn folder(&self, path: &Path, bytes: usize) {
            fs::create_dir_all(path.join("nested")).unwrap();
            fs::write(path.join("nested").join("data.bin"), vec![7u8; bytes]).unwrap();
        }

        fn preview(&self, places: &Places) -> Value {
            preview_value(&preview_items(&self.root, places).unwrap()).unwrap()
        }

        fn webview(&self, places: &Places) -> PathBuf {
            places.webview.clone().unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.base);
        }
    }

    fn hash(c: char) -> String {
        std::iter::repeat_n(c, 64).collect()
    }

    /// A PID that no running process has: Windows PIDs are multiples of four.
    const ENDED_PID: u32 = 4_294_967_293;

    // #62: a shared core is listed only while no instance names it, and an
    // open one is in use. Clearing checks its names again.
    #[test]
    fn unused_shared_cores_are_listed_and_cleared() {
        let fixture = Fixture::new();
        let places = fixture.places();
        let source = fixture.base.join("core.jar");
        fs::write(&source, b"shared core").unwrap();
        let sha = content_revision::revision(b"shared core").unwrap();
        let link = fixture.root.join("instances").join("server.jar");
        assert_eq!(shared_core::place(&fixture.root, &source, &sha, &link).unwrap(), shared_core::Placement::Linked);
        let shared = |preview: &Value| preview["items"].as_array().unwrap().iter()
            .filter(|item| item["category"] == "sharedCore").cloned().collect::<Vec<_>>();
        assert!(shared(&fixture.preview(&places)).is_empty(), "an instance still names it");
        fs::remove_file(&link).unwrap();
        let listed = shared(&fixture.preview(&places));
        assert_eq!((listed.len(), &listed[0]["state"], &listed[0]["bytes"]), (1, &json!("ready"), &json!(11)));
        let reader = File::open(fixture.root.join("shared-cores").join(format!("{sha}.jar"))).unwrap();
        assert_eq!(shared(&fixture.preview(&places))[0]["state"], "inUse");
        drop(reader);
        let revision = fixture.preview(&places)["revision"].as_str().unwrap().to_owned();
        let result = clear(&fixture.root, &places, &json!({"revision":revision}).to_string()).unwrap();
        assert!(result["results"].as_array().unwrap().iter().any(|row| row["category"] == "sharedCore" && row["outcome"] == "deleted"));
        assert!(!fixture.root.join("shared-cores").join(format!("{sha}.jar")).exists());
        assert!(shared(&result["preview"]).is_empty());
    }

    #[test]
    fn only_exact_download_leftover_names_are_cache_entries() {
        assert!(matches!(leftover(&format!("download-{}", hash('a'))), Some(Leftover::Verified(_))));
        assert!(matches!(leftover("pending-download-12-34"), Some(Leftover::Scratch(12))));
        assert!(matches!(leftover("core-download-12-34"), Some(Leftover::Scratch(12))));
        for name in [
            format!("download-{}", hash('A')),
            format!("download-{}", &hash('a')[1..]),
            format!("download-{}x", hash('a')),
            "download-".into(),
            "pending-download-12".into(),
            "pending-download-12-3x".into(),
            "core-download--1".into(),
            "core-prefetch".into(),
            "gui-create-work-1-2".into(),
            "gui-creation-paper.json".into(),
            "agent-request".into(),
        ] {
            assert!(leftover(&name).is_none(), "{name}");
        }
    }

    #[test]
    fn preview_lists_each_category_with_sizes_and_leaves_other_files() {
        let f = Fixture::new();
        let places = f.places();
        let staging = f.root.join("staging");
        f.folder(&staging.join(format!("download-{}", hash('b'))), 10);
        f.folder(&staging.join(format!("pending-download-{ENDED_PID}-1")), 20);
        f.folder(&staging.join(format!("core-download-{ENDED_PID}-2")), 30);
        f.folder(&staging.join("core-prefetch"), 40);
        f.folder(&staging.join("gui-create-work-1-2"), 50);
        fs::write(staging.join("gui-creation-paper.json"), "{}").unwrap();
        f.folder(&places.legacy, 60);
        f.folder(&f.webview(&places), 70);

        let preview = f.preview(&places);
        let items = preview["items"].as_array().unwrap();
        let summary: Vec<_> = items.iter().map(|item| (item["category"].as_str().unwrap(), item["bytes"].as_u64().unwrap(), item["state"].as_str().unwrap())).collect();
        assert_eq!(summary, [
            ("download", 30, "ready"),
            ("download", 10, "ready"),
            ("download", 20, "ready"),
            ("legacyWebView", 60, "ready"),
            ("webView", 70, "ready"),
        ]);
        assert!(items.iter().all(|item| item["complete"] == true));
    }

    #[test]
    fn entries_a_running_job_uses_are_listed_but_not_deleted() {
        let f = Fixture::new();
        let places = f.places();
        let staging = f.root.join("staging");
        let verified = staging.join(format!("download-{}", hash('c')));
        let scratch = staging.join(format!("pending-download-{}-1", std::process::id()));
        f.folder(&verified, 5);
        f.folder(&scratch, 5);
        let _creation = command_lock(&f.root, &format!("download-{}.lock", hash('c')), false).unwrap();

        let preview = f.preview(&places);
        let states: Vec<_> = preview["items"].as_array().unwrap().iter()
            .filter(|item| item["category"] == "download").map(|item| item["state"].as_str().unwrap()).collect();
        assert_eq!(states, ["inUse", "inUse"]);
        let result = clear(&f.root, &places, &json!({"revision":preview["revision"]}).to_string()).unwrap();
        assert!(result["results"].as_array().unwrap().iter().all(|item| item["category"] != "download"));
        assert!(verified.join("nested").join("data.bin").is_file());
        assert!(scratch.join("nested").join("data.bin").is_file());
    }

    #[test]
    fn clearing_deletes_listed_entries_and_schedules_the_webview_folder() {
        let f = Fixture::new();
        let places = f.places();
        let staging = f.root.join("staging");
        let verified = staging.join(format!("download-{}", hash('d')));
        f.folder(&verified, 5);
        f.folder(&staging.join("core-prefetch"), 5);
        f.folder(&places.legacy, 5);
        let webview = f.webview(&places);
        f.folder(&webview, 5);
        let registry = f.root.join("instances").join("server");
        f.folder(&registry, 5);

        let preview = f.preview(&places);
        let result = clear(&f.root, &places, &json!({"revision":preview["revision"]}).to_string()).unwrap();
        let outcomes: Vec<_> = result["results"].as_array().unwrap().iter().map(|item| item["outcome"].as_str().unwrap()).collect();
        assert_eq!(outcomes, ["deleted", "deleted", "scheduled"]);
        assert!(!verified.exists() && !places.legacy.exists());
        assert!(staging.join("core-prefetch").join("nested").is_dir());
        assert!(registry.join("nested").join("data.bin").is_file());
        // The running GUI keeps its folder until the next start.
        assert!(webview.join("nested").join("data.bin").is_file());
        let after = result["preview"]["items"].as_array().unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0]["state"], "scheduled");

        run_scheduled(&f.root, &places, read_marker(&f.root).unwrap().unwrap()).unwrap();
        assert!(webview.is_dir());
        assert_eq!(fs::read_dir(&webview).unwrap().count(), 0);
        assert!(!f.root.join(MARKER_FILE).exists());
        assert!(registry.join("nested").join("data.bin").is_file());
    }

    #[test]
    fn a_changed_listing_is_refused_and_failures_do_not_stop_other_entries() {
        let f = Fixture::new();
        let places = f.places();
        let staging = f.root.join("staging");
        let first = staging.join(format!("download-{}", hash('e')));
        f.folder(&first, 5);
        let preview = f.preview(&places);
        let second = staging.join(format!("download-{}", hash('f')));
        f.folder(&second, 5);
        let stale = clear(&f.root, &places, &json!({"revision":preview["revision"]}).to_string()).unwrap_err();
        assert_eq!(stale, "CachePreviewChanged");
        assert!(first.exists() && second.exists());

        // An open file without delete sharing makes one entry fail.
        use std::os::windows::fs::OpenOptionsExt;
        let _held = OpenOptions::new().read(true).share_mode(1).open(first.join("nested").join("data.bin")).unwrap();
        let preview = f.preview(&places);
        let result = clear(&f.root, &places, &json!({"revision":preview["revision"]}).to_string()).unwrap();
        let results = result["results"].as_array().unwrap();
        assert_eq!(results[0]["outcome"], "failed");
        assert_eq!(results[0]["reason"], "FileInUse");
        assert_eq!(results[1]["outcome"], "deleted");
        assert_eq!(results[2]["outcome"], "scheduled");
        assert!(!second.exists());
    }

    #[test]
    fn a_custom_webview_folder_is_never_scheduled_and_links_are_not_followed() {
        let f = Fixture::new();
        let custom = f.base.join("custom-webview");
        f.folder(&custom, 5);
        let places = Places::new(&f.local, Some("agent-29"), Some(&custom));
        let preview = f.preview(&places);
        let webview = preview["items"].as_array().unwrap().iter().find(|item| item["category"] == "webView").unwrap().clone();
        assert_eq!(webview["state"], "unavailable");
        assert_eq!(webview["reason"], "WebViewCacheCustomFolder");
        clear(&f.root, &places, &json!({"revision":preview["revision"]}).to_string()).unwrap();
        assert!(custom.join("nested").join("data.bin").is_file());
        assert!(!f.root.join(MARKER_FILE).exists());

        // A junction named like a download leftover is skipped, and its target stays.
        let target = f.base.join("user-data");
        f.folder(&target, 5);
        let junction = f.root.join("staging").join(format!("download-{}", hash('9')));
        let status = Command::new("cmd").args(["/C", "mklink", "/J"]).arg(&junction).arg(&target)
            .stdout(Stdio::null()).status().unwrap();
        assert!(status.success());
        let preview = f.preview(&places);
        assert!(preview["items"].as_array().unwrap().iter().all(|item| item["category"] != "download"));
        assert!(target.join("nested").join("data.bin").is_file());
        fs::remove_dir(&junction).unwrap();
    }

    #[test]
    fn a_failed_scheduled_clear_keeps_the_request_with_its_reason() {
        let f = Fixture::new();
        let places = f.places();
        let webview = f.webview(&places);
        f.folder(&webview, 5);
        core_maintenance::write_durable(&f.root.join(MARKER_FILE), &Marker { folder: path_format::display(&webview), error: None }, false).unwrap();
        use std::os::windows::fs::OpenOptionsExt;
        let held = OpenOptions::new().read(true).share_mode(1).open(webview.join("nested").join("data.bin")).unwrap();
        let error = run_scheduled(&f.root, &places, read_marker(&f.root).unwrap().unwrap()).unwrap_err();
        assert_eq!(error, "WebViewCacheClearFailed: FileInUse");
        drop(held);
        let preview = f.preview(&places);
        let item = preview["items"].as_array().unwrap().iter().find(|item| item["category"] == "webView").unwrap().clone();
        assert_eq!(item["state"], "scheduled");
        assert_eq!(item["reason"], "FileInUse");
        // A marker for another folder is left alone.
        let other = Places::new(&f.local, Some("agent-29"), Some(&f.base.join("other-webview")));
        run_scheduled(&f.root, &other, read_marker(&f.root).unwrap().unwrap()).unwrap();
        assert!(f.root.join(MARKER_FILE).exists());
    }
}
