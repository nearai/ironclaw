use super::{
    GeneratedSuggestion, GeneratedSuggestions, GenerationId, MAX_DESCRIPTION_LENGTH,
    MAX_ICON_LENGTH, MAX_PROMPT_LENGTH, MAX_SOURCE_LENGTH, MAX_SOURCES, MAX_SUGGESTIONS,
    MAX_TITLE_LENGTH, MIN_GENERATED_FIELD_LENGTH, MIN_SOURCES, MIN_SUGGESTIONS,
    SUGGESTIONS_OUTPUT_SCHEMA, finish_start_binding, generated_records, parse_suggestion_id,
    suggestion_start_reservation,
};
use crate::suggestions_store::{SuggestionBinding, SuggestionId, SuggestionsStoreError};
use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_host_api::turn::TurnRunId;
use ironclaw_product_contracts::surface::ProductSurfaceCaller;

fn valid_suggestion() -> GeneratedSuggestion {
    GeneratedSuggestion {
        title: "Review".to_string(),
        description: "Review the release".to_string(),
        suggested_prompt: "Review the release".to_string(),
        icon: "web".to_string(),
        sources: vec!["Web Search".to_string()],
    }
}

#[test]
fn start_reservation_keys_are_stable_across_target_contexts() {
    let tenant_id = TenantId::new("tenant").expect("valid tenant");
    let user_id = UserId::new("user").expect("valid user");
    let suggestion_id = SuggestionId::new("suggestion").expect("valid suggestion");
    let original = ProductSurfaceCaller::new(
        tenant_id.clone(),
        user_id.clone(),
        Some(AgentId::new("agent-a").expect("valid agent")),
        Some(ProjectId::new("project-a").expect("valid project")),
    );
    let changed = ProductSurfaceCaller::new(
        tenant_id,
        user_id,
        Some(AgentId::new("agent-b").expect("valid agent")),
        Some(ProjectId::new("project-b").expect("valid project")),
    );

    let original_reservation = suggestion_start_reservation(&original, &suggestion_id);
    let changed_reservation = suggestion_start_reservation(&changed, &suggestion_id);
    assert_eq!(
        original_reservation.thread_action_id,
        changed_reservation.thread_action_id
    );
    assert_eq!(
        original_reservation.turn_action_id,
        changed_reservation.turn_action_id
    );
    assert_ne!(original_reservation.agent_id, changed_reservation.agent_id);
    assert_ne!(
        original_reservation.project_id,
        changed_reservation.project_id
    );
}

#[test]
fn generated_prompts_reject_product_invalid_control_characters() {
    let generated = GeneratedSuggestions {
        suggestions: vec![GeneratedSuggestion {
            title: "Review".to_string(),
            description: "Review the release".to_string(),
            suggested_prompt: "Review\0the release".to_string(),
            icon: "generic".to_string(),
            sources: vec!["Web Search".to_string()],
        }],
    };

    assert!(
        generated_records(
            &GenerationId::new("generation").expect("valid id"),
            generated
        )
        .is_err()
    );
}

#[test]
fn generated_prompts_allow_message_newlines_and_tabs() {
    let generated = GeneratedSuggestions {
        suggestions: vec![GeneratedSuggestion {
            title: "Review".to_string(),
            description: "Review the release".to_string(),
            suggested_prompt: "Review:\n\t- release".to_string(),
            icon: "generic".to_string(),
            sources: vec!["Web Search".to_string()],
        }],
    };

    assert!(
        generated_records(
            &GenerationId::new("generation").expect("valid id"),
            generated
        )
        .is_ok()
    );
}

#[test]
fn generated_records_reject_whitespace_only_title_description_and_prompt() {
    for field in ["title", "description", "suggested_prompt"] {
        let mut suggestion = valid_suggestion();
        match field {
            "title" => suggestion.title = " \t\n ".to_string(),
            "description" => suggestion.description = " \t\n ".to_string(),
            "suggested_prompt" => suggestion.suggested_prompt = " \t\n ".to_string(),
            _ => unreachable!("test field is exhaustive"),
        }
        assert!(
            generated_records(
                &GenerationId::new("generation").expect("valid id"),
                GeneratedSuggestions {
                    suggestions: vec![suggestion],
                },
            )
            .is_err(),
            "whitespace-only {field} must be rejected"
        );
    }
}

