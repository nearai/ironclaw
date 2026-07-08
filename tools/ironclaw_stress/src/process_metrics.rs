use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub(crate) struct ProcessSnapshot {
    pub(crate) rss_kb: Option<u64>,
    pub(crate) peak_rss_kb: Option<u64>,
    pub(crate) user_cpu_ms: Option<u128>,
    pub(crate) system_cpu_ms: Option<u128>,
    pub(crate) threads: Option<u64>,
    pub(crate) open_fds: Option<u64>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub(crate) struct ProcessMetrics {
    pub(crate) start: ProcessSnapshot,
    pub(crate) end: ProcessSnapshot,
    pub(crate) delta_user_cpu_ms: Option<u128>,
    pub(crate) delta_system_cpu_ms: Option<u128>,
    pub(crate) delta_cpu_ms: Option<u128>,
    pub(crate) peak_rss_kb: Option<u64>,
    pub(crate) peak_threads: Option<u64>,
    pub(crate) peak_open_fds: Option<u64>,
}

pub(crate) fn aggregate_process_metrics<'a>(
    metrics: impl IntoIterator<Item = &'a ProcessMetrics>,
) -> ProcessMetrics {
    let mut aggregate = ProcessMetrics::default();
    for metrics in metrics {
        aggregate.delta_user_cpu_ms =
            add_option(aggregate.delta_user_cpu_ms, metrics.delta_user_cpu_ms);
        aggregate.delta_system_cpu_ms =
            add_option(aggregate.delta_system_cpu_ms, metrics.delta_system_cpu_ms);
        aggregate.delta_cpu_ms = add_option(aggregate.delta_cpu_ms, metrics.delta_cpu_ms);
        aggregate.peak_rss_kb = max_option(aggregate.peak_rss_kb, metrics.peak_rss_kb);
        aggregate.peak_threads = max_option(aggregate.peak_threads, metrics.peak_threads);
        aggregate.peak_open_fds = max_option(aggregate.peak_open_fds, metrics.peak_open_fds);
    }
    aggregate
}

#[derive(Debug, Clone, Copy, Default)]
struct Peaks {
    rss_kb: Option<u64>,
    threads: Option<u64>,
    open_fds: Option<u64>,
}

pub(crate) struct ProcessMetricsSampler {
    start: ProcessSnapshot,
    peaks: Arc<Mutex<Peaks>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl ProcessMetricsSampler {
    pub(crate) fn start(interval: Duration) -> Self {
        let start = capture_snapshot();
        let peaks = Arc::new(Mutex::new(Peaks::from_snapshot(start)));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_peaks = Arc::clone(&peaks);
        let thread_stop = Arc::clone(&stop);
        let handle = thread::Builder::new()
            .name("ironclaw-stress-process-metrics".to_string())
            .spawn(move || {
                while !thread_stop.load(Ordering::Relaxed) {
                    thread::sleep(interval);
                    if let Ok(mut peaks) = thread_peaks.lock() {
                        peaks.record(capture_snapshot());
                    }
                }
            })
            .ok();
        Self {
            start,
            peaks,
            stop,
            handle,
        }
    }

    pub(crate) fn finish(mut self) -> ProcessMetrics {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        let end = capture_snapshot();
        let peaks = match self.peaks.lock() {
            Ok(mut peaks) => {
                peaks.record(end);
                *peaks
            }
            Err(_) => Peaks::from_snapshot(end),
        };
        let delta_user_cpu_ms = subtract_option(end.user_cpu_ms, self.start.user_cpu_ms);
        let delta_system_cpu_ms = subtract_option(end.system_cpu_ms, self.start.system_cpu_ms);
        let delta_cpu_ms = match (delta_user_cpu_ms, delta_system_cpu_ms) {
            (Some(user), Some(system)) => Some(user.saturating_add(system)),
            _ => None,
        };
        ProcessMetrics {
            start: self.start,
            end,
            delta_user_cpu_ms,
            delta_system_cpu_ms,
            delta_cpu_ms,
            peak_rss_kb: max_option(peaks.rss_kb, end.peak_rss_kb),
            peak_threads: peaks.threads,
            peak_open_fds: peaks.open_fds,
        }
    }
}

impl Peaks {
    fn from_snapshot(snapshot: ProcessSnapshot) -> Self {
        Self {
            rss_kb: max_option(snapshot.rss_kb, snapshot.peak_rss_kb),
            threads: snapshot.threads,
            open_fds: snapshot.open_fds,
        }
    }

