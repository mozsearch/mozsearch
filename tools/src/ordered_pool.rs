//! A pool of worker threads which compute jobs in whatever order threads are
//! free, and give their results in the jobs' order: each thread has its own
//! state (ex: a git repository, which isn't `Sync`), takes the next job when
//! it's done with one, and the results which arrive early wait for those
//! before them.  At most `window` jobs are submitted but not taken (the
//! callers submit as they take), which bounds the results waiting, and so the
//! memory.
//!
//! The history tools (build-blame, build-syntax-token-tree) used to deal their
//! revisions out to their threads round-robin, each thread computing its in
//! order: a big revision (ex: a history window's start, or a big merge) held
//! up the revisions behind it on its thread, and then the other threads,
//! whose results the main thread couldn't take until the big one's, idled.

use std::collections::HashMap;
use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread;

use log::info;

pub struct OrderedPool<J, R> {
    jobs: Option<Sender<(usize, J)>>,
    results: Receiver<(usize, thread::Result<R>)>,
    early: HashMap<usize, R>,
    submitted: usize,
    taken: usize,
}

impl<J: Send + 'static, R: Send + 'static> OrderedPool<J, R> {
    /// Start `threads` threads (at least one), each with its state from
    /// `init`, computing jobs with `work`.
    pub fn new<S>(
        threads: usize,
        init: impl Fn() -> S + Send + Sync + 'static,
        work: impl Fn(&mut S, J) -> R + Send + Sync + 'static,
    ) -> Self {
        let (jobs, job_rx) = channel::<(usize, J)>();
        let job_rx = Arc::new(Mutex::new(job_rx));
        let (result_tx, results) = channel();
        let (init, work) = (Arc::new(init), Arc::new(work));
        for _ in 0..threads.max(1) {
            let (job_rx, result_tx) = (job_rx.clone(), result_tx.clone());
            let (init, work) = (init.clone(), work.clone());
            thread::spawn(move || {
                let mut state = init();
                loop {
                    let job = job_rx.lock().unwrap().recv();
                    let Ok((index, job)) = job else {
                        break;
                    };
                    // (A panic goes to the main thread, which panics with it,
                    // rather than leaving it waiting.)
                    let result = panic::catch_unwind(AssertUnwindSafe(|| work(&mut state, job)));
                    if let Err(err) = &result {
                        eprintln!("Compute thread panicked computing job {}: {:?}", index, err);
                    }
                    let failed = result.is_err();
                    if result_tx.send((index, result)).is_err() || failed {
                        break;
                    }
                }
            });
        }
        OrderedPool {
            jobs: Some(jobs),
            results,
            early: HashMap::new(),
            submitted: 0,
            taken: 0,
        }
    }

    /// Submit a job, whose result `take` gives after those of the jobs
    /// submitted before it.
    pub fn submit(&mut self, job: J) {
        self.jobs
            .as_ref()
            .unwrap()
            .send((self.submitted, job))
            .expect("the compute threads are gone");
        self.submitted += 1;
    }

    /// How many jobs are submitted but not taken.
    pub fn pending(&self) -> usize {
        self.submitted - self.taken
    }

    /// The next job's result, waiting for it if it isn't done.  (A panic in a
    /// thread computing a job is resumed here.)
    pub fn take(&mut self) -> R {
        assert!(self.taken < self.submitted, "no job to take");
        loop {
            if let Some(result) = self.early.remove(&self.taken) {
                self.taken += 1;
                return result;
            }
            let (index, result) = match self.results.try_recv() {
                Ok(received) => received,
                Err(_) => {
                    info!("Waiting on compute, work on optimizing that...");
                    self.results.recv().expect("the compute threads are gone")
                }
            };
            match result {
                Ok(result) => {
                    self.early.insert(index, result);
                }
                Err(err) => panic::resume_unwind(err),
            }
        }
    }
}

impl<J, R> Drop for OrderedPool<J, R> {
    fn drop(&mut self) {
        // (The threads finish their jobs and exit; nothing waits for them.)
        self.jobs.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ordered_pool() {
        // (Jobs which take longer the earlier they are, so they finish out of
        // order, with each thread's own state.)
        let mut pool = OrderedPool::new(
            4,
            || 0usize,
            |done: &mut usize, n: usize| {
                std::thread::sleep(std::time::Duration::from_millis((20 - n as u64) * 2));
                *done += 1;
                n * 10
            },
        );
        let mut results = vec![];
        let mut next = 0;
        while next < 6 {
            pool.submit(next);
            next += 1;
        }
        while pool.pending() > 0 {
            results.push(pool.take());
            if next < 20 {
                pool.submit(next);
                next += 1;
            }
        }
        assert_eq!(results, (0..20).map(|n| n * 10).collect::<Vec<_>>());
    }

    #[test]
    #[should_panic(expected = "job 3")]
    fn test_ordered_pool_panic() {
        let mut pool = OrderedPool::new(
            2,
            || (),
            |_: &mut (), n: usize| {
                if n == 3 {
                    panic!("job 3");
                }
                n
            },
        );
        for n in 0..5 {
            pool.submit(n);
        }
        for _ in 0..5 {
            pool.take();
        }
    }
}
