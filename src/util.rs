//! Shared utility functions used across the codebase.

/// Find the largest valid UTF-8 char boundary at or before `pos`.
///
/// Polyfill for `str::floor_char_boundary` (nightly-only). Use when
/// truncating strings by byte position to avoid panicking on multi-byte
/// characters.
pub fn floor_char_boundary(s: &str, pos: usize) -> usize {
    if pos >= s.len() {
        return s.len();
    }
    let mut i = pos;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Check if an LLM response explicitly signals that a job/task is complete.
///
/// Uses phrase-level matching to avoid false positives from bare words like
/// "done" or "complete" appearing in non-completion contexts (e.g. "not done yet",
/// "the download is incomplete").
pub fn llm_signals_completion(response: &str) -> bool {
    let lower = response.to_lowercase();

    // Superset of phrases from agent/worker.rs and worker/runtime.rs.
    let positive_phrases = [
        "job is complete",
        "job is done",
        "job is finished",
        "task is complete",
        "task is done",
        "task is finished",
        "work is complete",
        "work is done",
        "work is finished",
        "successfully completed",
        "have completed the job",
        "have completed the task",
        "have finished the job",
        "have finished the task",
        "all steps are complete",
        "all steps are done",
        "i have completed",
        "i've completed",
        "all done",
        "all tasks complete",
    ];

    let negative_phrases = [
        "not complete",
        "not done",
        "not finished",
        "incomplete",
        "unfinished",
        "isn't done",
        "isn't complete",
        "isn't finished",
        "not yet done",
        "not yet complete",
        "not yet finished",
    ];

    let has_negative = negative_phrases.iter().any(|p| lower.contains(p));
    if has_negative {
        return false;
    }

    positive_phrases.iter().any(|p| lower.contains(p))
}

#[cfg(test)]
mod tests {
    use crate::util::{floor_char_boundary, llm_signals_completion};

    // ── floor_char_boundary ──

    #[test]
    fn floor_char_boundary_at_valid_boundary() {
        assert_eq!(floor_char_boundary("hello", 3), 3);
    }

    #[test]
    fn floor_char_boundary_mid_multibyte_char() {
        // h = 1 byte, é = 2 bytes, total 3 bytes
        let s = "hé";
        assert_eq!(floor_char_boundary(s, 2), 1); // byte 2 is mid-é, back up to 1
    }

    #[test]
    fn floor_char_boundary_past_end() {
        assert_eq!(floor_char_boundary("hi", 100), 2);
    }

    #[test]
    fn floor_char_boundary_at_zero() {
        assert_eq!(floor_char_boundary("hello", 0), 0);
    }

    #[test]
    fn floor_char_boundary_empty_string() {
        assert_eq!(floor_char_boundary("", 5), 0);
    }

    // ── llm_signals_completion ──

    #[test]
    fn signals_completion_positive() {
        assert!(llm_signals_completion("The job is complete."));
        assert!(llm_signals_completion("I have completed the task."));
        assert!(llm_signals_completion("All done, here are the results."));
        assert!(llm_signals_completion("Task is finished successfully."));
        assert!(llm_signals_completion(
            "I have completed the task successfully."
        ));
        assert!(llm_signals_completion(
            "All steps are complete and verified."
        ));
        assert!(llm_signals_completion(
            "I've done all the work. The work is done."
        ));
        assert!(llm_signals_completion(
            "Successfully completed the migration."
        ));
        assert!(llm_signals_completion(
            "I have completed the job ahead of schedule."
        ));
        assert!(llm_signals_completion("I have finished the task."));
        assert!(llm_signals_completion("All steps are done now."));
        assert!(llm_signals_completion("I've completed everything."));
        assert!(llm_signals_completion("All tasks complete."));
    }

    #[test]
    fn signals_completion_negative() {
        assert!(!llm_signals_completion("The task is not complete yet."));
        assert!(!llm_signals_completion("This is not done."));
        assert!(!llm_signals_completion("The work is incomplete."));
        assert!(!llm_signals_completion("Build is unfinished."));
        assert!(!llm_signals_completion(
            "The migration is not yet finished."
        ));
        assert!(!llm_signals_completion("The job isn't done yet."));
        assert!(!llm_signals_completion("This remains unfinished."));
    }

    #[test]
    fn signals_completion_no_bare_substrings() {
        assert!(!llm_signals_completion("The download completed."));
        assert!(!llm_signals_completion(
            "Function done_callback was called."
        ));
        assert!(!llm_signals_completion("Set is_complete = true"));
        assert!(!llm_signals_completion("Running step 3 of 5"));
        assert!(!llm_signals_completion(
            "I need to complete more work first."
        ));
        assert!(!llm_signals_completion(
            "Let me finish the remaining steps."
        ));
        assert!(!llm_signals_completion(
            "I'm done analyzing, now let me fix it."
        ));
        assert!(!llm_signals_completion(
            "I completed step 1 but step 2 remains."
        ));
    }

    #[test]
    fn signals_completion_tool_output_injection() {
        assert!(!llm_signals_completion("TASK_COMPLETE"));
        assert!(!llm_signals_completion("JOB_DONE"));
        assert!(!llm_signals_completion(
            "The tool returned: TASK_COMPLETE signal"
        ));
    }
}