#[test]
fn generated_records_reject_empty_and_oversized_lists() {
    assert!(
        generated_records(
            &GenerationId::new("generation").expect("valid id"),
            GeneratedSuggestions {
                suggestions: Vec::new(),
            },
        )
        .is_err()
    );

    let suggestions = (0..=MAX_SUGGESTIONS).map(|_| valid_suggestion()).collect();
    assert!(
        generated_records(
            &GenerationId::new("generation").expect("valid id"),
            GeneratedSuggestions { suggestions },
        )
        .is_err()
    );
}

#[test]
fn domain_bounds_match_the_canonical_output_schema() {
    let schema: serde_json::Value =
        serde_json::from_str(SUGGESTIONS_OUTPUT_SCHEMA).expect("canonical schema parses");
    let suggestions = &schema["properties"]["suggestions"];
    assert_eq!(suggestions["minItems"], serde_json::json!(MIN_SUGGESTIONS));
    assert_eq!(suggestions["maxItems"], serde_json::json!(MAX_SUGGESTIONS));

    for (field, max_length) in [
        ("title", MAX_TITLE_LENGTH),
        ("description", MAX_DESCRIPTION_LENGTH),
        ("suggested_prompt", MAX_PROMPT_LENGTH),
    ] {
        let field_schema = &suggestions["items"]["properties"][field];
        assert_eq!(
            field_schema["minLength"],
            serde_json::json!(MIN_GENERATED_FIELD_LENGTH)
        );
        assert_eq!(field_schema["maxLength"], serde_json::json!(max_length));
    }
    let item_schema = &suggestions["items"];
    assert_eq!(
        item_schema["properties"]["icon"]["type"],
        serde_json::json!("string")
    );
    assert_eq!(
        item_schema["properties"]["icon"]["enum"],
        serde_json::json!([
            "email",
            "calendar",
            "document",
            "storage",
            "spreadsheet",
            "presentation",
            "code",
            "messaging",
            "notes",
            "web",
            "memory",
            "generic"
        ]),
        "the schema owns one provider-neutral icon vocabulary with a generic fallback"
    );
    assert_eq!(
        item_schema["properties"]["sources"]["minItems"],
        serde_json::json!(MIN_SOURCES)
    );
    assert_eq!(
        item_schema["properties"]["sources"]["maxItems"],
        serde_json::json!(MAX_SOURCES)
    );
    assert_eq!(
        item_schema["properties"]["sources"]["items"]["maxLength"],
        serde_json::json!(MAX_SOURCE_LENGTH)
    );
    assert_eq!(
        item_schema["properties"]["sources"]["uniqueItems"],
        serde_json::json!(true)
    );
    assert!(item_schema["required"].as_array().is_some_and(|required| {
        required.iter().any(|name| name == "icon") && required.iter().any(|name| name == "sources")
    }));
}

#[test]
fn generated_records_reject_each_field_over_its_schema_limit() {
    let mut title = valid_suggestion();
    title.title = "t".repeat(MAX_TITLE_LENGTH + 1);
    assert!(
        generated_records(
            &GenerationId::new("generation").expect("valid id"),
            GeneratedSuggestions {
                suggestions: vec![title],
            },
        )
        .is_err()
    );

    let mut description = valid_suggestion();
    description.description = "d".repeat(MAX_DESCRIPTION_LENGTH + 1);
    assert!(
        generated_records(
            &GenerationId::new("generation").expect("valid id"),
            GeneratedSuggestions {
                suggestions: vec![description],
            },
        )
        .is_err()
    );

    let mut suggested_prompt = valid_suggestion();
    suggested_prompt.suggested_prompt = "p".repeat(MAX_PROMPT_LENGTH + 1);
    assert!(
        generated_records(
            &GenerationId::new("generation").expect("valid id"),
            GeneratedSuggestions {
                suggestions: vec![suggested_prompt],
            },
        )
        .is_err()
    );
}

