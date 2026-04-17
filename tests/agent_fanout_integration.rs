//! Integration coverage for the per-thread fanout layer.
//!
//! These tests drive `ThreadFanout` directly with a `FanoutHandler` that
//! simulates `Agent::process_one` timing. They are the behavioral contract
//! the plan's "R1/R2 regression tests" describe:
//!
//! - R1 (no head-of-line blocking across threads): 5 messages across 5
//!   different threads, each synthetic handler sleeps 200 ms → total wall
//!   time must be < 600 ms (well under 5 × 200 ms).
//! - R2 (strict in-order processing per thread): 5 messages in the same
//!   thread, each sleeping 200 ms → total ≥ 5 × 200 ms and responses arrive
//!   in insertion order.
//! - R3 (Session lock discipline): cross-thread messages from the same user
//!   do NOT deadlock even when the handler takes a shared per-user lock.
//!
//! The legacy (flag-off) rollback path is covered by the existing test
//! suite — `Agent::process_one` is the single shared entry point and the
//! config flag only toggles whether the outer loop wraps dispatches via
//! the fanout or calls `process_one` directly.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use ironclaw::agent::thread_fanout::{DispatchError, FanoutConfig, FanoutHandler, ThreadFanout};
use ironclaw::channels::IncomingMessage;
use tokio::sync::Mutex;
use uuid::Uuid;

/// Simulates `Agent::process_one`: sleeps for a fixed duration, then appends
/// the message content plus the bucket key to a shared capture log.
struct SleepyHandler {
    sleep: Duration,
    log: Arc<Mutex<Vec<(String, String)>>>,
}

#[async_trait]
impl FanoutHandler for SleepyHandler {
    async fn handle(&self, msg: IncomingMessage) {
        tokio::time::sleep(self.sleep).await;
        let key = ThreadFanout::bucket_key(&msg);
        self.log.lock().await.push((key, msg.content.clone()));
    }
}

/// Simulates a handler that acquires a per-user mutex before processing —
/// mirrors the `Session` lock behavior. Two concurrent threads from the
/// same user would serialize here but must not deadlock.
struct UserLockedHandler {
    sleep: Duration,
    user_lock: Arc<Mutex<usize>>,
    concurrent_peak: Arc<AtomicUsize>,
    current: Arc<AtomicUsize>,
    done_count: Arc<AtomicUsize>,
}

#[async_trait]
impl FanoutHandler for UserLockedHandler {
    async fn handle(&self, _msg: IncomingMessage) {
        // Observe how many handlers are running simultaneously before we
        // take the "session" lock. This proves cross-thread concurrency is
        // real (R1).
        let now = self.current.fetch_add(1, Ordering::SeqCst) + 1;
        self.concurrent_peak.fetch_max(now, Ordering::SeqCst);

        let mut g = self.user_lock.lock().await;
        *g += 1;
        tokio::time::sleep(self.sleep).await;
        drop(g);

        self.current.fetch_sub(1, Ordering::SeqCst);
        self.done_count.fetch_add(1, Ordering::SeqCst);
    }
}

fn msg(channel: &str, user: &str, thread: Option<&str>, content: &str) -> IncomingMessage {
    let mut m = IncomingMessage::new(channel, user, content);
    m.id = Uuid::new_v4();
    if let Some(t) = thread {
        m = m.with_thread(t);
    }
    m
}

#[tokio::test]
async fn r1_no_head_of_line_blocking_across_threads() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let handler = Arc::new(SleepyHandler {
        sleep: Duration::from_millis(200),
        log: Arc::clone(&log),
    });
    let fanout = ThreadFanout::new(FanoutConfig::default(), handler);

    let start = Instant::now();
    for i in 0..5 {
        fanout
            .dispatch(msg(
                "dingtalk",
                "alice",
                Some(&format!("cid-{i}:alice")),
                &format!("msg-{i}"),
            ))
            .await
            .expect("dispatch must succeed");
    }

    // Wait for all to drain.
    for _ in 0..40 {
        if log.lock().await.len() == 5 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let elapsed = start.elapsed();
    let got = log.lock().await.clone();
    assert_eq!(got.len(), 5, "all 5 messages must complete");
    assert!(
        elapsed < Duration::from_millis(600),
        "R1 concurrency: 5 threads × 200 ms must complete under 600 ms, got {elapsed:?}"
    );

    // Verify 5 distinct bucket keys fired in parallel.
    let mut keys: Vec<String> = got.iter().map(|(k, _)| k.clone()).collect();
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), 5, "5 distinct buckets observed");
}

