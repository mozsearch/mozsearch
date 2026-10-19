//! The pipeline-server's budget of memory for its requests' expected costs
//! (see `PipelineCommand::estimate`, and the data API's), so that under load
//! the requests expected to take much memory (ex: a big try push's slicing, a
//! wide traversal) take turns, rather than all running at once and pushing
//! it past its cgroup's `MemoryHigh`, above which the kernel throttles all of
//! its requests (see infrastructure/pipeline-server-run.sh, whose cgroup
//! stays the backstop for wrong estimates).
//!
//! The budget is a semaphore of MiB: a request takes as many permits as it's
//! expected to take MiB, at most all of them (so that one expected to take
//! more than the budget runs alone, rather than never), and requests expected
//! to take less than `SMALL` don't take any.  The semaphore is FIFO, so a
//! big request waits for those before it, and those after it wait for it.
//! Bots' big requests are turned away rather than wait when the budget can't
//! have them (see the pipeline-server's `handle_query`).
//!
//! Each budgeted request is logged with its estimate, how long it waited, and
//! its cgroup's memory before and after, which is how the estimates are to be
//! calibrated (they're rough, from what the commands are asked for, without
//! lookups that would cost much themselves).

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use tokio::sync::{Semaphore, SemaphorePermit, TryAcquireError};

/// Requests expected to take less than this don't take from the budget.
pub const SMALL: u64 = 64 << 20;

const MIB: u64 = 1 << 20;

/// The budget's fraction of the cgroup's `memory.high`, and else of the
/// machine's memory.
const OF_MEMORY_HIGH: (u64, u64) = (1, 2);
const OF_MEMORY: (u64, u64) = (1, 8);

pub struct MemoryBudget {
    semaphore: Semaphore,
    mib: u32,
}

static BUDGET: OnceLock<MemoryBudget> = OnceLock::new();

/// The budget: `PIPELINE_SERVER_MEMORY_BUDGET_MIB` MiB, else half of its
/// cgroup's `memory.high`, else an eighth of the machine's memory.
pub fn budget() -> &'static MemoryBudget {
    BUDGET.get_or_init(|| {
        let bytes = std::env::var("PIPELINE_SERVER_MEMORY_BUDGET_MIB")
            .ok()
            .and_then(|mib| mib.parse::<u64>().ok())
            .map(|mib| mib * MIB)
            .or_else(|| {
                cgroup_file("memory.high").map(|high| high * OF_MEMORY_HIGH.0 / OF_MEMORY_HIGH.1)
            })
            .or_else(|| mem_total().map(|total| total * OF_MEMORY.0 / OF_MEMORY.1))
            .unwrap_or(4 << 30);
        let mib = (bytes / MIB).clamp(SMALL / MIB, u32::MAX as u64 >> 4) as u32;
        MemoryBudget {
            semaphore: Semaphore::new(mib as usize),
            mib,
        }
    })
}

/// A request's share of the budget, while it runs, and how long it waited
/// for it.
pub struct Share {
    _permit: Option<SemaphorePermit<'static>>,
    pub mib: u32,
    pub waited: Duration,
}

impl MemoryBudget {
    pub fn mib(&self) -> u32 {
        self.mib
    }

    /// The permits a request expected to take `bytes` takes.
    pub fn permits(&self, bytes: u64) -> u32 {
        if bytes < SMALL {
            return 0;
        }
        (bytes.div_ceil(MIB)).min(self.mib as u64) as u32
    }

    /// Wait for a request's share of the budget, for `bytes`.
    pub async fn acquire(&'static self, bytes: u64) -> Share {
        let mib = self.permits(bytes);
        let start = Instant::now();
        let permit = match mib {
            0 => None,
            n => self.semaphore.acquire_many(n).await.ok(),
        };
        Share {
            _permit: permit,
            mib,
            waited: start.elapsed(),
        }
    }

    /// A request's share of the budget, for `bytes`, if it has it now.
    pub fn try_acquire(&'static self, bytes: u64) -> Option<Share> {
        let mib = self.permits(bytes);
        let permit = match mib {
            0 => None,
            n => match self.semaphore.try_acquire_many(n) {
                Ok(permit) => Some(permit),
                Err(TryAcquireError::NoPermits) | Err(TryAcquireError::Closed) => return None,
            },
        };
        Some(Share {
            _permit: permit,
            mib,
            waited: Duration::ZERO,
        })
    }
}

/// A file of this process's cgroup (v2), as a number of bytes (not "max").
fn cgroup_file(name: &str) -> Option<u64> {
    let cgroup = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let path = cgroup.lines().find_map(|line| line.strip_prefix("0::"))?;
    std::fs::read_to_string(format!("/sys/fs/cgroup{}/{}", path, name))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// The cgroup's memory now (for logging estimates against), in MiB.
pub fn cgroup_memory_mib() -> Option<u64> {
    cgroup_file("memory.current").map(|bytes| bytes / MIB)
}

fn mem_total() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kib: u64 = meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))?
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse()
        .ok()?;
    Some(kib * 1024)
}

/// Estimates' units, for commands' `estimate`s.
pub const fn mib(n: u64) -> u64 {
    n * MIB
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_shares() {
        let budget: &'static MemoryBudget = Box::leak(Box::new(MemoryBudget {
            semaphore: Semaphore::new(1000),
            mib: 1000,
        }));
        assert_eq!(budget.permits(SMALL - 1), 0, "small requests take nothing");
        assert_eq!(budget.permits(mib(100)), 100);
        assert_eq!(budget.permits(mib(5000)), 1000, "at most the budget");
        let big = budget.acquire(mib(600)).await;
        assert_eq!(big.mib, 600);
        assert!(
            budget.try_acquire(mib(500)).is_none(),
            "the rest isn't enough"
        );
        assert!(budget.try_acquire(mib(10)).is_some(), "small ones go on");
        let other = budget.try_acquire(mib(400)).expect("the rest is");
        drop(big);
        drop(other);
        assert_eq!(budget.try_acquire(mib(5000)).unwrap().mib, 1000, "alone");
    }
}
