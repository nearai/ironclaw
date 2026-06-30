//! Contract tests for shared edge slash-command parsing.

use ironclaw_product_adapters::{
    ProductSlashCommandParseError, ProductTriggerReason, parse_product_slash_command,
};

#[test]
fn slash_parser_normalizes_command_name_and_preserves_arguments() {
    let payload = parse_product_slash_command(
        "  /MODEL Claude-3.5 --reasoning high  ",
        ProductTriggerReason::BotCommand,
    )
    .expect("parse")
    .expect("slash command");

    assert_eq!(payload.command, "model");
    assert_eq!(payload.arguments, "Claude-3.5 --reasoning high");
    assert_eq!(payload.trigger, ProductTriggerReason::BotCommand);
}

#[test]
fn slash_parser_ignores_ordinary_user_text() {
    let parsed =
        parse_product_slash_command("model this as a prompt", ProductTriggerReason::DirectChat)
            .expect("parse");

    assert!(parsed.is_none());
}

#[test]
fn slash_parser_rejects_empty_command() {
    let err = parse_product_slash_command("/", ProductTriggerReason::BotCommand)
        .expect_err("empty command must fail");

    assert_eq!(err, ProductSlashCommandParseError::Empty);
}

#[test]
fn slash_parser_rejects_invalid_command_names() {
    for input in ["//bad", "/bad\\name"] {
        let err = parse_product_slash_command(input, ProductTriggerReason::BotCommand)
            .expect_err("invalid command name must fail");

        assert!(matches!(
            err,
            ProductSlashCommandParseError::InvalidPayload(_)
        ));
    }
}

#[test]
fn slash_parser_rejects_oversized_command_and_arguments_before_payload_build() {
    let oversized_command = format!("/{}", "h".repeat(257));
    let err = parse_product_slash_command(&oversized_command, ProductTriggerReason::BotCommand)
        .expect_err("oversized command must fail");
    assert!(matches!(
        err,
        ProductSlashCommandParseError::InvalidPayload(_)
    ));

    let oversized_arguments = format!("/help {}", "a".repeat(64 * 1024 + 1));
    let err = parse_product_slash_command(&oversized_arguments, ProductTriggerReason::BotCommand)
        .expect_err("oversized arguments must fail");
    assert!(matches!(
        err,
        ProductSlashCommandParseError::InvalidPayload(_)
    ));
}
