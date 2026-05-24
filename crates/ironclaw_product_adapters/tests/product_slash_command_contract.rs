//! Contract tests for shared edge slash-command parsing.

use ironclaw_product_adapters::{ProductTriggerReason, parse_product_slash_command};

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

    assert_eq!(
        err,
        ironclaw_product_adapters::ProductSlashCommandParseError::Empty
    );
}
