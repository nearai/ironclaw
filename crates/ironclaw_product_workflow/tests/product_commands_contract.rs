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
fn command_registry_declares_model_without_source_policy() {
    let model = product_command_descriptors()
        .find(|descriptor| descriptor.name == "model")
        .expect("model descriptor");

    assert!(model.aliases.is_empty());
}
