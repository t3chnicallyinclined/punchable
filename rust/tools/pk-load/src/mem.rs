//! The API process's memory while the load runs, when it runs on the same machine (`--pid`): the resident set as
//! the OS reports it (`VmRSS` on Linux, the working set on Windows), sampled on a fixed interval, with the peak kept.
//! On the server the same number comes from cAdvisor's `container_memory_working_set_bytes` instead
//! (`infra/README.md` "Memory"); a pod's process is not visible from the PC.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

/// Samples one process's resident memory.
pub struct Probe {
    sys: System,
    pid: Pid,
}

impl Probe {
    /// A probe on process `pid`.
    pub fn new(pid: u32) -> Self {
        Self {
            sys: System::new(),
            pid: Pid::from_u32(pid),
        }
    }

    /// The resident set now, in bytes; `None` once the process is gone.
    pub fn resident(&mut self) -> Option<u64> {
        self.sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[self.pid]),
            true,
            ProcessRefreshKind::nothing().with_memory(),
        );
        self.sys.process(self.pid).map(sysinfo::Process::memory)
    }
}

/// A background sampler: the highest resident set seen since it started.
pub struct Peak {
    highest: Arc<AtomicU64>,
    stop: StopOnDrop,
    task: tokio::task::JoinHandle<Probe>,
}

/// Sets the sampler's stop flag when dropped: a run that ends early (an error, a panic) still stops the sampler,
/// or the runtime would wait for its blocking loop forever at shutdown.
struct StopOnDrop(Arc<AtomicBool>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

impl Peak {
    /// Start sampling `probe` every `every` on the blocking pool (a sysinfo refresh is a syscall, not async work).
    pub fn start(mut probe: Probe, every: Duration) -> Self {
        let highest = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (p, s) = (highest.clone(), stop.clone());
        let task = tokio::task::spawn_blocking(move || {
            while !s.load(Ordering::Relaxed) {
                if let Some(b) = probe.resident() {
                    p.fetch_max(b, Ordering::Relaxed);
                }
                std::thread::sleep(every);
            }
            probe
        });
        Self {
            highest,
            stop: StopOnDrop(stop),
            task,
        }
    }

    /// Stop sampling; the peak in bytes (0 if no sample landed) and the probe back.
    pub async fn finish(self) -> Result<(u64, Probe), tokio::task::JoinError> {
        let Self { highest, stop, task } = self;
        drop(stop);
        let probe = task.await?;
        Ok((highest.load(Ordering::Relaxed), probe))
    }
}
