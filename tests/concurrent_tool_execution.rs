//! Integration tests for concurrent tool execution batch partitioning.
//!
//! These tests verify:
//! 1. Built-in tool concurrency classifications are correct
//! 2. Batch partitioning produces correct execution order
//! 3. Tool results map back to correct tool_call_ids after concurrent execution
//! 4. JoinSet panic recovery fills error slots correctly
//! 5. Rate limiter behavior under concurrency

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Barrier;

use ironclaw::agent::batch::{ToolBatch, partition_tool_calls};
use ironclaw::context::JobContext;
use ironclaw::llm::ToolCall;
use ironclaw::tools::{Tool, ToolError, ToolOutput, ToolRateLimitConfig};

// ---------------------------------------------------------------------------
// Test tool fixtures
// ---------------------------------------------------------------------------

/// A concurrent-safe tool that records execution via an atomic counter.
#[derive(Debug)]
struct TimestampedReadTool {
    name: String,
    execution_order: Arc<AtomicUsize>,
    delay: Duration,
}

impl TimestampedReadTool {
    fn new(name: &str, order_tracker: Arc<AtomicUsize>, delay: Duration) -> Self {
        Self {
            name: name.to_string(),
            execution_order: order_tracker,
            delay,
        }
    }
}

#[async_trait]
impl Tool for TimestampedReadTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        "Test read tool"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {}})
    }
    async fn execute(
        &self,
        _params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let order = self.execution_order.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        Ok(ToolOutput::text(format!("read_result_{order}"), self.delay))
    }
    fn is_concurrent_safe(&self, _params: &serde_json::Value) -> bool {
        true
    }
}

/// A concurrent-safe tool that waits on a shared barrier (for structural concurrency tests).
#[derive(Debug)]
struct BarrierReadTool {
    name: String,
    barrier: Arc<Barrier>,
}

#[async_trait]
impl Tool for BarrierReadTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        "Barrier-based read tool"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {}})
    }
    async fn execute(
        &self,
        _params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        // All tools must reach the barrier before any can proceed.
        // If executed serially, this would deadlock (timeout).
        self.barrier.wait().await;
        Ok(ToolOutput::text("barrier_passed", Duration::ZERO))
    }
    fn is_concurrent_safe(&self, _params: &serde_json::Value) -> bool {
        true
    }
}

/// A mutating tool that records execution order.
#[derive(Debug)]
struct TimestampedWriteTool {
    name: String,
    execution_order: Arc<AtomicUsize>,
    delay: Duration,
}

impl TimestampedWriteTool {
    fn new(name: &str, order_tracker: Arc<AtomicUsize>, delay: Duration) -> Self {
        Self {
            name: name.to_string(),
            execution_order: order_tracker,
            delay,
        }
    }
}

#[async_trait]
impl Tool for TimestampedWriteTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        "Test write tool"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {}})
    }
    async fn execute(
        &self,
        _params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let order = self.execution_order.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        Ok(ToolOutput::text(
            format!("write_result_{order}"),
            self.delay,
        ))
    }
    fn is_concurrent_safe(&self, _params: &serde_json::Value) -> bool {
        false
    }
    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(20, 200))
    }
}

/// A tool that panics during execution (for panic-recovery tests).
#[derive(Debug)]
struct PanickingTool;

#[async_trait]
impl Tool for PanickingTool {
    fn name(&self) -> &str {
        "panicking_tool"
    }
    fn description(&self) -> &str {
        "Always panics"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {}})
    }
    async fn execute(
        &self,
        _params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        panic!("intentional test panic");
    }
    fn is_concurrent_safe(&self, _params: &serde_json::Value) -> bool {
        true
    }
}

fn tc(name: &str, idx: usize) -> ToolCall {
    ToolCall {
        id: format!("call_{idx}"),
        name: name.to_string(),
        arguments: serde_json::json!({}),
        reasoning: None,
    }
}

// ---------------------------------------------------------------------------
// Batch partitioning with realistic tool names
// ---------------------------------------------------------------------------

