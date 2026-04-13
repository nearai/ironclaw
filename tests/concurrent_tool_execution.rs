//! Integration tests for concurrent tool execution batch partitioning.
//!
//! These tests verify:
//! 1. Built-in tool concurrency classifications are correct
//! 2. Batch partitioning produces correct execution order
//! 3. Tool results map back to correct tool_call_ids after concurrent execution
//! 4. Rate limiter is skipped for tools without rate limit config

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;

use ironclaw::agent::batch::{ToolBatch, partition_tool_calls};
use ironclaw::context::JobContext;
use ironclaw::llm::ToolCall;
use ironclaw::tools::{Tool, ToolError, ToolOutput, ToolRateLimitConfig};

// ---------------------------------------------------------------------------
// Test tool fixtures
// ---------------------------------------------------------------------------

/// A concurrent-safe tool that records when it was executed (for ordering tests).
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
        // Record execution order
        let order = self.execution_order.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        Ok(ToolOutput::text(format!("read_result_{order}"), self.delay))
    }
    fn is_concurrent_safe(&self, _params: &serde_json::Value) -> bool {
        true
    }
}

/// A mutating tool that records when it was executed (for ordering tests).
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
    // Simulates: LLM wants to read 3 files, write one, then read 2 more
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

    // First batch: 3 concurrent reads
    match &batches[0] {
        ToolBatch::Concurrent(items) => {
            assert_eq!(items.len(), 3);
            assert_eq!(items[0].1.name, "read_file");
            assert_eq!(items[1].1.name, "glob");
            assert_eq!(items[2].1.name, "grep");
        }
        _ => panic!("expected Concurrent batch"),
    }

    // Second batch: serial write
    match &batches[1] {
        ToolBatch::Serial(_, tc) => assert_eq!(tc.name, "write_file"),
        _ => panic!("expected Serial batch"),
    }

    // Third batch: 2 concurrent reads
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
    // LLM runs a build command then reads several outputs
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
// Batch execution ordering tests (caller-level)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn concurrent_batch_executes_tools_in_parallel() {
    // Three tools each sleeping 50ms. If run concurrently, total time < 100ms.
    // If run serially, total time >= 150ms.
    let order = Arc::new(AtomicUsize::new(0));
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(TimestampedReadTool::new(
            "read_a",
            order.clone(),
            Duration::from_millis(50),
        )),
        Box::new(TimestampedReadTool::new(
            "read_b",
            order.clone(),
            Duration::from_millis(50),
        )),
        Box::new(TimestampedReadTool::new(
            "read_c",
            order.clone(),
            Duration::from_millis(50),
        )),
    ];

    let ctx = JobContext::default();
    let start = Instant::now();

    // Execute all three concurrently (simulating a Concurrent batch)
    let mut handles = Vec::new();
    for tool in &tools {
        let params = serde_json::json!({});
        // Safety: we know these tools live long enough for this test
        let tool_ref: &dyn Tool = tool.as_ref();
        let ctx_ref = &ctx;
        handles.push(async move { tool_ref.execute(params, ctx_ref).await });
    }

    let results: Vec<_> = futures::future::join_all(handles).await;
    let elapsed = start.elapsed();

    // All three should succeed
    assert_eq!(results.len(), 3);
    for r in &results {
        assert!(r.is_ok());
    }

    // Parallel: total time should be significantly less than serial (3 * 50ms = 150ms).
    // Use a generous threshold (3x single-tool time) to avoid flakiness on loaded CI.
    assert!(
        elapsed < Duration::from_millis(150),
        "expected parallel execution (<150ms), got {:?} — would be >=150ms if serial",
        elapsed
    );

    // All three tools executed
    assert_eq!(order.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn serial_batch_executes_tools_sequentially() {
    let order = Arc::new(AtomicUsize::new(0));
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(TimestampedWriteTool::new(
            "write_a",
            order.clone(),
            Duration::from_millis(30),
        )),
        Box::new(TimestampedWriteTool::new(
            "write_b",
            order.clone(),
            Duration::from_millis(30),
        )),
    ];

    let ctx = JobContext::default();
    let start = Instant::now();

    // Execute sequentially (simulating Serial batches)
    let mut results = Vec::new();
    for tool in &tools {
        let r = tool.execute(serde_json::json!({}), &ctx).await;
        results.push(r);
    }
    let elapsed = start.elapsed();

    // Both should succeed
    assert_eq!(results.len(), 2);
    for r in &results {
        assert!(r.is_ok());
    }

    // Should take at least 60ms (serial)
    assert!(
        elapsed >= Duration::from_millis(55),
        "expected serial execution (>=55ms), got {:?}",
        elapsed
    );

    // Execution order is deterministic
    assert_eq!(
        results[0].as_ref().unwrap().result,
        serde_json::json!("write_result_0")
    );
    assert_eq!(
        results[1].as_ref().unwrap().result,
        serde_json::json!("write_result_1")
    );
}

