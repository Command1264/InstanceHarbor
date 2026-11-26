//! How often the GUI repeats `gui-attach` with its workspace's Manager.

use std::time::Duration;

/// The pause before the next attach after `failures` failed attaches in a row. Each
/// failed attach may start `manager-serve` again, so a Manager that keeps failing is
/// started at most once a minute per GUI instead of every 5 s (#118).
pub(super) fn retry_delay(failures: u32) -> Duration {
    Duration::from_secs((5u64 << failures.min(4)).min(60))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_attaches_back_off_to_one_minute() {
        let delays: Vec<_> = (0..7).map(|failures| retry_delay(failures).as_secs()).collect();
        assert_eq!(delays, [5, 10, 20, 40, 60, 60, 60]);
        assert_eq!(retry_delay(u32::MAX), Duration::from_secs(60));
    }
}
