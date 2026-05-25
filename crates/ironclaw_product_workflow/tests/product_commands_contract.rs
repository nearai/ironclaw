//! Contract tests for the Reborn-native product command model.

use ironclaw_product_adapters::{InboundCommandPayload, ProductTriggerReason};
use ironclaw_product_workflow::{
    LifecyclePackageId, LifecyclePackageKind, LifecyclePackageRef, LifecycleProductAction,
    ProductCommand, ProductModelCommand, product_command_descriptors,
};

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
            "extension_install",
            "github",
            ProductCommand::Lifecycle {
                action: LifecycleProductAction::ExtensionInstall {
                    package_ref: ironclaw_product_workflow::LifecyclePackageRef::new(
                        LifecyclePackageKind::Extension,
                        "github",
                    )
                    .unwrap(),
                },
            },
            "extension_install",
            Some("extension_install"),
        ),
        (
            "skill_install",
            r#"{"name":"review-helper","content":"---\nname: review-helper\n---\nUse review helper."}"#,
            ProductCommand::Lifecycle {
                action: LifecycleProductAction::SkillInstall {
                    name: Some(LifecyclePackageId::new("review-helper").unwrap()),
                    content: "---\nname: review-helper\n---\nUse review helper.".to_string(),
                },
            },
            "skill_install",
            Some("skill_install"),
        ),
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
fn lifecycle_command_parser_handles_json_forms_and_rejects_malformed_refs() {
    let configure_payload = InboundCommandPayload::new(
        "extension_configure",
        r#"{"id":"github","payload":{"mode":"dry_run"}}"#,
        ProductTriggerReason::BotCommand,
    )
    .expect("valid command payload");

    assert_eq!(
        ProductCommand::from_payload(&configure_payload),
        ProductCommand::Lifecycle {
            action: LifecycleProductAction::ExtensionConfigure {
                package_ref: ironclaw_product_workflow::LifecyclePackageRef::new(
                    LifecyclePackageKind::Extension,
                    "github",
                )
                .unwrap(),
                payload: Some(serde_json::json!({"mode": "dry_run"})),
            },
        }
    );

    for arguments in [r#"{"id":"review-helper"}"#, r#"{"name":"review-helper"}"#] {
        let payload =
            InboundCommandPayload::new("skill_remove", arguments, ProductTriggerReason::BotCommand)
                .expect("valid command payload");
        assert_eq!(
            ProductCommand::from_payload(&payload),
            ProductCommand::Lifecycle {
                action: LifecycleProductAction::SkillRemove {
                    package_ref: ironclaw_product_workflow::LifecyclePackageRef::new(
                        LifecyclePackageKind::Skill,
                        "review-helper",
                    )
                    .unwrap(),
                },
            }
        );
    }

    for (command, arguments) in [
        ("skill_remove", ""),
        ("extension_install", r#"{"id":"git\nhub"}"#),
        (
            "skill_install",
            r#"{"content":"---\nname: nul-skill\n---\nNo\u0000pe."}"#,
        ),
        (
            "skill_install",
            &format!(r#"{{"content":"{}"}}"#, "x".repeat(64 * 1024 + 1)),
        ),
    ] {
        let payload = InboundCommandPayload {
            command: command.to_string(),
            arguments: arguments.to_string(),
            trigger: ProductTriggerReason::BotCommand,
        };
        assert!(matches!(
            ProductCommand::from_payload(&payload),
            ProductCommand::Unknown { .. }
        ));
    }
}

#[test]
fn lifecycle_command_parser_maps_every_lifecycle_command_variant() {
    let cases = [
        (
            "extension_search",
            "git",
            ProductCommand::Lifecycle {
                action: LifecycleProductAction::ExtensionSearch {
                    query: "git".to_string(),
                },
            },
        ),
        (
            "extension_auth",
            "github",
            ProductCommand::Lifecycle {
                action: LifecycleProductAction::ExtensionAuth {
                    package_ref: LifecyclePackageRef::new(
                        LifecyclePackageKind::Extension,
                        "github",
                    )
                    .unwrap(),
                },
            },
        ),
        (
            "extension_activate",
            "github",
            ProductCommand::Lifecycle {
                action: LifecycleProductAction::ExtensionActivate {
                    package_ref: LifecyclePackageRef::new(
                        LifecyclePackageKind::Extension,
                        "github",
                    )
                    .unwrap(),
                },
            },
        ),
        (
            "extension_remove",
            r#"{"id":"github"}"#,
            ProductCommand::Lifecycle {
                action: LifecycleProductAction::ExtensionRemove {
                    package_ref: LifecyclePackageRef::new(
                        LifecyclePackageKind::Extension,
                        "github",
                    )
                    .unwrap(),
                },
            },
        ),
        (
            "skill_search",
            "review",
            ProductCommand::Lifecycle {
                action: LifecycleProductAction::SkillSearch {
                    query: "review".to_string(),
                },
            },
        ),
    ];

    for (command, arguments, expected) in cases {
        let payload =
            InboundCommandPayload::new(command, arguments, ProductTriggerReason::BotCommand)
                .expect("valid command payload");

        assert_eq!(ProductCommand::from_payload(&payload), expected);
    }
}

#[test]
fn lifecycle_refs_validate_during_deserialization() {
    for json in [
        r#"{"kind":"extension","id":""}"#,
        r#"{"kind":"extension","id":"git\nhub"}"#,
    ] {
        assert!(
            serde_json::from_str::<LifecyclePackageRef>(json).is_err(),
            "invalid lifecycle ref should reject: {json}"
        );
    }
}

#[test]
fn lifecycle_command_parser_rejects_invalid_skill_install_name() {
    let payload = InboundCommandPayload {
        command: "skill_install".to_string(),
        arguments: r#"{"name":"bad\nname","content":"---\nname: bad-name\n---\nUse bad name."}"#
            .to_string(),
        trigger: ProductTriggerReason::BotCommand,
    };

    assert!(matches!(
        ProductCommand::from_payload(&payload),
        ProductCommand::Unknown { .. }
    ));
}

#[test]
fn lifecycle_command_parser_preserves_skill_install_content() {
    let content = "---\nname: review-helper\n---\nUse review helper.\n";
    let payload = InboundCommandPayload {
        command: "skill_install".to_string(),
        arguments: serde_json::json!({
            "name": "review-helper",
            "content": content,
        })
        .to_string(),
        trigger: ProductTriggerReason::BotCommand,
    };

    assert_eq!(
        ProductCommand::from_payload(&payload),
        ProductCommand::Lifecycle {
            action: LifecycleProductAction::SkillInstall {
                name: Some(LifecyclePackageId::new("review-helper").unwrap()),
                content: content.to_string(),
            },
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

#[test]
fn command_registry_declares_canonical_lifecycle_commands() {
    let names = product_command_descriptors()
        .map(|descriptor| descriptor.name)
        .collect::<Vec<_>>();

    for name in [
        "extension_search",
        "extension_install",
        "extension_auth",
        "extension_activate",
        "extension_configure",
        "extension_remove",
        "skill_search",
        "skill_install",
        "skill_remove",
    ] {
        assert!(names.contains(&name), "missing lifecycle command {name}");
    }
}