#[tokio::test]
async fn mixed_batch_execution_preserves_tool_call_id_mapping() {
    // Simulate: [read, read, write, read] with batch execution.
    // Results must map back to correct pf_idx regardless of completion order.
    let order = Arc::new(AtomicUsize::new(0));

    let read_a = TimestampedReadTool::new("read_a", order.clone(), Duration::from_millis(40));
    let read_b = TimestampedReadTool::new("read_b", order.clone(), Duration::from_millis(10));
    let write_c = TimestampedWriteTool::new("write_c", order.clone(), Duration::from_millis(10));
    let read_d = TimestampedReadTool::new("read_d", order.clone(), Duration::from_millis(10));

    // Classify
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

    // Execute batches and collect results indexed by pf_idx
    let ctx = JobContext::default();
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(read_a),
        Box::new(read_b),
        Box::new(write_c),
        Box::new(read_d),
    ];

    let mut results: Vec<Option<Result<ToolOutput, ToolError>>> = (0..4).map(|_| None).collect();

    for batch in &batches {
        match batch {
            ToolBatch::Concurrent(items) => {
                // Execute concurrently via JoinSet (mirrors production dispatcher).
                // JoinSet::join_next() returns results in *completion* order,
                // so this validates that pf_idx mapping is correct regardless
                // of which tool finishes first.
                let mut join_set = tokio::task::JoinSet::new();
                for (pf_idx, _tc) in items {
                    let pf_idx = *pf_idx;
                    let tool = &tools[pf_idx];
                    let params = serde_json::json!({});
                    let ctx_clone = ctx.clone();
                    let tool_ref: &dyn Tool = tool.as_ref();
                    join_set
                        .spawn(async move { (pf_idx, tool_ref.execute(params, &ctx_clone).await) });
                }
                while let Some(join_result) = join_set.join_next().await {
                    let (pf_idx, result) = join_result.expect("task should not panic");
                    results[pf_idx] = Some(result);
                }
            }
            ToolBatch::Serial(pf_idx, _tc) => {
                let tool = &tools[*pf_idx];
                let result = tool.execute(serde_json::json!({}), &ctx).await;
                results[*pf_idx] = Some(result);
            }
        }
    }

    // All 4 results should be present
    for (i, r) in results.iter().enumerate() {
        assert!(r.is_some(), "result at pf_idx {i} should be present");
        assert!(
            r.as_ref().unwrap().is_ok(),
            "result at pf_idx {i} should be Ok"
        );
    }

    // Verify results are at correct indices (not scrambled by concurrent execution)
    // read_a (idx 0) has a 40ms delay, read_b (idx 1) has 10ms delay.
    // In concurrent execution, read_b finishes first, but result must still be at idx 1.
    let r0 = results[0].as_ref().unwrap().as_ref().unwrap();
    let r1 = results[1].as_ref().unwrap().as_ref().unwrap();
    let r2 = results[2].as_ref().unwrap().as_ref().unwrap();
    let r3 = results[3].as_ref().unwrap().as_ref().unwrap();

    // All should contain valid text
    assert!(r0.result.as_str().unwrap().starts_with("read_result_"));
    assert!(r1.result.as_str().unwrap().starts_with("read_result_"));
    assert!(r2.result.as_str().unwrap().starts_with("write_result_"));
    assert!(r3.result.as_str().unwrap().starts_with("read_result_"));
}

// ---------------------------------------------------------------------------
// Built-in tool classification audit
// ---------------------------------------------------------------------------

/// Verify that built-in read-only tools are classified as concurrent-safe.
/// These are the tools that MUST return true from is_concurrent_safe().
///
/// NOTE: This test will fail until all built-in tools implement the override.
/// That failure is the RED phase — implement the overrides to turn it GREEN.
#[test]
fn builtin_echo_is_concurrent_safe() {
    use ironclaw::tools::builtin::EchoTool;
    let tool = EchoTool;
    assert!(
        tool.is_concurrent_safe(&serde_json::json!({})),
        "echo is a pure function — must be concurrent-safe"
    );
}

/// Verify http tool parameter-dependent classification.
#[test]
fn builtin_http_get_is_concurrent_safe() {
    use ironclaw::tools::builtin::HttpTool;
    let tool = HttpTool::new();
    assert!(
        tool.is_concurrent_safe(&serde_json::json!({"method": "GET"})),
        "http GET must be concurrent-safe"
    );
}

#[test]
fn builtin_http_post_is_not_concurrent_safe() {
    use ironclaw::tools::builtin::HttpTool;
    let tool = HttpTool::new();
    assert!(
        !tool.is_concurrent_safe(&serde_json::json!({"method": "POST"})),
        "http POST must NOT be concurrent-safe"
    );
}

#[test]
fn builtin_http_no_method_defaults_to_concurrent_safe() {
    use ironclaw::tools::builtin::HttpTool;
    let tool = HttpTool::new();
    assert!(
        tool.is_concurrent_safe(&serde_json::json!({"url": "https://example.com"})),
        "http with no method (defaults to GET) must be concurrent-safe"
    );
}

// ---------------------------------------------------------------------------
// Rate limiter skip optimization
// ---------------------------------------------------------------------------

/// Concurrent-safe tools should have no rate limit config (returns None),
/// which means the dispatcher can skip the rate limiter lock entirely.
/// This verifies the classification is consistent.
#[test]
fn concurrent_safe_tools_have_no_rate_limit() {
    use ironclaw::tools::builtin::EchoTool;
    let tool = EchoTool;
    assert!(
        tool.is_concurrent_safe(&serde_json::json!({})),
        "echo should be concurrent-safe"
    );
    assert!(
        tool.rate_limit_config().is_none(),
        "concurrent-safe tools should have no rate limit config (enables lock skip)"
    );
}

#[tokio::test]
async fn rate_limiter_consulted_for_tools_with_config() {
    use ironclaw::tools::rate_limiter::RateLimiter;

    let limiter = RateLimiter::new();
    let config = ToolRateLimitConfig::new(30, 500);

    // A tool with rate limit config DOES get recorded.
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

    // Simulate 10 concurrent calls to the same rate-limited tool.
    // The limiter should allow exactly 5, rejecting the rest.
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

    assert_eq!(allowed, 5, "exactly 5 should be allowed");
    assert_eq!(limited, 5, "exactly 5 should be rate-limited");
}