    fn record(&mut self, snapshot: ProcessSnapshot) {
        self.rss_kb = max_option(
            self.rss_kb,
            max_option(snapshot.rss_kb, snapshot.peak_rss_kb),
        );
        self.threads = max_option(self.threads, snapshot.threads);
        self.open_fds = max_option(self.open_fds, snapshot.open_fds);
    }
}

pub(crate) fn capture_snapshot() -> ProcessSnapshot {
    let proc_status = proc_status_snapshot();
    let rusage = rusage_snapshot();
    ProcessSnapshot {
        rss_kb: proc_status
            .and_then(|snapshot| snapshot.rss_kb)
            .or_else(current_rss_kb_fallback),
        peak_rss_kb: rusage.and_then(|snapshot| snapshot.peak_rss_kb),
        user_cpu_ms: rusage.and_then(|snapshot| snapshot.user_cpu_ms),
        system_cpu_ms: rusage.and_then(|snapshot| snapshot.system_cpu_ms),
        threads: proc_status
            .and_then(|snapshot| snapshot.threads)
            .or_else(thread_count_fallback),
        open_fds: open_fd_count(),
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct ProcStatusSnapshot {
    rss_kb: Option<u64>,
    threads: Option<u64>,
}

#[cfg(target_os = "linux")]
fn proc_status_snapshot() -> Option<ProcStatusSnapshot> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let mut snapshot = ProcStatusSnapshot::default();
    for line in status.lines() {
        if let Some(value) = line.strip_prefix("VmRSS:") {
            snapshot.rss_kb = value.split_whitespace().next()?.parse().ok();
        } else if let Some(value) = line.strip_prefix("Threads:") {
            snapshot.threads = value.trim().parse().ok();
        }
    }
    Some(snapshot)
}

#[cfg(not(target_os = "linux"))]
fn proc_status_snapshot() -> Option<ProcStatusSnapshot> {
    None
}

#[derive(Debug, Clone, Copy, Default)]
struct RusageSnapshot {
    peak_rss_kb: Option<u64>,
    user_cpu_ms: Option<u128>,
    system_cpu_ms: Option<u128>,
}

#[cfg(unix)]
fn rusage_snapshot() -> Option<RusageSnapshot> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: getrusage initializes the provided rusage pointer when it returns 0.
    let status = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if status != 0 {
        return None;
    }
    // SAFETY: getrusage returned 0, so usage has been initialized by libc.
    let usage = unsafe { usage.assume_init() };
    Some(RusageSnapshot {
        peak_rss_kb: Some(max_rss_to_kb(usage.ru_maxrss)),
        user_cpu_ms: Some(timeval_to_ms(usage.ru_utime)),
        system_cpu_ms: Some(timeval_to_ms(usage.ru_stime)),
    })
}

#[cfg(not(unix))]
fn rusage_snapshot() -> Option<RusageSnapshot> {
    None
}

#[cfg(target_os = "macos")]
fn max_rss_to_kb(max_rss: libc::c_long) -> u64 {
    u64::try_from(max_rss).unwrap_or(0) / 1024
}

#[cfg(all(unix, not(target_os = "macos")))]
fn max_rss_to_kb(max_rss: libc::c_long) -> u64 {
    u64::try_from(max_rss).unwrap_or(0)
}

#[cfg(unix)]
fn timeval_to_ms(time: libc::timeval) -> u128 {
    let seconds = u128::try_from(time.tv_sec).unwrap_or(0);
    let micros = u128::try_from(time.tv_usec).unwrap_or(0);
    seconds.saturating_mul(1_000).saturating_add(micros / 1_000)
}

#[cfg(target_os = "macos")]
fn current_rss_kb_fallback() -> Option<u64> {
    ps_value("rss")
}

#[cfg(not(target_os = "macos"))]
fn current_rss_kb_fallback() -> Option<u64> {
    None
}

#[cfg(target_os = "macos")]
fn thread_count_fallback() -> Option<u64> {
    ps_value("thcount")
}

#[cfg(not(target_os = "macos"))]
fn thread_count_fallback() -> Option<u64> {
    None
}

#[cfg(target_os = "macos")]
fn ps_value(column: &str) -> Option<u64> {
    let pid = std::process::id().to_string();
    let output = std::process::Command::new("ps")
        .arg("-o")
        .arg(format!("{column}="))
        .arg("-p")
        .arg(pid)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()?.trim().parse().ok()
}

#[cfg(target_os = "linux")]
fn open_fd_count() -> Option<u64> {
    u64::try_from(std::fs::read_dir("/proc/self/fd").ok()?.count()).ok()
}

#[cfg(all(unix, not(target_os = "linux")))]
fn open_fd_count() -> Option<u64> {
    u64::try_from(std::fs::read_dir("/dev/fd").ok()?.count()).ok()
}

#[cfg(not(unix))]
fn open_fd_count() -> Option<u64> {
    None
}

fn subtract_option(end: Option<u128>, start: Option<u128>) -> Option<u128> {
    Some(end?.saturating_sub(start?))
}

fn add_option<T>(left: Option<T>, right: Option<T>) -> Option<T>
where
    T: std::ops::Add<Output = T>,
{
    match (left, right) {
        (Some(left), Some(right)) => Some(left + right),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn max_option<T: Ord>(left: Option<T>, right: Option<T>) -> Option<T> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}
