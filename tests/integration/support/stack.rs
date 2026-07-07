//! 16MB-stack test runner for turns whose capability path nests enough async
//! state machines (live subprocess capture, capability-retry re-dispatch) to
//! overflow the default `#[tokio::test]` thread stack in a debug build.
//! Mirrors `tests/reborn_qa_smoke_scenarios_e2e.rs::run_async_test_with_stack`.

// Not every test binary that mounts this support tree needs the larger stack —
// mirrors the `#![allow(dead_code)]` used in sibling modules.
#![allow(dead_code)]

/// Runs `test` to completion on a dedicated 16MB-stack thread with a
/// current-thread tokio runtime, re-panicking any test failure.
pub(crate) fn run_with_larger_stack<F>(name: &str, test: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let handle = std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio test runtime")
                .block_on(test);
        })
        .expect("spawn stack-sized test thread");
    if let Err(panic) = handle.join() {
        std::panic::resume_unwind(panic);
    }
}
