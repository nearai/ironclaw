//! Contract tests for the Reborn-native product command model.

use ironclaw_assistant::{
    CommandAudience, LifecyclePackageId, LifecyclePackageKind, LifecyclePackageRef,
    LifecycleProductAction, ProductCommand, ProductModelCommand, ProductStopInvocation,
    declared_command_help_text, product_command_descriptors, required_audience,
    validate_declared_product_command,
};
use ironclaw_extension_contracts::channel_adapter::ProductTriggerReason;
use ironclaw_product_contracts::inbound::{InboundCommandPayload, ProductRejectionKind};

#[test]
fn command_payload_maps_to_typed_model_command_without_v1_parser() {
    let payload =
        InboundCommandPayload::new("model", "gpt-5-mini", ProductTriggerReason::BotCommand)
            .expect("valid command");

    assert_eq!(
        ProductCommand::from_payload(&payload).expect("parse model command"),
        ProductCommand::Model {
            action: ProductModelCommand::Set {
                model: "gpt-5-mini".to_string(),
            }
        }
    );
}

#[test]
fn model_command_maps_user_preference_actions() {
    let use_model =
        InboundCommandPayload::new("model", "use gpt-5-mini", ProductTriggerReason::BotCommand)
            .expect("valid command");
    let follow_default =
        InboundCommandPayload::new("model", "default", ProductTriggerReason::BotCommand)
            .expect("valid command");

    assert_eq!(
        ProductCommand::from_payload(&use_model).expect("parse preference command"),
        ProductCommand::Model {
            action: ProductModelCommand::Use {
                model: "gpt-5-mini".to_string(),
            }
        }
    );
    assert_eq!(
        ProductCommand::from_payload(&follow_default).expect("parse default command"),
        ProductCommand::Model {
            action: ProductModelCommand::Default,
        }
    );
}

#[test]
fn model_preference_commands_reject_missing_or_extra_arguments() {
    for arguments in ["use", "use model-a extra", "default extra"] {
        let payload =
            InboundCommandPayload::new("model", arguments, ProductTriggerReason::BotCommand)
                .expect("valid command");
        let rejection = ProductCommand::from_payload(&payload).expect_err("invalid command");
        assert_eq!(rejection.kind, ProductRejectionKind::InvalidRequest);
    }
}

#[test]
fn model_command_maps_provider_selection_without_cli_shelling_contract() {
    let payload = InboundCommandPayload::new(
        "model",
        "set-provider openai --model gpt-5-mini",
        ProductTriggerReason::BotCommand,
    )
    .expect("valid command");

    assert_eq!(
        ProductCommand::from_payload(&payload).expect("parse model provider command"),
        ProductCommand::Model {
            action: ProductModelCommand::SetProvider {
                provider: "openai".to_string(),
                model: Some("gpt-5-mini".to_string()),
            }
        }
    );
}

#[test]
fn model_provider_command_rejects_missing_provider() {
    let payload =
        InboundCommandPayload::new("model", "set-provider", ProductTriggerReason::BotCommand)
            .expect("valid command");

    let rejection = ProductCommand::from_payload(&payload).expect_err("missing provider");

    assert_eq!(rejection.kind, ProductRejectionKind::InvalidRequest);
}

#[test]
fn model_provider_command_rejects_unsupported_option() {
    let payload = InboundCommandPayload::new(
        "model",
        "set-provider openai --foo bar",
        ProductTriggerReason::BotCommand,
    )
    .expect("valid command");

    let rejection = ProductCommand::from_payload(&payload).expect_err("unsupported option");

    assert_eq!(rejection.kind, ProductRejectionKind::InvalidRequest);
}

#[test]
fn model_command_rejects_flag_shaped_model_name() {
    let payload = InboundCommandPayload::new("model", "--json", ProductTriggerReason::BotCommand)
        .expect("valid command");

    let rejection = ProductCommand::from_payload(&payload).expect_err("flag-shaped model");

    assert_eq!(rejection.kind, ProductRejectionKind::InvalidRequest);
}

