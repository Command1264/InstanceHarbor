use std::time::Duration;

/// A stop intent belongs to one retained child handle. Repeated client requests
/// do not reset its budget, and readiness gates the first command.
#[derive(Default)]
pub(super) struct StopRetry {
    requested: bool,
    attempts: u8,
    last: Option<Duration>,
}
impl StopRetry {
    pub(super) fn request(&mut self) {
        self.requested = true;
    }
    pub(super) fn due(&mut self, now: Duration, ready: bool) -> bool {
        if !self.requested
            || !ready
            || self.attempts >= 3
            || self
                .last
                .is_some_and(|last| now.saturating_sub(last) < Duration::from_secs(5))
        {
            return false;
        }
        self.attempts += 1;
        self.last = Some(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn waits_for_readiness_then_retries_at_bounded_intervals() {
        let mut stop = StopRetry::default();
        stop.request();
        assert!(!stop.due(Duration::from_secs(30), false));
        assert!(stop.due(Duration::from_secs(31), true));
        assert!(!stop.due(Duration::from_secs(35), true));
        assert!(stop.due(Duration::from_secs(36), true));
        assert!(stop.due(Duration::from_secs(41), true));
        stop.request();
        assert!(!stop.due(Duration::from_secs(100), true));
    }
    #[test]
    fn never_stops_without_an_explicit_intent() {
        assert!(!StopRetry::default().due(Duration::from_secs(100), true));
    }
}
