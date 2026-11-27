//! One background worker for replaceable, potentially slow requests.
//!
//! Exactly one job can run and at most one can wait. Submitting replaces the
//! pending job; the consumer must check the generation when handling a reply.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

struct State<T> {
    pending: Option<(u64, T)>,
    closed: bool,
    alive: bool,
}

struct Shared<T> {
    state: Mutex<State<T>>,
    wake: Condvar,
    generation: AtomicU64,
}

pub struct LatestWorker<T> {
    shared: Arc<Shared<T>>,
}

/// Marks an unexpectedly terminated worker unavailable even if its work or
/// delivery callback panicked. Submissions then report failure rather than
/// accumulating work that nobody can execute.
struct WorkerExit<T>(Arc<Shared<T>>);
impl<T> Drop for WorkerExit<T> {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap_or_else(|poison| poison.into_inner());
        state.alive = false;
        state.pending = None;
        self.0.wake.notify_all();
    }
}

impl<T: Send + 'static> LatestWorker<T> {
    pub fn start<R, F, D>(name: &str, work: F, deliver: D) -> std::io::Result<Self>
    where
        R: Send + 'static,
        F: Fn(T) -> R + Send + 'static,
        D: Fn(u64, R) + Send + 'static,
    {
        let shared = Arc::new(Shared {
            state: Mutex::new(State { pending: None, closed: false, alive: true }),
            wake: Condvar::new(),
            generation: AtomicU64::new(0),
        });
        let worker = shared.clone();
        std::thread::Builder::new().name(name.into()).spawn(move || {
            let _exit = WorkerExit(worker.clone());
            loop {
                let job = {
                    let mut state = worker.state.lock().unwrap_or_else(|poison| poison.into_inner());
                    while state.pending.is_none() && !state.closed {
                        state = worker.wake.wait(state).unwrap_or_else(|poison| poison.into_inner());
                    }
                    if state.closed { return; }
                    state.pending.take().expect("pending work under lock")
                };
                let (gen, value) = job;
                let result = work(value);
                if worker.generation.load(Ordering::Acquire) == gen {
                    deliver(gen, result);
                }
            }
        })?;
        Ok(Self { shared })
    }

    /// Non-blocking with respect to work: only a brief state lock is taken.
    pub fn submit(&self, value: T) -> bool {
        let mut state = self.shared.state.lock().unwrap_or_else(|poison| poison.into_inner());
        if state.closed || !state.alive { return false; }
        let gen = self.shared.generation.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
        state.pending = Some((gen, value));
        self.shared.wake.notify_one();
        true
    }

    pub fn is_current(&self, gen: u64) -> bool {
        self.shared.generation.load(Ordering::Acquire) == gen
    }
}

impl<T> Drop for LatestWorker<T> {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock().unwrap_or_else(|poison| poison.into_inner());
        state.closed = true;
        state.pending = None;
        self.shared.generation.fetch_add(1, Ordering::AcqRel);
        self.shared.wake.notify_all();
        // The current OS call is not cancelled or joined on the UI thread.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, time::Duration};
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
            move |gen, value| { out_tx.send((gen, value)).unwrap(); },
        ).unwrap();
        assert!(worker.submit(1));
        started_rx.recv_timeout(WAIT).unwrap();
        for i in 2..=100_000 { assert!(worker.submit(i)); }
        assert_eq!(worker.shared.state.lock().unwrap().pending.as_ref().unwrap().1, 100_000);
        release_tx.send(()).unwrap();
        assert_eq!(ran_rx.recv_timeout(WAIT).unwrap(), 1);
        assert_eq!(ran_rx.recv_timeout(WAIT).unwrap(), 100_000);
        let (generation, result) = out_rx.recv_timeout(WAIT).unwrap();
        assert_eq!(result, 100_000);
        assert!(worker.is_current(generation));
        assert!(out_rx.try_recv().is_err());
        assert!(ran_rx.try_recv().is_err());
    }

    #[test]
    fn concurrent_submissions_keep_only_the_last_ordered_request() {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (out_tx, out_rx) = mpsc::channel();
        let worker = Arc::new(LatestWorker::start("test-concurrent", move |value| {
            if value == 0 { started_tx.send(()).unwrap(); release_rx.recv_timeout(WAIT).unwrap(); }
            value
        }, move |gen, value| { out_tx.send((gen, value)).unwrap(); }).unwrap());
        assert!(worker.submit(0));
        started_rx.recv_timeout(WAIT).unwrap();
        let threads: Vec<_> = (0..8).map(|n| {
            let worker = worker.clone();
            std::thread::spawn(move || { for i in 0..1000 { assert!(worker.submit(1 + n * 1000 + i)); } })
        }).collect();
        for thread in threads { thread.join().unwrap(); }
        assert!(worker.submit(9000));
        assert_eq!(worker.shared.state.lock().unwrap().pending.as_ref().unwrap().1, 9000);
        release_tx.send(()).unwrap();
        let (gen, value) = out_rx.recv_timeout(WAIT).unwrap();
        assert_eq!(value, 9000);
        assert!(worker.is_current(gen));
        assert!(out_rx.try_recv().is_err());
    }

    #[test]
    fn consumer_must_recheck_even_an_already_delivered_reply() {
        let (out_tx, out_rx) = mpsc::channel();
        let worker = LatestWorker::start("test-current", |x| x, move |g, r| { let _ = out_tx.send((g, r)); }).unwrap();
        assert!(worker.submit("old"));
        let (old_gen, _) = out_rx.recv_timeout(WAIT).unwrap();
        assert!(worker.is_current(old_gen));
        assert!(worker.submit("new"));
        assert!(!worker.is_current(old_gen));
        let (new_gen, result) = out_rx.recv_timeout(WAIT).unwrap();
        assert_eq!(result, "new");
        assert!(worker.is_current(new_gen));
    }

    #[test]
    fn panic_marks_worker_unavailable() {
        let worker = LatestWorker::start("test-panic", |_: ()| -> () { panic!("injected panic") }, |_, _| {}).unwrap();
        assert!(worker.submit(()));
        let state = worker.shared.state.lock().unwrap();
        let (state, timeout) = worker.shared.wake.wait_timeout_while(state, WAIT, |s| s.alive).unwrap();
        assert!(!timeout.timed_out());
        assert!(!state.alive);
        drop(state);
        assert!(!worker.submit(()));
    }

    #[test]
    fn drop_discards_pending_and_suppresses_inflight_reply() {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (ran_tx, ran_rx) = mpsc::channel();
        let (out_tx, out_rx) = mpsc::channel();
        let worker = LatestWorker::start("test-drop", move |n| {
            ran_tx.send(n).unwrap();
            started_tx.send(()).unwrap();
            release_rx.recv_timeout(WAIT).unwrap();
            n
        }, move |_, n| { let _ = out_tx.send(n); }).unwrap();
        assert!(worker.submit(1)); started_rx.recv_timeout(WAIT).unwrap();
        assert!(worker.submit(2));
        let shared = worker.shared.clone();
        drop(worker);
        release_tx.send(()).unwrap();
        let state = shared.state.lock().unwrap();
        let (state, timeout) = shared.wake.wait_timeout_while(state, WAIT, |s| s.alive).unwrap();
        assert!(!timeout.timed_out());
        assert!(state.pending.is_none());
        assert_eq!(ran_rx.try_iter().collect::<Vec<_>>(), vec![1]);
        assert!(out_rx.try_recv().is_err());
    }
}