#[test]
fn realistic_multi_tool_turn_partitions_correctly() {
    let classified = vec![
        (0, tc("read_file", 0), true),
        (1, tc("glob", 1), true),
        (2, tc("grep", 2), true),
        (3, tc("write_file", 3), false),
        (4, tc("read_file", 4), true),
        (5, tc("memory_search", 5), true),
    ];
    let batches = partition_tool_calls(classified, 10);

    assert_eq!(batches.len(), 3);
    match &batches[0] {
        ToolBatch::Concurrent(items) => {
            assert_eq!(items.len(), 3);
            assert_eq!(items[0].1.name, "read_file");
            assert_eq!(items[1].1.name, "glob");
            assert_eq!(items[2].1.name, "grep");
        }
        _ => panic!("expected Concurrent batch"),
    }
    match &batches[1] {
        ToolBatch::Serial(_, tc) => assert_eq!(tc.name, "write_file"),
        _ => panic!("expected Serial batch"),
    }
    match &batches[2] {
        ToolBatch::Concurrent(items) => {
            assert_eq!(items.len(), 2);
            assert_eq!(items[0].1.name, "read_file");
            assert_eq!(items[1].1.name, "memory_search");
        }
        _ => panic!("expected Concurrent batch"),
    }
}

#[test]
fn shell_then_multiple_reads_partitions_correctly() {
    let classified = vec![
        (0, tc("shell", 0), false),
        (1, tc("read_file", 1), true),
        (2, tc("read_file", 2), true),
        (3, tc("read_file", 3), true),
    ];
    let batches = partition_tool_calls(classified, 10);

    assert_eq!(batches.len(), 2);
    assert!(matches!(&batches[0], ToolBatch::Serial(0, _)));
    match &batches[1] {
        ToolBatch::Concurrent(items) => assert_eq!(items.len(), 3),
        _ => panic!("expected Concurrent batch"),
    }
}

// ---------------------------------------------------------------------------
// Structural concurrency test (no timing assertions)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn concurrent_batch_executes_tools_in_parallel() {
    // 3 tools that each wait on a shared barrier. If run serially, this deadlocks.
    // If run concurrently, all 3 reach the barrier and proceed.
    let barrier = Arc::new(Barrier::new(3));
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(BarrierReadTool {
            name: "read_a".into(),
            barrier: barrier.clone(),
        }),
        Arc::new(BarrierReadTool {
            name: "read_b".into(),
            barrier: barrier.clone(),
        }),
        Arc::new(BarrierReadTool {
            name: "read_c".into(),
            barrier: barrier.clone(),
        }),
    ];

    let ctx = JobContext::default();
    let mut join_set = tokio::task::JoinSet::new();

    for tool in &tools {
        let tool = Arc::clone(tool);
        let ctx = ctx.clone();
        join_set.spawn(async move { tool.execute(serde_json::json!({}), &ctx).await });
    }

    // If tools run serially, the barrier never completes and this times out.
    let results: Vec<_> = tokio::time::timeout(Duration::from_secs(5), async {
        let mut out = Vec::new();
        while let Some(r) = join_set.join_next().await {
            out.push(r.expect("task should not panic"));
        }
        out
    })
    .await
    .expect("concurrent execution should not deadlock on barrier");

    assert_eq!(results.len(), 3);
    for r in &results {
        assert!(r.is_ok());
    }
}

#[tokio::test]
async fn serial_batch_executes_tools_sequentially() {
    let order = Arc::new(AtomicUsize::new(0));
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(TimestampedWriteTool::new(
            "write_a",
            order.clone(),
            Duration::from_millis(10),
        )),
        Arc::new(TimestampedWriteTool::new(
            "write_b",
            order.clone(),
            Duration::from_millis(10),
        )),
    ];

    let ctx = JobContext::default();
    let mut results = Vec::new();
    for tool in &tools {
        let r = tool.execute(serde_json::json!({}), &ctx).await;
        results.push(r);
    }

    assert_eq!(results.len(), 2);
    for r in &results {
        assert!(r.is_ok());
    }
    // Execution order is deterministic (serial)
    assert_eq!(
        results[0].as_ref().unwrap().result,
        serde_json::json!("write_result_0")
    );
    assert_eq!(
        results[1].as_ref().unwrap().result,
        serde_json::json!("write_result_1")
    );
}

