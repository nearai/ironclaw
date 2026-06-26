use std::{
    any::Any,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Default)]
pub(crate) struct ProgressCounters {
    attempted: AtomicU64,
    failed: AtomicU64,
}

impl ProgressCounters {
    pub(crate) fn record(&self, failed: bool) {
        if failed {
            self.failed.fetch_add(1, Ordering::Relaxed);
        }
        self.attempted.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) struct ProgressReporter {
    stop_sender: mpsc::Sender<()>,
    handle: JoinHandle<()>,
}

pub(crate) fn spawn_progress_reporter(
    prefix: String,
    backend: &'static str,
    scenario: &'static str,
    progress_interval_seconds: u64,
    total_operations: usize,
    progress: Arc<ProgressCounters>,
) -> Option<ProgressReporter> {
    if progress_interval_seconds == 0 || total_operations == 0 {
        return None;
    }

    let interval = Duration::from_secs(progress_interval_seconds);
    let total_operations = total_operations as u64;
    let (stop_sender, stop_receiver) = mpsc::channel();
    let handle = thread::Builder::new()
        .name("storage-stress-progress".to_string())
        .spawn(move || {
            let started = Instant::now();
            let mut last_attempted = 0;
            let mut last_report = Instant::now();
            loop {
                match stop_receiver.recv_timeout(interval) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        let attempted = progress.attempted.load(Ordering::Relaxed);
                        let failed = progress.failed.load(Ordering::Relaxed);
                        let succeeded = attempted.saturating_sub(failed);
                        let recent_attempted = attempted.saturating_sub(last_attempted);
                        let recent_elapsed = last_report.elapsed().as_secs_f64();
                        let recent_ops_sec =
                            recent_attempted as f64 / recent_elapsed.max(f64::MIN_POSITIVE);
                        eprintln!(
                            "{prefix} progress backend={backend} scenario={scenario} attempted={attempted}/{total_operations} succeeded={succeeded} failed={failed} elapsed_ms={} recent_ops_sec={recent_ops_sec:.1}",
                            started.elapsed().as_millis()
                        );
                        last_attempted = attempted;
                        last_report = Instant::now();
                    }
                }
            }
        })
        .ok()?;

    Some(ProgressReporter {
        stop_sender,
        handle,
    })
}

pub(crate) fn stop_progress_reporter(progress_reporter: Option<ProgressReporter>) {
    if let Some(progress_reporter) = progress_reporter {
        let _ = progress_reporter.stop_sender.send(());
        if let Err(payload) = progress_reporter.handle.join() {
            eprintln!(
                "progress reporter panicked: {}",
                panic_payload_to_string(&payload)
            );
        }
    }
}

fn panic_payload_to_string(payload: &Box<dyn Any + Send + 'static>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    "non-string panic payload".to_string()
}
