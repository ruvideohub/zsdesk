//! Message-loop timing hook. Kept separate so Fences diagnostics do not use Explorer's window host.

pub struct IterationTimer;
impl IterationTimer {
    pub fn new<T>(_message: &T) -> Self { Self }
}