// ---------------------------------------------------------------------------
// JoinSet result mapping (completion-order independence)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn mixed_batch_execution_preserves_tool_call_id_mapping() {
    // [read(40ms), read(10ms), write(10ms), read(10ms)]
    // read_b finishes before read_a, but results must map to correct pf_idx.
    let order = Arc::new(AtomicUsize::new(0));

    let read_a = Arc::new(TimestampedReadTool::new(
        "read_a",
        order.clone(),
        Duration::from_millis(40),
    ));
    let read_b = Arc::new(TimestampedReadTool::new(
        "read_b",
        order.clone(),
        Duration::from_millis(10),
    ));
    let write_c = Arc::new(TimestampedWriteTool::new(
        "write_c",
        order.clone(),
        Duration::from_millis(10),
    ));
    let read_d = Arc::new(TimestampedReadTool::new(
        "read_d",
        order.clone(),
        Duration::from_millis(10),
    ));

    let classified = vec![
        (
            0,
            tc("read_a", 0),
            read_a.is_concurrent_safe(&serde_json::json!({})),
        ),
        (
            1,
            tc("read_b", 1),
            read_b.is_concurrent_safe(&serde_json::json!({})),
        ),
        (
            2,
            tc("write_c", 2),
            write_c.is_concurrent_safe(&serde_json::json!({})),
        ),
        (
            3,
            tc("read_d", 3),
            read_d.is_concurrent_safe(&serde_json::json!({})),
        ),
    ];

    let batches = partition_tool_calls(classified, 10);
    assert_eq!(batches.len(), 3);

    let ctx = JobContext::default();
    let tools: Vec<Arc<dyn Tool>> = vec![read_a, read_b, write_c, read_d];
    let mut results: Vec<Option<Result<ToolOutput, ToolError>>> = (0..4).map(|_| None).collect();

    for batch in &batches {
        match batch {
            ToolBatch::Concurrent(items) => {
                // JoinSet returns results in completion order, not input order.
                let mut join_set = tokio::task::JoinSet::new();
                for (pf_idx, _tc) in items {
                    let pf_idx = *pf_idx;
                    let tool = Arc::clone(&tools[pf_idx]);
                    let ctx_clone = ctx.clone();
                    join_set.spawn(async move {
                        (
                            pf_idx,
                            tool.execute(serde_json::json!({}), &ctx_clone).await,
                        )
                    });
                }
                while let Some(join_result) = join_set.join_next().await {
                    let (pf_idx, result) = join_result.expect("task should not panic");
                    results[pf_idx] = Some(result);
                }
            }
            ToolBatch::Serial(pf_idx, _tc) => {
                let result = tools[*pf_idx].execute(serde_json::json!({}), &ctx).await;
                results[*pf_idx] = Some(result);
            }
        }
    }

    for (i, r) in results.iter().enumerate() {
        assert!(r.is_some(), "result at pf_idx {i} should be present");
        assert!(
            r.as_ref().unwrap().is_ok(),
            "result at pf_idx {i} should be Ok"
        );
    }

    let r0 = results[0].as_ref().unwrap().as_ref().unwrap();
    let r1 = results[1].as_ref().unwrap().as_ref().unwrap();
    let r2 = results[2].as_ref().unwrap().as_ref().unwrap();
    let r3 = results[3].as_ref().unwrap().as_ref().unwrap();

    assert!(r0.result.as_str().unwrap().starts_with("read_result_"));
    assert!(r1.result.as_str().unwrap().starts_with("read_result_"));
    assert!(r2.result.as_str().unwrap().starts_with("write_result_"));
    assert!(r3.result.as_str().unwrap().starts_with("read_result_"));
}

// ---------------------------------------------------------------------------
// JoinSet panic recovery
// ---------------------------------------------------------------------------

