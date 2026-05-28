//! Shared grammar for provider transcript artifacts.
//!
//! Some providers cannot preserve native tool-result roles and need a textual
//! compatibility representation. Keep that representation centralized so
//! producers, response cleanup, and final-reply admission cannot drift.

pub const LEGACY_TOOL_EVENT_PREFIX: &str = "Previous tool event: ";
pub const LEGACY_TOOL_EVENT_SUFFIX: &str = " was invoked.";
pub const LEGACY_TOOL_RESULT_PREFIX: &str = "Previous tool result from ";
pub const TOOL_RESULT_OBSERVATION_PREFIX: &str = "Tool result from ";

pub fn format_tool_result_observation(tool_name: &str, result: &str) -> String {
    if result.is_empty() {
        format!("{TOOL_RESULT_OBSERVATION_PREFIX}{tool_name}:")
    } else {
        format!("{TOOL_RESULT_OBSERVATION_PREFIX}{tool_name}: {result}")
    }
}

pub fn is_provider_transcript_artifact_line(line: &str) -> bool {
    let line = line.trim();
    is_legacy_tool_event_line(line)
        || is_tool_result_line(line, LEGACY_TOOL_RESULT_PREFIX)
        || is_tool_result_line(line, TOOL_RESULT_OBSERVATION_PREFIX)
}

fn is_legacy_tool_event_line(line: &str) -> bool {
    let Some(tool_name) = line
        .strip_prefix(LEGACY_TOOL_EVENT_PREFIX)
        .and_then(|rest| rest.strip_suffix(LEGACY_TOOL_EVENT_SUFFIX))
    else {
        return false;
    };
    is_transcript_tool_name(tool_name)
}

fn is_tool_result_line(line: &str, prefix: &str) -> bool {
    let Some(rest) = line.strip_prefix(prefix) else {
        return false;
    };
    let Some((tool_name, _result)) = rest.split_once(':') else {
        return false;
    };
    is_transcript_tool_name(tool_name)
}

fn is_transcript_tool_name(name: &str) -> bool {
    name.contains("__")
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

pub fn strip_provider_transcript_artifact_lines(text: &str) -> String {
    if !text.lines().any(is_provider_transcript_artifact_line) {
        return text.to_string();
    }

    let had_trailing_newline = text.ends_with('\n');
    let mut stripped = text
        .lines()
        .filter(|line| !is_provider_transcript_artifact_line(line))
        .collect::<Vec<_>>()
        .join("\n");
    if had_trailing_newline && !stripped.is_empty() {
        stripped.push('\n');
    }
    stripped
}

pub fn is_only_provider_transcript_artifact_lines(text: &str) -> bool {
    let mut meaningful_lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let Some(first) = meaningful_lines.next() else {
        return false;
    };
    is_provider_transcript_artifact_line(first)
        && meaningful_lines.all(is_provider_transcript_artifact_line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_and_classifies_tool_result_observations() {
        let observation = format_tool_result_observation("demo__echo", "hi");
        assert_eq!(observation, "Tool result from demo__echo: hi");
        assert!(is_provider_transcript_artifact_line(&observation));
    }

    #[test]
    fn classifies_legacy_artifact_lines() {
        assert!(is_provider_transcript_artifact_line(
            "Previous tool event: demo__echo was invoked."
        ));
        assert!(is_provider_transcript_artifact_line(
            "Previous tool result from demo__echo: hi"
        ));
    }

    #[test]
    fn strips_artifact_lines_without_removing_normal_text() {
        let text =
            "Done.\nPrevious tool event: demo__echo was invoked.\nTool result from demo__echo: hi";
        assert_eq!(strip_provider_transcript_artifact_lines(text), "Done.");
    }

    #[test]
    fn strip_preserves_non_artifact_text_verbatim() {
        let text = "Done.\n";

        assert_eq!(strip_provider_transcript_artifact_lines(text), text);
    }

    #[test]
    fn strip_preserves_trailing_newline_after_removing_artifacts() {
        let text = "Done.\nTool result from demo__echo: hi\n";

        assert_eq!(strip_provider_transcript_artifact_lines(text), "Done.\n");
    }

    #[test]
    fn detects_replay_only_artifacts() {
        assert!(is_only_provider_transcript_artifact_lines(
            "\nTool result from demo__echo: hi\n"
        ));
        assert!(!is_only_provider_transcript_artifact_lines(""));
        assert!(!is_only_provider_transcript_artifact_lines(" \n\t\n"));
        assert!(!is_only_provider_transcript_artifact_lines(
            "Done.\nTool result from demo__echo: hi"
        ));
    }

    #[test]
    fn does_not_classify_natural_language_tool_result_lines() {
        assert!(!is_provider_transcript_artifact_line(
            "Tool result from the benchmark: passed"
        ));
        assert!(!is_provider_transcript_artifact_line(
            "Previous tool event: the benchmark was invoked."
        ));
        assert!(!is_provider_transcript_artifact_line(
            "Previous tool result from the benchmark: passed"
        ));
        assert!(!is_provider_transcript_artifact_line(
            "Tool result from my_tool: success"
        ));
        assert!(!is_provider_transcript_artifact_line(
            "Previous tool event: cleanup was invoked."
        ));
        assert!(!is_provider_transcript_artifact_line(
            "Tool result from http: success"
        ));
    }
}
