//! Contract tests for the Reborn-native product command model.

use ironclaw_product_adapters::{InboundCommandPayload, ProductTriggerReason};
use ironclaw_product_workflow::{ProductCommand, ProductModelCommand, product_command_descriptors};

#[test]
fn command_payload_maps_to_typed_model_command_without_v1_parser() {
    let payload =
        InboundCommandPayload::new("model", "gpt-5-mini", ProductTriggerReason::BotCommand)
            .expect("valid command");

    assert_eq!(
        ProductCommand::from_payload(&payload),
        ProductCommand::Model {
            action: ProductModelCommand::Set {
                model: "gpt-5-mini".to_string(),
            }
        }
    );
}

#[test]
fn command_payload_maps_all_declared_commands_and_unknown_fallback() {
    let cases = [
        (
            "model",
            "",
            ProductCommand::Model {
                action: ProductModelCommand::Status,
            },
            "model",
            Some("model"),
        ),
        (
            "status",
            "",
            ProductCommand::Status,
            "status",
            Some("status"),
        ),
        (
            "progress",
            "",
            ProductCommand::Status,
            "status",
            Some("status"),
        ),
        (
            "unknown",
            "raw args",
            ProductCommand::Unknown {
                name: "unknown".to_string(),
                arguments: "raw args".to_string(),
            },
            "unknown",
            None,
        ),
    ];

    for (name, arguments, expected, expected_name, expected_descriptor) in cases {
        let payload = InboundCommandPayload::new(name, arguments, ProductTriggerReason::BotCommand)
            .expect("valid command payload");
        let command = ProductCommand::from_payload(&payload);

        assert_eq!(command, expected);
        assert_eq!(command.name(), expected_name);
        assert_eq!(
            command.descriptor().map(|descriptor| descriptor.name),
            expected_descriptor
        );
    }
}

#[test]
fn command_registry_declares_model_without_source_policy() {
    let model = product_command_descriptors()
        .find(|descriptor| descriptor.name == "model")
        .expect("model descriptor");

    assert!(model.aliases.is_empty());
}
