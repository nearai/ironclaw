//! Batch partitioning for concurrent tool execution.
//!
//! When the LLM returns multiple tool calls in a single response, this module
//! partitions them into batches based on concurrency safety:
//! - Adjacent concurrent-safe tools are grouped into a single parallel batch.
//! - Mutating tools get their own serial batch.
//!
//! The dispatcher executes batches sequentially: parallel batches use `JoinSet`,
//! serial batches execute one tool at a time.

use crate::llm::ToolCall;

/// A batch of tool calls to execute together.
#[derive(Debug, Clone)]
pub enum ToolBatch {
    /// Tools that are safe to run concurrently via `JoinSet`.
    /// Contains `(preflight_index, tool_call)` pairs.
    Concurrent(Vec<(usize, ToolCall)>),
    /// A single tool that must run serially (mutating / not concurrent-safe).
    Serial(usize, ToolCall),
}

impl ToolBatch {
    /// Number of tool calls in this batch.
    pub fn len(&self) -> usize {
        match self {
            ToolBatch::Concurrent(items) => items.len(),
            ToolBatch::Serial(..) => 1,
        }
    }

    /// Whether this batch is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Partition tool calls into batches based on concurrency safety classification.
///
/// `classified` is a list of `(preflight_index, tool_call, is_concurrent_safe)` triples.
/// Adjacent concurrent-safe tools are merged into `Concurrent` batches; mutating tools
/// become individual `Serial` batches. Order is preserved.
///
/// # Examples
///
/// ```text
/// Input:  [safe, safe, mutating, safe, safe]
/// Output: [Concurrent([0,1]), Serial(2), Concurrent([3,4])]
///
/// Input:  [mutating, mutating, mutating]
/// Output: [Serial(0), Serial(1), Serial(2)]
///
/// Input:  [safe, safe, safe]
/// Output: [Concurrent([0,1,2])]
/// ```
pub fn partition_tool_calls(
    classified: Vec<(usize, ToolCall, bool)>,
    max_concurrent: usize,
) -> Vec<ToolBatch> {
    if classified.is_empty() {
        return Vec::new();
    }

    // Clamp to at least 1 so max_concurrent=0 doesn't cause surprising behavior.
    let max_concurrent = max_concurrent.max(1);

    let mut batches = Vec::new();
    let mut current_concurrent: Vec<(usize, ToolCall)> = Vec::new();

    for (pf_idx, tc, is_safe) in classified {
        if is_safe {
            current_concurrent.push((pf_idx, tc));
            // If we hit the concurrency limit, flush the current batch
            if current_concurrent.len() >= max_concurrent {
                batches.push(ToolBatch::Concurrent(std::mem::take(
                    &mut current_concurrent,
                )));
            }
        } else {
            // Flush any pending concurrent batch before the serial tool
            if !current_concurrent.is_empty() {
                batches.push(ToolBatch::Concurrent(std::mem::take(
                    &mut current_concurrent,
                )));
            }
            batches.push(ToolBatch::Serial(pf_idx, tc));
        }
    }

    // Flush remaining concurrent tools
    if !current_concurrent.is_empty() {
        batches.push(ToolBatch::Concurrent(current_concurrent));
    }

    batches
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create a ToolCall with a given name and sequential ID.
    fn tc(name: &str, idx: usize) -> ToolCall {
        ToolCall {
            id: format!("call_{idx}"),
            name: name.to_string(),
            arguments: serde_json::json!({}),
            reasoning: None,
        }
    }

    // -----------------------------------------------------------------------
    // Basic partitioning
    // -----------------------------------------------------------------------

    #[test]
    fn empty_input_produces_no_batches() {
        let batches = partition_tool_calls(vec![], 10);
        assert!(batches.is_empty());
    }

    #[test]
    fn single_safe_tool_produces_one_concurrent_batch() {
        let classified = vec![(0, tc("echo", 0), true)];
        let batches = partition_tool_calls(classified, 10);

        assert_eq!(batches.len(), 1);
        assert!(matches!(&batches[0], ToolBatch::Concurrent(items) if items.len() == 1));
    }

    #[test]
    fn single_mutating_tool_produces_one_serial_batch() {
        let classified = vec![(0, tc("shell", 0), false)];
        let batches = partition_tool_calls(classified, 10);

        assert_eq!(batches.len(), 1);
        assert!(matches!(&batches[0], ToolBatch::Serial(0, _)));
    }

    #[test]
    fn all_safe_tools_merge_into_one_concurrent_batch() {
        let classified = vec![
            (0, tc("echo", 0), true),
            (1, tc("glob", 1), true),
            (2, tc("grep", 2), true),
            (3, tc("memory_search", 3), true),
            (4, tc("read_file", 4), true),
        ];
        let batches = partition_tool_calls(classified, 10);

        assert_eq!(batches.len(), 1);
        match &batches[0] {
            ToolBatch::Concurrent(items) => assert_eq!(items.len(), 5),
            _ => panic!("expected Concurrent batch"),
        }
    }

    #[test]
    fn all_mutating_tools_produce_individual_serial_batches() {
        let classified = vec![
            (0, tc("shell", 0), false),
            (1, tc("write_file", 1), false),
            (2, tc("memory_write", 2), false),
        ];
        let batches = partition_tool_calls(classified, 10);

        assert_eq!(batches.len(), 3);
        for (i, batch) in batches.iter().enumerate() {
            match batch {
                ToolBatch::Serial(pf_idx, _) => assert_eq!(*pf_idx, i),
                _ => panic!("expected Serial batch at index {i}"),
            }
        }
    }

    // -----------------------------------------------------------------------
    // Mixed sequences
    // -----------------------------------------------------------------------

    #[test]
    fn safe_safe_mutating_safe_partitions_into_three_batches() {
        // [safe, safe, mutating, safe] -> [[safe, safe], [mutating], [safe]]
        let classified = vec![
            (0, tc("echo", 0), true),
            (1, tc("glob", 1), true),
            (2, tc("shell", 2), false),
            (3, tc("grep", 3), true),
        ];
        let batches = partition_tool_calls(classified, 10);

        assert_eq!(batches.len(), 3);

        // Batch 0: Concurrent [echo, glob]
        match &batches[0] {
            ToolBatch::Concurrent(items) => {
                assert_eq!(items.len(), 2);
                assert_eq!(items[0].0, 0); // pf_idx
                assert_eq!(items[1].0, 1);
                assert_eq!(items[0].1.name, "echo");
                assert_eq!(items[1].1.name, "glob");
            }
            _ => panic!("expected Concurrent batch at index 0"),
        }

        // Batch 1: Serial [shell]
        match &batches[1] {
            ToolBatch::Serial(pf_idx, tc) => {
                assert_eq!(*pf_idx, 2);
                assert_eq!(tc.name, "shell");
            }
            _ => panic!("expected Serial batch at index 1"),
        }

        // Batch 2: Concurrent [grep]
        match &batches[2] {
            ToolBatch::Concurrent(items) => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].0, 3);
                assert_eq!(items[0].1.name, "grep");
            }
            _ => panic!("expected Concurrent batch at index 2"),
        }
    }

    #[test]
    fn mutating_then_safe_tools() {
        // [mutating, safe, safe] -> [[mutating], [safe, safe]]
        let classified = vec![
            (0, tc("write_file", 0), false),
            (1, tc("echo", 1), true),
            (2, tc("glob", 2), true),
        ];
        let batches = partition_tool_calls(classified, 10);

        assert_eq!(batches.len(), 2);
        assert!(matches!(&batches[0], ToolBatch::Serial(0, _)));
        match &batches[1] {
            ToolBatch::Concurrent(items) => assert_eq!(items.len(), 2),
            _ => panic!("expected Concurrent batch"),
        }
    }

    #[test]
    fn alternating_safe_and_mutating() {
        // [safe, mutating, safe, mutating, safe]
        let classified = vec![
            (0, tc("echo", 0), true),
            (1, tc("shell", 1), false),
            (2, tc("glob", 2), true),
            (3, tc("write_file", 3), false),
            (4, tc("grep", 4), true),
        ];
        let batches = partition_tool_calls(classified, 10);

        assert_eq!(batches.len(), 5);
        assert!(matches!(&batches[0], ToolBatch::Concurrent(items) if items.len() == 1));
        assert!(matches!(&batches[1], ToolBatch::Serial(1, _)));
        assert!(matches!(&batches[2], ToolBatch::Concurrent(items) if items.len() == 1));
        assert!(matches!(&batches[3], ToolBatch::Serial(3, _)));
        assert!(matches!(&batches[4], ToolBatch::Concurrent(items) if items.len() == 1));
    }

    #[test]
    fn consecutive_mutating_tools_each_get_own_batch() {
        // [mutating, mutating, safe] -> [[mutating], [mutating], [safe]]
        let classified = vec![
            (0, tc("shell", 0), false),
            (1, tc("write_file", 1), false),
            (2, tc("echo", 2), true),
        ];
        let batches = partition_tool_calls(classified, 10);

        assert_eq!(batches.len(), 3);
        assert!(matches!(&batches[0], ToolBatch::Serial(0, _)));
        assert!(matches!(&batches[1], ToolBatch::Serial(1, _)));
        assert!(matches!(&batches[2], ToolBatch::Concurrent(items) if items.len() == 1));
    }

    // -----------------------------------------------------------------------
    // Concurrency limit
    // -----------------------------------------------------------------------

    #[test]
    fn max_concurrent_splits_large_safe_batch() {
        // 5 safe tools with max_concurrent=3 -> [[safe x3], [safe x2]]
        let classified = vec![
            (0, tc("echo", 0), true),
            (1, tc("glob", 1), true),
            (2, tc("grep", 2), true),
            (3, tc("time", 3), true),
            (4, tc("json", 4), true),
        ];
        let batches = partition_tool_calls(classified, 3);

        assert_eq!(batches.len(), 2);
        match &batches[0] {
            ToolBatch::Concurrent(items) => assert_eq!(items.len(), 3),
            _ => panic!("expected Concurrent batch of size 3"),
        }
        match &batches[1] {
            ToolBatch::Concurrent(items) => assert_eq!(items.len(), 2),
            _ => panic!("expected Concurrent batch of size 2"),
        }
    }

    #[test]
    fn max_concurrent_one_makes_all_safe_tools_individual_batches() {
        let classified = vec![
            (0, tc("echo", 0), true),
            (1, tc("glob", 1), true),
            (2, tc("grep", 2), true),
        ];
        let batches = partition_tool_calls(classified, 1);

        assert_eq!(batches.len(), 3);
        for batch in &batches {
            match batch {
                ToolBatch::Concurrent(items) => assert_eq!(items.len(), 1),
                _ => panic!("expected single-item Concurrent batches"),
            }
        }
    }

    #[test]
    fn max_concurrent_exactly_matches_tool_count() {
        let classified = vec![
            (0, tc("echo", 0), true),
            (1, tc("glob", 1), true),
            (2, tc("grep", 2), true),
        ];
        let batches = partition_tool_calls(classified, 3);

        assert_eq!(batches.len(), 1);
        match &batches[0] {
            ToolBatch::Concurrent(items) => assert_eq!(items.len(), 3),
            _ => panic!("expected single Concurrent batch"),
        }
    }

    #[test]
    fn max_concurrent_zero_clamped_to_one() {
        // max_concurrent=0 should behave like max_concurrent=1 (not panic or loop)
        let classified = vec![
            (0, tc("echo", 0), true),
            (1, tc("glob", 1), true),
            (2, tc("grep", 2), true),
        ];
        let batches = partition_tool_calls(classified, 0);

        // Clamped to 1: each safe tool gets its own batch
        assert_eq!(batches.len(), 3);
        for batch in &batches {
            match batch {
                ToolBatch::Concurrent(items) => assert_eq!(items.len(), 1),
                _ => panic!("expected single-item Concurrent batches"),
            }
        }
    }

    #[test]
    fn max_concurrent_does_not_affect_serial_tools() {
        // max_concurrent=2, [safe, safe, safe, mutating, safe]
        // -> [[safe x2], [safe x1], [mutating], [safe x1]]
        let classified = vec![
            (0, tc("echo", 0), true),
            (1, tc("glob", 1), true),
            (2, tc("grep", 2), true),
            (3, tc("shell", 3), false),
            (4, tc("time", 4), true),
        ];
        let batches = partition_tool_calls(classified, 2);

        assert_eq!(batches.len(), 4);
        assert!(matches!(&batches[0], ToolBatch::Concurrent(items) if items.len() == 2));
        assert!(matches!(&batches[1], ToolBatch::Concurrent(items) if items.len() == 1));
        assert!(matches!(&batches[2], ToolBatch::Serial(3, _)));
        assert!(matches!(&batches[3], ToolBatch::Concurrent(items) if items.len() == 1));
    }

    // -----------------------------------------------------------------------
    // Preflight index preservation
    // -----------------------------------------------------------------------

    #[test]
    fn preflight_indices_preserved_through_partitioning() {
        // Non-contiguous pf indices (some tools may have been filtered by preflight)
        let classified = vec![
            (0, tc("echo", 0), true),
            (2, tc("glob", 2), true), // pf_idx 1 was filtered by approval
            (3, tc("shell", 3), false),
            (5, tc("grep", 5), true), // pf_idx 4 was filtered
        ];
        let batches = partition_tool_calls(classified, 10);

        assert_eq!(batches.len(), 3);

        // Concurrent batch preserves indices 0, 2
        match &batches[0] {
            ToolBatch::Concurrent(items) => {
                assert_eq!(items[0].0, 0);
                assert_eq!(items[1].0, 2);
            }
            _ => panic!("expected Concurrent"),
        }

        // Serial batch preserves index 3
        match &batches[1] {
            ToolBatch::Serial(pf_idx, _) => assert_eq!(*pf_idx, 3),
            _ => panic!("expected Serial"),
        }

        // Concurrent batch preserves index 5
        match &batches[2] {
            ToolBatch::Concurrent(items) => assert_eq!(items[0].0, 5),
            _ => panic!("expected Concurrent"),
        }
    }

    // -----------------------------------------------------------------------
    // Tool call ID integrity
    // -----------------------------------------------------------------------

    #[test]
    fn tool_call_ids_preserved_through_partitioning() {
        let classified = vec![
            (0, tc("echo", 0), true),
            (1, tc("shell", 1), false),
            (2, tc("grep", 2), true),
        ];
        let batches = partition_tool_calls(classified, 10);

        // Extract all IDs in batch order
        let mut ids = Vec::new();
        for batch in &batches {
            match batch {
                ToolBatch::Concurrent(items) => {
                    for (_, tc) in items {
                        ids.push(tc.id.clone());
                    }
                }
                ToolBatch::Serial(_, tc) => ids.push(tc.id.clone()),
            }
        }

        assert_eq!(ids, vec!["call_0", "call_1", "call_2"]);
    }

    // -----------------------------------------------------------------------
    // ToolBatch helpers
    // -----------------------------------------------------------------------

    #[test]
    fn batch_len_concurrent() {
        let batch = ToolBatch::Concurrent(vec![(0, tc("echo", 0)), (1, tc("glob", 1))]);
        assert_eq!(batch.len(), 2);
        assert!(!batch.is_empty());
    }

    #[test]
    fn batch_len_serial() {
        let batch = ToolBatch::Serial(0, tc("shell", 0));
        assert_eq!(batch.len(), 1);
        assert!(!batch.is_empty());
    }

    #[test]
    fn batch_concurrent_empty_is_empty() {
        let batch = ToolBatch::Concurrent(vec![]);
        assert_eq!(batch.len(), 0);
        assert!(batch.is_empty());
    }
}
