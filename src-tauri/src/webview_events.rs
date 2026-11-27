//! WebView2 start-up and browser-process events for the GUI diagnostics log (#117).
//!
//! Each event is written as it happens and also kept, so the startup deadline record
//! can say which WebView2 step was reached. Events carry fixed names, times since the
//! process started, and Windows result codes; never paths or page content.
use serde_json::{json, Value};
use std::sync::Mutex;

static EVENTS: Mutex<Vec<Value>> = Mutex::new(Vec::new());
/// A browser process that keeps failing must not fill the log.
const LIMIT: usize = 32;

fn push(event: Value) {
    if let Ok(mut events) = EVENTS.lock() {
        if events.len() < LIMIT {
            events.push(event.clone());
        }
    }
    if let Ok(log) = instanceharbor::diagnostics::global() {
        if let Err(error) = log.record(event) {
            eprintln!("GUI diagnostic write failed: {error}");
        }
    }
}

fn at() -> u64 {
    crate::startup_timing::since_start().as_millis() as u64
}

/// A step of WebView2 creation: `environment` or `controller`, and its HRESULT on failure.
pub fn step(step: &'static str, phase: &'static str, error: Option<i32>) {
    let mut event = json!({"type":"webview2","step":step,"phase":phase,"atMs":at()});
    if let Some(code) = error {
        event["hresult"] = json!(format!("0x{:08X}", code as u32));
    }
    push(event);
}

/// WebView2's ProcessFailed event: which process ended and why, as WebView2 names them.
pub fn process_failed(kind: i32, reason: Option<i32>, exit_code: Option<i32>) {
    push(json!({
        "type":"webview2",
        "step":"process-failed",
        "atMs":at(),
        "kind":process_kind(kind),
        "reason":reason,
        "exitCode":exit_code,
    }));
}

/// COREWEBVIEW2_PROCESS_FAILED_KIND names.
fn process_kind(kind: i32) -> &'static str {
    match kind {
        0 => "browser-process-exited",
        1 => "render-process-exited",
        2 => "render-process-unresponsive",
        3 => "frame-render-process-exited",
        4 => "utility-process-exited",
        5 => "sandbox-helper-process-exited",
        6 => "gpu-process-exited",
        7 => "ppapi-plugin-process-exited",
        8 => "ppapi-broker-process-exited",
        _ => "unknown",
    }
}

/// The events so far, for the startup deadline record.
pub fn snapshot() -> Value {
    EVENTS
        .lock()
        .map(|events| Value::Array(events.clone()))
        .unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_kinds_use_webview2_names() {
        assert_eq!(process_kind(0), "browser-process-exited");
        assert_eq!(process_kind(2), "render-process-unresponsive");
        assert_eq!(process_kind(99), "unknown");
    }
}