#[tokio::test]
async fn joinset_panic_recovery_fills_error_and_others_complete() {
    // Simulates the dispatcher's panic-recovery logic: if a tool panics inside
    // a JoinSet task, the other tools in the batch should still complete, and
    // the panicked slot should be filled with an error result.
    let order = Arc::new(AtomicUsize::new(0));

    let good_tool: Arc<dyn Tool> = Arc::new(TimestampedReadTool::new(
        "good_tool",
        order.clone(),
        Duration::from_millis(10),
    ));
    let panicking_tool: Arc<dyn Tool> = Arc::new(PanickingTool);
    let another_good: Arc<dyn Tool> = Arc::new(TimestampedReadTool::new(
        "another_good",
        order.clone(),
        Duration::from_millis(10),
    ));

    let tools: Vec<Arc<dyn Tool>> = vec![good_tool, panicking_tool, another_good];
    let ctx = JobContext::default();

    let mut results: Vec<Option<Result<ToolOutput, ToolError>>> = (0..3).map(|_| None).collect();
    let mut join_set = tokio::task::JoinSet::new();

    for (pf_idx, tool) in tools.iter().enumerate() {
        let tool = Arc::clone(tool);
        let ctx = ctx.clone();
        join_set.spawn(async move { (pf_idx, tool.execute(serde_json::json!({}), &ctx).await) });
    }

    while let Some(join_result) = join_set.join_next().await {
        match join_result {
            Ok((pf_idx, result)) => {
                results[pf_idx] = Some(result);
            }
            Err(e) => {
                // JoinError from panic — mirrors dispatcher behavior
                assert!(e.is_panic(), "expected panic, got cancellation");
            }
        }
    }

    // Fill panicked slots with error results (mirrors dispatcher.rs:1039-1062)
    for (pf_idx, slot) in results.iter_mut().enumerate() {
        if slot.is_none() {
            *slot = Some(Err(ToolError::ExecutionFailed(format!(
                "Task {} failed during execution",
                pf_idx,
            ))));
        }
    }

    // (a) All 3 slots should be filled
    assert!(results.iter().all(|r| r.is_some()));

    // (b) The good tools completed successfully
    assert!(
        results[0].as_ref().unwrap().is_ok(),
        "good_tool should succeed"
    );
    assert!(
        results[2].as_ref().unwrap().is_ok(),
        "another_good should succeed"
    );

    // (c) The panicking tool's slot has an error
    let panic_result = results[1].as_ref().unwrap();
    assert!(panic_result.is_err(), "panicking tool should have error");
    let err_msg = panic_result.as_ref().unwrap_err().to_string();
    assert!(
        err_msg.contains("failed during execution"),
        "error should indicate execution failure, got: {err_msg}"
    );
}

// ---------------------------------------------------------------------------
// max_concurrent=1 regression test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn max_concurrent_one_produces_ordered_results() {
    // With max_concurrent=1, every safe tool gets its own single-item Concurrent batch.
    // This is the regression path against the old sequential behavior.
    let order = Arc::new(AtomicUsize::new(0));

    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(TimestampedReadTool::new(
            "a",
            order.clone(),
            Duration::from_millis(5),
        )),
        Arc::new(TimestampedReadTool::new(
            "b",
            order.clone(),
            Duration::from_millis(5),
        )),
        Arc::new(TimestampedReadTool::new(
            "c",
            order.clone(),
            Duration::from_millis(5),
        )),
    ];

    let classified = vec![
        (0, tc("a", 0), true),
        (1, tc("b", 1), true),
        (2, tc("c", 2), true),
    ];
    let batches = partition_tool_calls(classified, 1);

    // Each tool in its own batch
    assert_eq!(batches.len(), 3);
    for batch in &batches {
        match batch {
            ToolBatch::Concurrent(items) => assert_eq!(items.len(), 1),
            _ => panic!("expected single-item Concurrent batches"),
        }
    }

    // Execute sequentially (each batch has 1 item)
    let ctx = JobContext::default();
    let mut results = Vec::new();
    for batch in &batches {
        if let ToolBatch::Concurrent(items) = batch {
            let (pf_idx, _) = &items[0];
            let r = tools[*pf_idx].execute(serde_json::json!({}), &ctx).await;
            results.push((*pf_idx, r));
        }
    }

    // Results should be in order
    assert_eq!(results.len(), 3);
    for (i, (pf_idx, r)) in results.iter().enumerate() {
        assert_eq!(*pf_idx, i);
        assert!(r.is_ok());
    }

    // Execution order should be deterministic (0, 1, 2)
    assert_eq!(
        results[0].1.as_ref().unwrap().result,
        serde_json::json!("read_result_0")
    );
    assert_eq!(
        results[1].1.as_ref().unwrap().result,
        serde_json::json!("read_result_1")
    );
    assert_eq!(
        results[2].1.as_ref().unwrap().result,
        serde_json::json!("read_result_2")
    );
}

// ---------------------------------------------------------------------------
// Built-in tool classification audit
// ---------------------------------------------------------------------------

