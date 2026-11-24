//! The page's first preferences and instance-list reads, started before WebView2.
//!
//! Creating the WebView2 window takes most of a cold start, and the page can only
//! ask for these reads after it has loaded. Each read starts on its own thread
//! when the workspace is bound and goes through the same recorded `manage` path
//! as a page call. The page's first background call for that verb receives the
//! reply; a reply older than `MAX_AGE` is dropped and the page reads again.
use serde_json::Value;
use std::{
    sync::Mutex,
    thread::JoinHandle,
    time::{Duration, Instant},
};

/// The reads the page makes before the first list and window are on screen.
pub(super) const VERBS: [&str; 2] = ["preferences-snapshot", "instance-snapshot"];
/// The page polls every 1.5 s, so an older first reply would be a stale list.
const MAX_AGE: Duration = Duration::from_millis(1500);

type Reply = (Instant, Result<Value, String>);
pub(super) struct Pending(JoinHandle<Reply>);

#[derive(Default)]
pub(super) struct Prefetch {
    pending: Mutex<Vec<(&'static str, JoinHandle<Reply>)>>,
}

impl Prefetch {
    pub(super) const fn new() -> Self {
        Self { pending: Mutex::new(Vec::new()) }
    }

    pub(super) fn start(&self, verb: &'static str, read: impl FnOnce() -> Result<Value, String> + Send + 'static) {
        let spawned = std::thread::Builder::new()
            .name(format!("prefetch-{verb}"))
            .spawn(move || {
                let result = read();
                (Instant::now(), result)
            });
        match (spawned, self.pending.lock()) {
            (Ok(handle), Ok(mut pending)) => pending.push((verb, handle)),
            (Err(error), _) => eprintln!("Could not start the {verb} prefetch: {error}"),
            (_, Err(_)) => eprintln!("Could not keep the {verb} prefetch"),
        }
    }

    /// The pending read for the page's first background call with exactly this verb.
    pub(super) fn take(&self, arguments: &[String], origin: Option<&str>) -> Option<Pending> {
        let [verb] = arguments else { return None };
        if origin == Some("manual") {
            return None;
        }
        let mut pending = self.pending.lock().ok()?;
        let index = pending.iter().position(|(name, _)| name == verb)?;
        Some(Pending(pending.swap_remove(index).1))
    }
}

impl Pending {
    /// Waits for the read; None when it failed to run or is too old to show.
    pub(super) fn finish(self) -> Option<Result<Value, String>> {
        let (finished, result) = self.0.join().ok()?;
        fresh(finished, Instant::now()).then_some(result)
    }
}

fn fresh(finished: Instant, now: Instant) -> bool {
    now.saturating_duration_since(finished) <= MAX_AGE
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn arguments(verb: &str) -> Vec<String> {
        vec![verb.to_owned()]
    }

    #[test]
    fn only_the_first_background_call_with_the_exact_verb_takes_the_reply() {
        let prefetch = Prefetch::new();
        prefetch.start("instance-snapshot", || Ok(json!({"rows":[]})));
        assert!(prefetch.take(&arguments("instance-snapshot"), Some("manual")).is_none());
        assert!(prefetch.take(&["instance-snapshot".into(), "rows".into()], None).is_none());
        assert!(prefetch.take(&arguments("preferences-snapshot"), None).is_none());
        let pending = prefetch.take(&arguments("instance-snapshot"), Some("background")).unwrap();
        assert_eq!(pending.finish(), Some(Ok(json!({"rows":[]}))));
        assert!(prefetch.take(&arguments("instance-snapshot"), None).is_none());
    }

    #[test]
    fn errors_reach_the_page_as_a_page_call_would_report_them() {
        let prefetch = Prefetch::new();
        prefetch.start("preferences-snapshot", || Err("ManagerUnavailable".into()));
        let pending = prefetch.take(&arguments("preferences-snapshot"), None).unwrap();
        assert_eq!(pending.finish(), Some(Err("ManagerUnavailable".into())));
    }

    #[test]
    fn a_panicked_read_lets_the_page_read_again() {
        let prefetch = Prefetch::new();
        prefetch.start("instance-snapshot", || panic!("read failed"));
        assert_eq!(prefetch.take(&arguments("instance-snapshot"), None).unwrap().finish(), None);
    }

    #[test]
    fn a_reply_older_than_one_poll_is_not_shown() {
        let finished = Instant::now();
        assert!(fresh(finished, finished + MAX_AGE));
        assert!(!fresh(finished, finished + MAX_AGE + Duration::from_millis(1)));
    }
}