#[tokio::test]
async fn r2_strict_in_order_processing_per_thread() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let handler = Arc::new(SleepyHandler {
        sleep: Duration::from_millis(200),
        log: Arc::clone(&log),
    });
    let fanout = ThreadFanout::new(FanoutConfig::default(), handler);

    let start = Instant::now();
    for i in 0..5 {
        fanout
            .dispatch(msg("dingtalk", "alice", Some("cid-same"), &format!("m{i}")))
            .await
            .expect("dispatch must succeed");
    }

    for _ in 0..80 {
        if log.lock().await.len() == 5 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let elapsed = start.elapsed();
    let got = log.lock().await.clone();
    assert_eq!(got.len(), 5);
    assert!(
        elapsed >= Duration::from_millis(1000),
        "R2 ordering: same-thread must serialize — expected ≥ 1 s, got {elapsed:?}"
    );
    let contents: Vec<_> = got.iter().map(|(_, c)| c.clone()).collect();
    assert_eq!(
        contents,
        vec!["m0", "m1", "m2", "m3", "m4"],
        "strict insertion order within bucket"
    );
}

#[tokio::test]
async fn error_in_one_bucket_does_not_affect_others() {
    // Handler that errors for messages with content "boom" and succeeds for
    // others. We model "error" as a panic-free miss: handler records a
    // tagged entry instead of completing the response.
    struct PartialFailureHandler {
        log: Arc<Mutex<Vec<String>>>,
    }
    #[async_trait]
    impl FanoutHandler for PartialFailureHandler {
        async fn handle(&self, msg: IncomingMessage) {
            if msg.content == "boom" {
                self.log.lock().await.push("boom-handled".into());
            } else {
                self.log.lock().await.push(format!("ok:{}", msg.content));
            }
        }
    }

    let log = Arc::new(Mutex::new(Vec::new()));
    let handler = Arc::new(PartialFailureHandler {
        log: Arc::clone(&log),
    });
    let fanout = ThreadFanout::new(FanoutConfig::default(), handler);

    // One "boom" in bucket-A, two oks in bucket-B.
    fanout
        .dispatch(msg("web", "u-1", Some("A"), "boom"))
        .await
        .unwrap();
    fanout
        .dispatch(msg("web", "u-1", Some("B"), "x"))
        .await
        .unwrap();
    fanout
        .dispatch(msg("web", "u-1", Some("B"), "y"))
        .await
        .unwrap();

    for _ in 0..20 {
        if log.lock().await.len() == 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let got = log.lock().await.clone();
    assert!(got.contains(&"boom-handled".to_string()));
    assert!(got.contains(&"ok:x".to_string()));
    assert!(got.contains(&"ok:y".to_string()));
}

#[tokio::test]
async fn backpressure_busy_signal_path() {
    // Tiny bucket queue capacity + slow handler forces backpressure.
    let handler = Arc::new(SleepyHandler {
        sleep: Duration::from_millis(400),
        log: Arc::new(Mutex::new(Vec::new())),
    });
    let fanout = ThreadFanout::new(
        FanoutConfig {
            bucket_queue_capacity: 1,
            ..FanoutConfig::default()
        },
        handler,
    );

    fanout
        .dispatch(msg("web", "u-1", Some("t-1"), "seed"))
        .await
        .expect("seed accepts");

    let mut observed_full = false;
    for i in 2..20 {
        match fanout
            .dispatch(msg("web", "u-1", Some("t-1"), &format!("{i}")))
            .await
        {
            Err(DispatchError::BucketFull(key)) => {
                assert_eq!(key, "web:t-1");
                observed_full = true;
                break;
            }
            _ => continue,
        }
    }
    assert!(observed_full, "backpressure must surface as BucketFull");
    assert!(
        fanout.stats().drops_bucket_full.load(Ordering::Relaxed) >= 1,
        "stats must record the drop"
    );
}

#[tokio::test]
async fn cross_thread_same_user_does_not_deadlock() {
    // Two threads for the same user, handler takes a shared per-user lock.
    let user_lock = Arc::new(Mutex::new(0usize));
    let concurrent_peak = Arc::new(AtomicUsize::new(0));
    let current = Arc::new(AtomicUsize::new(0));
    let done = Arc::new(AtomicUsize::new(0));
    let handler = Arc::new(UserLockedHandler {
        sleep: Duration::from_millis(100),
        user_lock: Arc::clone(&user_lock),
        concurrent_peak: Arc::clone(&concurrent_peak),
        current: Arc::clone(&current),
        done_count: Arc::clone(&done),
    });
    let fanout = ThreadFanout::new(FanoutConfig::default(), handler);

    for i in 0..4 {
        fanout
            .dispatch(msg(
                "dingtalk",
                "alice",
                Some(&format!("t-{i}")),
                &format!("m{i}"),
            ))
            .await
            .unwrap();
    }

    for _ in 0..80 {
        if done.load(Ordering::SeqCst) == 4 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(
        done.load(Ordering::SeqCst),
        4,
        "all 4 must finish without deadlock"
    );
    // Concurrent peak > 1 proves cross-thread parallelism was real.
    assert!(
        concurrent_peak.load(Ordering::SeqCst) >= 2,
        "cross-thread parallelism expected, peak was {}",
        concurrent_peak.load(Ordering::SeqCst)
    );
}

#[tokio::test]
async fn shutdown_aborts_laggards_past_grace() {
    // Handler sleeps longer than the shutdown grace window — forces the
    // timeout branch to fire.
    let handler = Arc::new(SleepyHandler {
        sleep: Duration::from_secs(30),
        log: Arc::new(Mutex::new(Vec::new())),
    });
    let fanout = ThreadFanout::new(FanoutConfig::default(), handler);

    fanout
        .dispatch(msg("web", "u-1", Some("t-1"), "stuck"))
        .await
        .unwrap();
    // Let the handler start so it's in-flight when shutdown fires.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let start = Instant::now();
    fanout.shutdown(Duration::from_millis(200)).await;
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_millis(600),
        "shutdown must return near the grace window, got {elapsed:?}"
    );
    assert_eq!(fanout.active_buckets().await, 0);
}

#[tokio::test]
async fn shutdown_idempotent_second_call_noop() {
    let handler = Arc::new(SleepyHandler {
        sleep: Duration::from_millis(10),
        log: Arc::new(Mutex::new(Vec::new())),
    });
    let fanout = ThreadFanout::new(FanoutConfig::default(), handler);

    fanout.shutdown(Duration::from_millis(100)).await;
    // Second call must return promptly and not panic.
    let start = Instant::now();
    fanout.shutdown(Duration::from_millis(100)).await;
    assert!(start.elapsed() < Duration::from_millis(20));
}

#[tokio::test]
async fn shutdown_drains_inflight_buckets() {
    let handler = Arc::new(SleepyHandler {
        sleep: Duration::from_millis(50),
        log: Arc::new(Mutex::new(Vec::new())),
    });
    let fanout = ThreadFanout::new(FanoutConfig::default(), handler);

    for i in 0..3 {
        fanout
            .dispatch(msg("web", "u-1", Some(&format!("t-{i}")), "hi"))
            .await
            .unwrap();
    }

    fanout.shutdown(Duration::from_secs(2)).await;
    assert_eq!(fanout.active_buckets().await, 0);

    let err = fanout
        .dispatch(msg("web", "u-1", Some("late"), "too late"))
        .await
        .unwrap_err();
    assert!(matches!(err, DispatchError::ShuttingDown));
}
