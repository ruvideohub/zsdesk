//! Restart policy for isolated desktop widget processes.
use std::time::{Duration, Instant};

pub const BACKOFF: Duration = Duration::from_secs(30);

/// A missing or exited child remains eligible after backoff; a running child never is.
pub fn due(child_running: bool, now: Instant, retry_at: Instant) -> bool {
    !child_running && now >= retry_at
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_child_is_retried_after_backoff_not_forgotten() {
        let now = Instant::now();
        let next = now + BACKOFF;
        assert!(!due(false, now, next));
        assert!(due(false, next, next));
        assert!(due(false, next + BACKOFF, next));
        assert!(!due(true, next + BACKOFF, next));
    }
}
