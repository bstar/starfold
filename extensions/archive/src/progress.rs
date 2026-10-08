use std::sync::atomic::{AtomicU64, Ordering};
#[derive(Default)]
pub struct Progress {
    done: AtomicU64,
}
impl Progress {
    pub fn is_cancelled(&self) -> bool {
        false
    }
    pub fn add(&self, n: u64) {
        self.done.fetch_add(n, Ordering::Relaxed);
    }
    pub fn done(&self) -> u64 {
        self.done.load(Ordering::Relaxed)
    }
}