#[test]
fn command_payload_maps_all_declared_commands_and_unknown_fallback() {
    let cases = [
        (
            "extension_install",
            "github",
            ProductCommand::Lifecycle {
                action: LifecycleProductAction::ExtensionInstall {
                    package_ref: ironclaw_assistant::LifecyclePackageRef::new(
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
        let command = ProductCommand::from_payload(&payload).expect("parse command");

        assert_eq!(command, expected);
        assert_eq!(command.name(), expected_name);
        assert_eq!(
            command.descriptor().map(|descriptor| descriptor.name),
            expected_descriptor
        );
    }
}

#[test]
fn new_stop_and_interrupt_are_explicit_user_commands() {
    let cases = [
        ("new", ProductCommand::New),
        (
            "stop",
            ProductCommand::Stop {
                invocation: ProductStopInvocation::Stop,
            },
        ),
        (
            "interrupt",
            ProductCommand::Stop {
                invocation: ProductStopInvocation::Interrupt,
            },
        ),
    ];

    for (name, expected) in cases {
        let payload = InboundCommandPayload::new(name, "", ProductTriggerReason::DirectChat)
            .expect("valid command payload");
        let command = ProductCommand::from_payload(&payload).expect("parse control command");
        assert_eq!(command, expected);
        assert_eq!(command.name(), name);
        assert_eq!(required_audience(&command), CommandAudience::User);
        assert_eq!(
            command.descriptor().map(|descriptor| descriptor.name),
            Some(name)
        );
        assert!(validate_declared_product_command(name).is_ok());
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
        ProductCommand::from_payload(&configure_payload).expect("parse configure command"),
        ProductCommand::Lifecycle {
            action: LifecycleProductAction::ExtensionConfigure {
                package_ref: ironclaw_assistant::LifecyclePackageRef::new(
                    LifecyclePackageKind::Extension,
                    "github",
                )
                .unwrap(),
                payload: Some(serde_json::json!({"mode": "dry_run"})),
            },
        }
    );

    for (command, expected_action) in [
        (
            "extension_install",
            LifecycleProductAction::ExtensionInstall {
                package_ref: ironclaw_assistant::LifecyclePackageRef::new(
                    LifecyclePackageKind::Extension,
                    "github",
                )
                .unwrap(),
            },
        ),
        (
            "extension_activate",
            LifecycleProductAction::ExtensionActivate {
                package_ref: ironclaw_assistant::LifecyclePackageRef::new(
                    LifecyclePackageKind::Extension,
                    "github",
                )
                .unwrap(),
            },
        ),
        (
            "extension_remove",
            LifecycleProductAction::ExtensionRemove {
                package_ref: ironclaw_assistant::LifecyclePackageRef::new(
                    LifecyclePackageKind::Extension,
                    "github",
                )
                .unwrap(),
            },
        ),
    ] {
        let payload = InboundCommandPayload::new(
            command,
            r#"{"extension_id":"github"}"#,
            ProductTriggerReason::BotCommand,
        )
        .expect("valid command payload");
        assert_eq!(
            ProductCommand::from_payload(&payload).expect("parse extension lifecycle command"),
            ProductCommand::Lifecycle {
                action: expected_action
            }
        );
    }

    for arguments in [r#"{"id":"review-helper"}"#, r#"{"name":"review-helper"}"#] {
        let payload =
            InboundCommandPayload::new("skill_remove", arguments, ProductTriggerReason::BotCommand)
                .expect("valid command payload");
        assert_eq!(
            ProductCommand::from_payload(&payload).expect("parse skill remove command"),
            ProductCommand::Lifecycle {
                action: LifecycleProductAction::SkillRemove {
                    package_ref: ironclaw_assistant::LifecyclePackageRef::new(
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
        let rejection = ProductCommand::from_payload(&payload).expect_err("invalid command");
        assert_eq!(rejection.kind, ProductRejectionKind::InvalidRequest);
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

        assert_eq!(
            ProductCommand::from_payload(&payload).expect("parse lifecycle command"),
            expected
        );
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

    let rejection = ProductCommand::from_payload(&payload).expect_err("invalid skill name");
    assert_eq!(rejection.kind, ProductRejectionKind::InvalidRequest);
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
        ProductCommand::from_payload(&payload).expect("parse skill install command"),
        ProductCommand::Lifecycle {
            action: LifecycleProductAction::SkillInstall {
                name: Some(LifecyclePackageId::new("review-helper").unwrap()),
                content: content.to_string(),
            },
        }
    );
}

#[test]
fn command_registry_declares_model_with_user_audience_and_metadata() {
    let model_descriptor = product_command_descriptors()
        .find(|descriptor| descriptor.name == "model")
        .expect("model descriptor");
    assert_eq!(
        model_descriptor.audience,
        CommandAudience::User,
        "model audience must be User"
    );
    assert_eq!(model_descriptor.title, "Model");
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

#[test]
fn declared_command_validation_uses_exact_registry_tokens() {
    assert!(validate_declared_product_command("status").is_ok());
    // Aliases are retired: `progress` is no longer a declarable token.
    assert!(validate_declared_product_command("progress").is_err());

    let error =
        validate_declared_product_command("notacommand").expect_err("unknown command must fail");
    assert_eq!(
        error.to_string(),
        "unknown product command `notacommand`",
        "startup diagnostics should identify the invalid manifest token"
    );
}

#[test]
fn declared_command_help_is_scoped_and_fail_closed() {
    assert_eq!(
        declared_command_help_text(["status"]),
        "Available commands:\n/status"
    );

    let empty = declared_command_help_text(std::iter::empty::<&str>());
    assert_eq!(empty, "Commands are not available in this channel.");
    assert!(!empty.contains("/model"));
    assert!(!empty.contains("/extension_configure"));
}

#[test]
fn listing_audience_is_user_for_model_and_status_and_admin_for_lifecycle() {
    for descriptor in product_command_descriptors() {
        let expected = match descriptor.name {
            "model" | "status" | "new" | "stop" | "interrupt" => CommandAudience::User,
            _ => CommandAudience::Admin, // the lifecycle family
        };
        assert_eq!(
            descriptor.audience, expected,
            "descriptor {}",
            descriptor.name
        );
    }
}

#[test]
fn execution_audience_is_per_action() {
    let user_cases = [
        ProductCommand::Status,
        ProductCommand::Model {
            action: ProductModelCommand::Status,
        },
        ProductCommand::Model {
            action: ProductModelCommand::Use { model: "m".into() },
        },
        ProductCommand::Model {
            action: ProductModelCommand::Default,
        },
        ProductCommand::Unknown {
            name: "nope".into(),
            arguments: String::new(),
        },
    ];
    for command in user_cases {
        assert_eq!(
            required_audience(&command),
            CommandAudience::User,
            "{command:?}"
        );
    }
    let admin_cases = [
        ProductCommand::Model {
            action: ProductModelCommand::Set { model: "m".into() },
        },
        ProductCommand::Model {
            action: ProductModelCommand::SetProvider {
                provider: "p".into(),
                model: None,
            },
        },
        ProductCommand::Lifecycle {
            action: LifecycleProductAction::ExtensionList,
        },
    ];
    for command in admin_cases {
        assert_eq!(
            required_audience(&command),
            CommandAudience::Admin,
            "{command:?}"
        );
    }
}

#[test]
fn every_descriptor_has_presentation_metadata() {
    for descriptor in product_command_descriptors() {
        assert!(
            !descriptor.title.is_empty(),
            "descriptor {} must have non-empty title",
            descriptor.name
        );
        assert!(
            !descriptor.description.is_empty(),
            "descriptor {} must have non-empty description",
            descriptor.name
        );
        assert!(
            !descriptor.usage.is_empty(),
            "descriptor {} must have non-empty usage",
            descriptor.name
        );
        assert!(
            descriptor
                .usage
                .starts_with(&format!("/{}", descriptor.name)),
            "descriptor {} usage must start with /{}, got: {}",
            descriptor.name,
            descriptor.name,
            descriptor.usage
        );
    }
}

#[test]
fn command_audience_serializes_snake_case() {
    assert_eq!(
        serde_json::to_string(&CommandAudience::User).unwrap(),
        "\"user\""
    );
    assert_eq!(
        serde_json::to_string(&CommandAudience::Admin).unwrap(),
        "\"admin\""
    );
}