#[test]
fn builtin_echo_is_concurrent_safe() {
    use ironclaw::tools::builtin::EchoTool;
    let tool = EchoTool;
    assert!(
        tool.is_concurrent_safe(&serde_json::json!({})),
        "echo is a pure function — must be concurrent-safe"
    );
}

#[test]
fn builtin_http_get_is_concurrent_safe() {
    use ironclaw::tools::builtin::HttpTool;
    let tool = HttpTool::new();
    assert!(tool.is_concurrent_safe(&serde_json::json!({"method": "GET"})));
}

#[test]
fn builtin_http_head_is_concurrent_safe() {
    use ironclaw::tools::builtin::HttpTool;
    let tool = HttpTool::new();
    assert!(
        tool.is_concurrent_safe(&serde_json::json!({"method": "HEAD"})),
        "HEAD is idempotent and read-only"
    );
}

#[test]
fn builtin_http_options_is_concurrent_safe() {
    use ironclaw::tools::builtin::HttpTool;
    let tool = HttpTool::new();
    assert!(
        tool.is_concurrent_safe(&serde_json::json!({"method": "OPTIONS"})),
        "OPTIONS is idempotent and read-only"
    );
}

#[test]
fn builtin_http_get_with_save_to_is_not_concurrent_safe() {
    use ironclaw::tools::builtin::HttpTool;
    let tool = HttpTool::new();
    assert!(
        !tool.is_concurrent_safe(&serde_json::json!({
            "url": "https://example.com/image.png",
            "method": "GET",
            "save_to": "/tmp/image.png"
        })),
        "GET with save_to writes to disk — not concurrent-safe"
    );
}

#[test]
fn builtin_http_post_is_not_concurrent_safe() {
    use ironclaw::tools::builtin::HttpTool;
    let tool = HttpTool::new();
    assert!(!tool.is_concurrent_safe(&serde_json::json!({"method": "POST"})));
}

#[test]
fn builtin_http_no_method_defaults_to_concurrent_safe() {
    use ironclaw::tools::builtin::HttpTool;
    let tool = HttpTool::new();
    assert!(tool.is_concurrent_safe(&serde_json::json!({"url": "https://example.com"})));
}

// ---------------------------------------------------------------------------
// Rate limiter
// ---------------------------------------------------------------------------

/// Pure concurrent-safe tools like echo have no rate limit config, which lets
/// the dispatcher skip the rate limiter lock. Note: parameter-dependent tools
/// like http may be concurrent-safe for GET yet still have a rate limit.
#[test]
fn pure_concurrent_safe_tool_has_no_rate_limit() {
    use ironclaw::tools::builtin::EchoTool;
    let tool = EchoTool;
    assert!(tool.is_concurrent_safe(&serde_json::json!({})));
    assert!(
        tool.rate_limit_config().is_none(),
        "echo should have no rate limit (enables lock skip)"
    );
}

#[tokio::test]
async fn rate_limiter_consulted_for_tools_with_config() {
    use ironclaw::tools::rate_limiter::RateLimiter;

    let limiter = RateLimiter::new();
    let config = ToolRateLimitConfig::new(30, 500);

    let result = limiter
        .check_and_record("test_user", "shell", &config)
        .await;
    assert!(result.is_allowed());

    let usage = limiter.get_usage("test_user", "shell").await;
    assert_eq!(usage, Some((1, 1)));
}

#[tokio::test]
async fn concurrent_rate_limited_tools_dont_exceed_limit() {
    use ironclaw::tools::rate_limiter::RateLimiter;

    let limiter = Arc::new(RateLimiter::new());
    let config = ToolRateLimitConfig::new(5, 100);

    // 10 concurrent calls, limit is 5 per minute.
    let mut handles = Vec::new();
    for _ in 0..10 {
        let limiter = limiter.clone();
        let config = config.clone();
        handles.push(tokio::spawn(async move {
            limiter
                .check_and_record("user1", "write_file", &config)
                .await
        }));
    }

    let results: Vec<_> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    let allowed = results.iter().filter(|r| r.is_allowed()).count();
    let limited = results.iter().filter(|r| !r.is_allowed()).count();

    // The write lock serializes access, making the count deterministic.
    assert_eq!(allowed, 5, "exactly 5 should be allowed");
    assert_eq!(limited, 5, "exactly 5 should be rate-limited");
}
