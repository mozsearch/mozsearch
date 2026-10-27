//! A queue of files for threads to process, by size, under a budget of the
//! sizes being processed at once, for tools whose memory processing a file is
//! some multiple of its size (dataflow-summarize, normalize-analyses): a
//! thread that's free takes the biggest file left that fits in what's left of
//! the budget (a file bigger than the budget counts as the budget, so it's
//! processed alone), or waits for one to.  So the big files start first, and
//! small ones fill the threads while they run (rather than files in a list's
//! order, where a thread that took a big file waited for the budget, idle, or
//! static chunks of the list, where the chunk with the big files straggled).

use std::collections::BTreeSet;
use std::sync::{Condvar, Mutex};

pub struct BudgetQueue {
    state: Mutex<QueueState>,
    freed: Condvar,
    cap: u64,
}

struct QueueState {
    /// (Size, index into the files), for the files left.
    left: BTreeSet<(u64, usize)>,
    used: u64,
}

impl BudgetQueue {
    /// A queue of the files of these sizes (their indexes the callers'),
    /// with a budget of `cap`.
    pub fn new(sizes: impl IntoIterator<Item = (usize, u64)>, cap: u64) -> Self {
        BudgetQueue {
            state: Mutex::new(QueueState {
                left: sizes.into_iter().map(|(i, size)| (size, i)).collect(),
                used: 0,
            }),
            freed: Condvar::new(),
            cap,
        }
    }

    /// The next file to process (its index), and the budget it takes (to
    /// `release` once it's done), waiting for one to fit; `None` once none
    /// are left.
    pub fn take(&self) -> Option<(usize, u64)> {
        let mut state = self.state.lock().unwrap();
        loop {
            let room = self.cap.saturating_sub(state.used);
            let fits = match state.left.last().copied() {
                // (A file bigger than the budget, alone, first: it's the
                // longest.)
                Some(biggest) if biggest.0 >= self.cap => (state.used == 0).then_some(biggest),
                // (The biggest that fits.)
                _ => state
                    .left
                    .range(..(room.saturating_add(1), 0))
                    .next_back()
                    .copied(),
            };
            match fits {
                Some(entry) => {
                    state.left.remove(&entry);
                    let taken = entry.0.min(self.cap);
                    state.used += taken;
                    return Some((entry.1, taken));
                }
                None if state.left.is_empty() => return None,
                None => state = self.freed.wait(state).unwrap(),
            }
        }
    }

    /// The budget.
    pub fn cap(&self) -> u64 {
        self.cap
    }

    pub fn release(&self, bytes: u64) {
        self.state.lock().unwrap().used -= bytes;
        self.freed.notify_all();
    }
}

/// Give the memory freed in all of the threads' heaps back to the system
/// (glibc keeps it in each thread's arena otherwise).
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub fn release_freed_memory() {
    unsafe extern "C" {
        fn malloc_trim(pad: usize) -> std::os::raw::c_int;
    }
    // SAFETY: malloc_trim only releases free memory.
    unsafe {
        malloc_trim(0);
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub fn release_freed_memory() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_budget_queue_order() {
        // (Biggest first; the one bigger than the budget alone, as the
        // budget; then the biggest that fit in what's left.)
        let queue = BudgetQueue::new([(0, 5), (1, 50), (2, 200), (3, 30), (4, 1)], 100);
        assert_eq!(queue.take(), Some((2, 100)));
        queue.release(100);
        assert_eq!(queue.take(), Some((1, 50)));
        assert_eq!(queue.take(), Some((3, 30)));
        assert_eq!(queue.take(), Some((0, 5)));
        assert_eq!(queue.take(), Some((4, 1)));
        assert_eq!(queue.take(), None);
    }

    #[test]
    fn test_budget_queue_threads() {
        // (Threads never have more than the budget at once.)
        let sizes: Vec<u64> = (0..200).map(|i| (i * 37 % 61) + 1).collect();
        let queue = BudgetQueue::new(sizes.iter().copied().enumerate(), 100);
        let in_use = std::sync::atomic::AtomicU64::new(0);
        let done = std::sync::atomic::AtomicUsize::new(0);
        std::thread::scope(|s| {
            for _ in 0..8 {
                s.spawn(|| {
                    while let Some((_, taken)) = queue.take() {
                        let now = in_use.fetch_add(taken, std::sync::atomic::Ordering::SeqCst);
                        assert!(now + taken <= 100);
                        std::thread::yield_now();
                        in_use.fetch_sub(taken, std::sync::atomic::Ordering::SeqCst);
                        done.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        queue.release(taken);
                    }
                });
            }
        });
        assert_eq!(done.into_inner(), 200);
    }
}