#[test]
fn generated_records_accept_title_at_the_schema_bound() {
    let mut suggestion = valid_suggestion();
    suggestion.title = "t".repeat(MAX_TITLE_LENGTH);
    assert!(
        generated_records(
            &GenerationId::new("generation").expect("valid id"),
            GeneratedSuggestions {
                suggestions: vec![suggestion],
            },
        )
        .is_ok()
    );
}

#[test]
fn generated_records_accept_unicode_within_json_schema_code_point_limit() {
    let mut suggestion = valid_suggestion();
    suggestion.title = "é".repeat(MAX_TITLE_LENGTH);
    suggestion.description = "界".repeat(MAX_DESCRIPTION_LENGTH);
    suggestion.suggested_prompt = "🚀".repeat(MAX_PROMPT_LENGTH);
    assert!(
        generated_records(
            &GenerationId::new("generation").expect("valid id"),
            GeneratedSuggestions {
                suggestions: vec![suggestion],
            },
        )
        .is_ok()
    );
}

#[test]
fn generated_records_preserve_icon_and_sources_and_reject_invalid_values() {
    let records = generated_records(
        &GenerationId::new("generation").expect("valid id"),
        GeneratedSuggestions {
            suggestions: vec![valid_suggestion()],
        },
    )
    .expect("valid icon is accepted");
    assert_eq!(records[0].icon, "web");
    assert_eq!(records[0].sources, vec!["Web Search"]);

    // The provider-facing JSON Schema owns the supported icon vocabulary.
    // Persistence and wire types intentionally accept future schema values so
    // adding an icon does not require a backend type or storage migration.
    let mut future_icon = valid_suggestion();
    future_icon.icon = "calendar".to_string();
    let records = generated_records(
        &GenerationId::new("future-icon-generation").expect("valid id"),
        GeneratedSuggestions {
            suggestions: vec![future_icon],
        },
    )
    .expect("a future schema icon is accepted by generic backend validation");
    assert_eq!(records[0].icon, "calendar");

    for icon in [
        "".to_string(),
        "bad\nicon".to_string(),
        "x".repeat(MAX_ICON_LENGTH + 1),
    ] {
        let mut suggestion = valid_suggestion();
        suggestion.icon = icon;
        assert!(
            generated_records(
                &GenerationId::new("generation").expect("valid id"),
                GeneratedSuggestions {
                    suggestions: vec![suggestion],
                },
            )
            .is_err()
        );
    }

    for sources in [
        Vec::new(),
        vec![String::new()],
        vec!["Web Search".to_string(), "Web Search".to_string()],
        vec!["x".repeat(MAX_SOURCE_LENGTH + 1)],
        vec!["bad\nsource".to_string()],
    ] {
        let mut suggestion = valid_suggestion();
        suggestion.sources = sources;
        assert!(
            generated_records(
                &GenerationId::new("generation").expect("valid id"),
                GeneratedSuggestions {
                    suggestions: vec![suggestion],
                },
            )
            .is_err()
        );
    }
}

#[test]
fn suggestion_ids_are_canonicalized_before_store_lookup_and_wire_echo() {
    let parsed = parse_suggestion_id("550E8400-E29B-41D4-A716-446655440000")
        .expect("uppercase UUID is valid");
    assert_eq!(parsed.as_str(), "550e8400-e29b-41d4-a716-446655440000");
}

#[test]
fn accepted_start_binding_survives_replacement_generation_clearing_its_card() {
    let binding = SuggestionBinding {
        thread_id: ThreadId::new("thread-started").expect("valid thread id"),
        run_id: TurnRunId::new(),
    };
    let result = finish_start_binding(
        Err(SuggestionsStoreError::SuggestionNotFound {
            suggestion_id: crate::suggestions_store::SuggestionId::new("cleared-card")
                .expect("valid id"),
        }),
        binding.clone(),
    )
    .expect("replacement generation must not turn an accepted start into an error");
    assert_eq!(result, binding);
}
