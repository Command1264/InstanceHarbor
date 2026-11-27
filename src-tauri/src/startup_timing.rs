//! Cold-start timing marks for the GUI diagnostics log.
//!
//! Each mark is milliseconds since the Windows process was created, so the time the
//! loader spends before `main` is counted too. The page reports its own marks once,
//! after the first instance list is on screen; one `gui-startup` event then holds
//! both sets. Marks carry only fixed names and numbers, never paths or values.
use serde_json::{json, Map, Value};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};

/// Marks the page may report, in the order it reaches them.
pub(super) const PAGE_MARKS: &[&str] = &[
    "page-navigation-start",
    "page-script-start",
    "page-mounted",
    "page-preferences-read",
    "page-window-presented",
    "page-first-list-read",
    "page-first-list-rendered",
    "page-first-list-painted",
];
/// A page mark later than this is a clock problem, not a slow start.
const PAGE_MARK_LIMIT_MS: f64 = 10.0 * 60.0 * 1000.0;

struct Clock {
    created_unix_ms: f64,
    marks: Mutex<Vec<(&'static str, f64)>>,
}

static CLOCK: OnceLock<Clock> = OnceLock::new();
static REPORTED: AtomicBool = AtomicBool::new(false);

fn unix_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
        * 1000.0
}

#[cfg(windows)]
fn process_created_unix_ms() -> Option<f64> {
    use std::ffi::c_void;
    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn GetProcessTimes(
            process: *mut c_void,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
    }
    let mut times: [FileTime; 4] = Default::default();
    let [creation, exit, kernel, user] = &mut times;
    let ok = unsafe { GetProcessTimes(GetCurrentProcess(), creation, exit, kernel, user) };
    if ok == 0 {
        return None;
    }
    // FILETIME counts 100 ns intervals since 1601-01-01.
    let ticks = (u64::from(times[0].high) << 32) | u64::from(times[0].low);
    Some(ticks as f64 / 10_000.0 - 11_644_473_600_000.0)
}

#[cfg(not(windows))]
fn process_created_unix_ms() -> Option<f64> {
    None
}

fn clock() -> &'static Clock {
    CLOCK.get_or_init(|| {
        let now = unix_ms();
        // A creation time after now or far in the past cannot be this process.
        let created = process_created_unix_ms()
            .filter(|created| *created <= now && now - created < PAGE_MARK_LIMIT_MS)
            .unwrap_or(now);
        Clock { created_unix_ms: created, marks: Mutex::new(Vec::new()) }
    })
}

/// Records a native mark. The first call also fixes the process creation time.
pub(super) fn mark(name: &'static str) {
    let clock = clock();
    let at = unix_ms() - clock.created_unix_ms;
    if let Ok(mut marks) = clock.marks.lock() {
        if marks.len() < 64 && !marks.iter().any(|(existing, _)| *existing == name) {
            marks.push((name, at));
        }
    }
}

/// The last native mark reached, so a stuck start can say where it stopped.
pub(super) fn last_mark() -> Option<&'static str> {
    clock().marks.lock().ok()?.iter().max_by(|a, b| a.1.total_cmp(&b.1)).map(|(name, _)| *name)
}

/// Time since the process was created.
pub(super) fn since_start() -> std::time::Duration {
    std::time::Duration::from_secs_f64(((unix_ms() - clock().created_unix_ms) / 1000.0).max(0.0))
}

fn round(value: f64) -> Value {
    json!((value * 10.0).round() / 10.0)
}

/// Keeps only known page marks inside the plausible window, relative to process creation.
pub(super) fn page_marks(created_unix_ms: f64, reported: &BTreeMap<String, f64>) -> Map<String, Value> {
    let mut marks = Map::new();
    for name in PAGE_MARKS {
        let Some(at) = reported.get(*name).copied() else { continue };
        let relative = at - created_unix_ms;
        // A page clock a little before process creation is rounding, not a later run.
        if at.is_finite() && relative > -50.0 && relative < PAGE_MARK_LIMIT_MS {
            marks.insert((*name).to_owned(), round(relative.max(0.0)));
        }
    }
    marks
}

/// Builds the single `gui-startup` event, or None after the first report.
pub(super) fn report(reported: &BTreeMap<String, f64>) -> Result<Option<Value>, String> {
    if reported.len() > PAGE_MARKS.len() || reported.keys().any(|name| !PAGE_MARKS.contains(&name.as_str())) {
        return Err("InvalidStartupMarks".into());
    }
    if REPORTED.swap(true, Ordering::SeqCst) {
        return Ok(None);
    }
    let clock = clock();
    let mut native = Map::new();
    if let Ok(marks) = clock.marks.lock() {
        for (name, at) in marks.iter() {
            native.insert((*name).to_owned(), round(*at));
        }
    }
    Ok(Some(json!({
        "type": "gui-startup",
        "native": native,
        "page": page_marks(clock.created_unix_ms, reported),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_marks_are_relative_to_process_creation_and_bounded() {
        let reported = BTreeMap::from([
            ("page-mounted".to_owned(), 1_000_250.0),
            ("page-first-list-rendered".to_owned(), 1_000_900.04),
            ("page-script-start".to_owned(), 999_990.0),
            ("page-window-presented".to_owned(), f64::NAN),
            ("page-first-list-painted".to_owned(), 1_000_000.0 + PAGE_MARK_LIMIT_MS),
        ]);
        let marks = page_marks(1_000_000.0, &reported);
        assert_eq!(marks["page-mounted"], json!(250.0));
        assert_eq!(marks["page-first-list-rendered"], json!(900.0));
        // Within rounding of process creation counts as zero.
        assert_eq!(marks["page-script-start"], json!(0.0));
        assert!(!marks.contains_key("page-window-presented"));
        assert!(!marks.contains_key("page-first-list-painted"));
    }

    #[test]
    fn unknown_page_marks_are_rejected_without_using_the_report() {
        let reported = BTreeMap::from([("C:\\Users\\someone".to_owned(), 1.0)]);
        assert_eq!(report(&reported), Err("InvalidStartupMarks".into()));
    }

    #[test]
    fn native_marks_keep_the_first_time_for_each_name() {
        mark("test-mark");
        let first = clock().marks.lock().unwrap().iter().find(|(name, _)| *name == "test-mark").unwrap().1;
        mark("test-mark");
        let marks = clock().marks.lock().unwrap();
        assert_eq!(marks.iter().filter(|(name, _)| *name == "test-mark").count(), 1);
        assert_eq!(marks.iter().find(|(name, _)| *name == "test-mark").unwrap().1, first);
        assert!(first >= 0.0);
    }
}
