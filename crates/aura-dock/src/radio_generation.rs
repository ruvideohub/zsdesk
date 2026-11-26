//! Failure callbacks are tagged with the station generation that created them.
//! This prevents a late failure from a closed player from failing a new one.
use std::sync::atomic::{AtomicU64, Ordering};

pub fn record_failure(status: &AtomicU64, generation: u64) {
    let _ = status.compare_exchange(
        generation,
        generation | 1,
        Ordering::SeqCst,
        Ordering::SeqCst,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn late_failure_cannot_poison_new_station() {
        let status = AtomicU64::new(2);
        record_failure(&status, 2);
        assert_eq!(status.load(Ordering::SeqCst), 3);
        status.store(4, Ordering::SeqCst);
        record_failure(&status, 2);
        assert_eq!(status.load(Ordering::SeqCst), 4);
        record_failure(&status, 4);
        assert_eq!(status.load(Ordering::SeqCst), 5);
    }
}
