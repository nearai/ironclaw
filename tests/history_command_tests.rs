//! Tests for /history and /thread list command parsing.
//!
//! These tests validate:
//! - Parser recognizes /history and /thread list commands
//! - Command routing behavior
//! - Case insensitivity

use ironclaw::agent::submission::{Submission, SubmissionParser};

#[test]
fn test_parser_history_command() {
    let submission = SubmissionParser::parse("/history");
    assert!(matches!(
        submission,
        Submission::SystemCommand { ref command, ref args }
            if command == "history" && args.is_empty()
    ));
}

#[test]
fn test_parser_history_with_args() {
    let submission = SubmissionParser::parse("/history all");
    assert!(matches!(
        submission,
        Submission::SystemCommand { ref command, ref args }
            if command == "history" && args == &vec!["all".to_string()]
    ));
}

#[test]
fn test_parser_thread_list_alias() {
    let submission = SubmissionParser::parse("/thread list");
    assert!(matches!(
        submission,
        Submission::SystemCommand { ref command, ref args }
            if command == "history" && args.is_empty()
    ));
}

#[test]
fn test_parser_thread_list_case_insensitive() {
    let submission = SubmissionParser::parse("/thread LIST");
    assert!(matches!(
        submission,
        Submission::SystemCommand { ref command, ref args }
            if command == "history" && args.is_empty()
    ));
}

#[test]
fn test_parser_thread_no_args() {
    let submission = SubmissionParser::parse("/thread");
    assert!(matches!(
        submission,
        Submission::SystemCommand { ref command, ref args }
            if command == "thread" && args.is_empty()
    ));
}

#[test]
fn test_parser_thread_new() {
    let submission = SubmissionParser::parse("/thread new");
    assert!(matches!(submission, Submission::NewThread));
}

#[test]
fn test_parser_thread_new_alias() {
    let submission = SubmissionParser::parse("/new");
    assert!(matches!(submission, Submission::NewThread));
}

#[test]
fn test_parser_thread_switch() {
    let uuid = "11111111-1111-1111-1111-111111111111";
    let submission = SubmissionParser::parse(&format!("/thread {}", uuid));
    assert!(matches!(
        submission,
        Submission::SwitchThread { thread_id } if thread_id.to_string() == uuid
    ));
}

#[test]
fn test_history_command_variations() {
    let variations = vec!["/history", "/history ", "/HISTORY", "/History"];

    for variation in variations {
        let submission = SubmissionParser::parse(variation);
        assert!(
            matches!(
                submission,
                Submission::SystemCommand { ref command, ref args }
                    if command == "history" && args.is_empty()
            ),
            "Failed for variation: {}",
            variation
        );
    }
}

#[test]
fn test_thread_list_alias_variations() {
    let variations = vec![
        "/thread list",
        "/thread LIST",
        "/thread List",
        "/THREAD LIST",
    ];

    for variation in variations {
        let submission = SubmissionParser::parse(variation);
        assert!(
            matches!(
                submission,
                Submission::SystemCommand { ref command, ref args }
                    if command == "history" && args.is_empty()
            ),
            "Failed for variation: {}",
            variation
        );
    }
}

#[test]
fn test_history_messages_command() {
    // Test basic /history messages command
    let submission = SubmissionParser::parse("/history messages");
    assert!(matches!(
        submission,
        Submission::SystemCommand { ref command, ref args }
            if command == "history" && args == &vec!["messages".to_string()]
    ));
}

#[test]
fn test_history_messages_subcommand_case_insensitive() {
    let submission =
        SubmissionParser::parse("/history MESSAGES 11111111-1111-1111-1111-111111111111");
    assert!(matches!(
        submission,
        Submission::SystemCommand { ref command, ref args }
            if command == "history"
                && args.len() == 2
                && args[0] == "messages"
                && args[1] == "11111111-1111-1111-1111-111111111111"
    ));
}

#[test]
fn test_history_messages_with_uuid() {
    let uuid = "11111111-1111-1111-1111-111111111111";
    let submission = SubmissionParser::parse(&format!("/history messages {}", uuid));
    assert!(matches!(
        submission,
        Submission::SystemCommand { ref command, ref args }
            if command == "history" && args.len() == 2 && args[0] == "messages" && args[1] == uuid
    ));
}

#[test]
fn test_history_messages_with_pagination() {
    let uuid = "11111111-1111-1111-1111-111111111111";
    let submission =
        SubmissionParser::parse(&format!("/history messages {} --limit 20 --page 2", uuid));
    let expected_args = vec![
        "messages".to_string(),
        uuid.to_string(),
        "--limit".to_string(),
        "20".to_string(),
        "--page".to_string(),
        "2".to_string(),
    ];
    assert!(matches!(
        submission,
        Submission::SystemCommand { ref command, ref args }
            if command == "history" && args == &expected_args
    ));
}
