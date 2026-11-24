//! One background worker for replaceable, potentially slow requests.
//!
//! At most one request runs at once. Pending requests are coalesced to the latest;
//! the generation must be checked again by the consumer when it handles a reply.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};

pub struct LatestWorker<T> {
    tx: mpsc::Sender<(u64, T)>,
    generation: Arc<AtomicU64>,
    // Assigning a generation and sending are one ordered operation, even if
    // callers submit from different threads.
    submit_lock: Mutex<()>,
}

impl<T: Send + 'static> LatestWorker<T> {
    pub fn start<R, F, D>(name: &str, work: F, deliver: D) -> std::io::Result<Self>
    where
        R: Send + 'static,
        F: Fn(T) -> R + Send + 'static,
        D: Fn(u64, R) + Send + 'static,
    {
        let (tx, rx) = mpsc::channel::<(u64, T)>();
        let generation = Arc::new(AtomicU64::new(0));
        let current = generation.clone();
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                while let Ok(mut job) = rx.recv() {
                    while let Ok(newer) = rx.try_recv() {
                        job = newer;
                    }
                    let (gen, value) = job;
                    let result = work(value);
                    if current.load(Ordering::Acquire) == gen {
                        deliver(gen, result);
                    }
                }
            })?;
        Ok(Self {
            tx,
            generation,
            submit_lock: Mutex::new(()),
        })
    }

    pub fn submit(&self, value: T) -> bool {
        let _guard = self.submit_lock.lock().unwrap();
        let gen = self
            .generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        self.tx.send((gen, value)).is_ok()
    }

    pub fn is_current(&self, gen: u64) -> bool {
        self.generation.load(Ordering::Acquire) == gen
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    const WAIT: Duration = Duration::from_secs(5);

    #[test]
    fn slow_work_does_not_block_submit_and_only_latest_pending_is_delivered() {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (ran_tx, ran_rx) = mpsc::channel();
        let (out_tx, out_rx) = mpsc::channel();
        let worker = LatestWorker::start(
            "test-latest",
            move |value| {
                ran_tx.send(value).unwrap();
                if value == 1 {
                    started_tx.send(()).unwrap();
                    release_rx.recv_timeout(WAIT).unwrap();
                }
                value
            },
            move |gen, value| {
                out_tx.send((gen, value)).unwrap();
            },
        )
        .unwrap();
        assert!(worker.submit(1));
        started_rx.recv_timeout(WAIT).unwrap();
        // These return while the first job is blocked; no new thread is spawned.
        assert!(worker.submit(2));
        assert!(worker.submit(3));
        release_tx.send(()).unwrap();
        assert_eq!(ran_rx.recv_timeout(WAIT).unwrap(), 1);
        assert_eq!(ran_rx.recv_timeout(WAIT).unwrap(), 3);
        let (generation, result) = out_rx.recv_timeout(WAIT).unwrap();
        assert_eq!(result, 3);
        assert!(worker.is_current(generation));
        assert!(out_rx.try_recv().is_err());
        assert!(ran_rx.try_recv().is_err());
    }

    #[test]
    fn consumer_must_recheck_even_an_already_delivered_reply() {
        let (out_tx, out_rx) = mpsc::channel();
        let worker = LatestWorker::start(
            "test-current",
            |x| x,
            move |g, r| {
                let _ = out_tx.send((g, r));
            },
        )
        .unwrap();
        assert!(worker.submit("old"));
        let (old_gen, _) = out_rx.recv_timeout(WAIT).unwrap();
        assert!(worker.is_current(old_gen));
        assert!(worker.submit("new"));
        assert!(!worker.is_current(old_gen));
        let (new_gen, result) = out_rx.recv_timeout(WAIT).unwrap();
        assert_eq!(result, "new");
        assert!(worker.is_current(new_gen));
    }
}
