use super::display_preview::{completed_preview_for_input, completed_preview_for_input_and_output};

#[tokio::test]
async fn third_party_trigger_suffix_keeps_its_custom_preview() {
    let preview = completed_preview_for_input_and_output(
        "acme__trigger_create",
        "acme.trigger_create",
        serde_json::json!({"name": "Third-party task"}),
        serde_json::json!({
            "trigger_id": "custom-visible-value",
            "schedule": "provider-defined"
        }),
    )
    .await;

    assert_eq!(preview.title, "acme__trigger_create");
    assert_ne!(preview.output_summary.as_deref(), Some("Routine created"));
    assert!(
        preview
            .output_preview
            .as_deref()
            .is_some_and(|output| output.contains("custom-visible-value")),
        "third-party results must remain on the generic projection path"
    );
}

#[tokio::test]
async fn trigger_create_preview_fails_closed_for_wrapped_internal_input() {
    let preview = completed_preview_for_input(
        "builtin__trigger_create",
        "builtin.trigger_create",
        serde_json::json!({
            "operation": "parse",
            "data": {
                "name": "Kansas Morning Weather",
                "prompt": "Call builtin.weather.lookup with an internal command",
                "schedule": {
                    "kind": "cron",
                    "expression": "0 8 * * *",
                    "timezone": "America/Chicago"
                }
            }
        }),
    )
    .await;

    assert_eq!(preview.title, "Routine");
    assert_eq!(
        preview.input_summary.as_deref(),
        Some("routine creation request")
    );
    assert_eq!(preview.output_summary.as_deref(), Some("Routine created"));
    let rendered = serde_json::to_string(&preview).expect("preview serializes");
    for internal in [
        "builtin.weather.lookup",
        "0 8 * * *",
        "America/Chicago",
        "operation",
        "prompt",
    ] {
        assert!(
            !rendered.contains(internal),
            "preview leaked {internal}: {rendered}"
        );
    }
}

#[tokio::test]
async fn trigger_create_preview_reads_only_allowlisted_top_level_display_fields() {
    let preview = completed_preview_for_input(
        "builtin__trigger_create",
        "builtin.trigger_create",
        serde_json::json!({
            "name": "Safe routine name",
            "prompt": "internal-prompt-secret",
            "trigger_id": "internal-trigger-id",
            "delivery_target_id": "internal-delivery-id",
            "action": {"capability": "internal.capability"},
            "schedule": {
                "kind": "cron",
                "expression": "internal-raw-cron",
                "timezone": "UTC",
                "metadata": {"secret": "internal-schedule-secret"}
            },
            "wrapper": {
                "name": "internal-wrapped-name",
                "timezone": "internal-wrapped-timezone"
            }
        }),
    )
    .await;

    assert_eq!(
        preview.input_summary.as_deref(),
        Some("routine: Safe routine name\nschedule: recurring\ntimezone: UTC")
    );
    assert_eq!(preview.subtitle.as_deref(), Some("Safe routine name"));
    let rendered = serde_json::to_string(&preview).expect("preview serializes");
    for internal in [
        "internal-prompt-secret",
        "internal-trigger-id",
        "internal-delivery-id",
        "internal.capability",
        "internal-raw-cron",
        "internal-schedule-secret",
        "internal-wrapped-name",
        "internal-wrapped-timezone",
    ] {
        assert!(
            !rendered.contains(internal),
            "preview leaked {internal}: {rendered}"
        );
    }
}

#[tokio::test]
async fn routine_preview_uses_handler_result_fields_and_bounds_list_output() {
    for (provider_name, capability_id, output_field, success_summary) in [
        (
            "builtin__trigger_remove",
            "builtin.trigger_remove",
            "removed",
            "Routine removed",
        ),
        (
            "builtin__trigger_pause",
            "builtin.trigger_pause",
            "updated",
            "Routine paused",
        ),
        (
            "builtin__trigger_resume",
            "builtin.trigger_resume",
            "updated",
            "Routine resumed",
        ),
    ] {
        let not_found = completed_preview_for_input_and_output(
            provider_name,
            capability_id,
            serde_json::json!({"trigger_id": "internal-id"}),
            serde_json::json!({(output_field): false}),
        )
        .await;
        assert_eq!(
            not_found.output_summary.as_deref(),
            Some("Routine not found")
        );

        let confirmed = completed_preview_for_input_and_output(
            provider_name,
            capability_id,
            serde_json::json!({"trigger_id": "internal-id"}),
            serde_json::json!({(output_field): true}),
        )
        .await;
        assert_eq!(confirmed.output_summary.as_deref(), Some(success_summary));

        for unavailable_output in [
            serde_json::json!({}),
            serde_json::json!({(output_field): "not-a-boolean"}),
        ] {
            let unavailable = completed_preview_for_input_and_output(
                provider_name,
                capability_id,
                serde_json::json!({"trigger_id": "internal-id"}),
                unavailable_output,
            )
            .await;
            assert_eq!(
                unavailable.output_summary.as_deref(),
                Some("Routine status unavailable")
            );
        }
    }

    let pause_with_remove_field = completed_preview_for_input_and_output(
        "builtin__trigger_pause",
        "builtin.trigger_pause",
        serde_json::json!({"trigger_id": "internal-id"}),
        serde_json::json!({"removed": false}),
    )
    .await;
    assert_eq!(
        pause_with_remove_field.output_summary.as_deref(),
        Some("Routine status unavailable"),
        "pause must fail closed when the handler's updated field is absent"
    );

    let triggers = (0..12)
        .map(|index| {
            serde_json::json!({
                "trigger_id": format!("internal-{index}"),
                "name": format!("Routine {index}"),
                "state": if index == 0 { "future_state" } else { "scheduled" },
                "schedule": { "kind": "cron", "expression": "0 8 * * *" }
            })
        })
        .collect::<Vec<_>>();
    let output = completed_preview_for_input_and_output(
        "builtin__trigger_list",
        "builtin.trigger_list",
        serde_json::json!({}),
        serde_json::json!({"triggers": triggers}),
    )
    .await;
    let preview = output
        .output_preview
        .as_deref()
        .expect("routine list preview");

    assert_eq!(output.output_summary.as_deref(), Some("Routines listed"));
    assert!(output.truncated);
    assert!(preview.contains("12 routines found"));
    assert!(preview.contains("Routine 0 — unknown, recurring"));
    assert!(preview.contains("Routine 9 — active, recurring"));
    assert!(!preview.contains("Routine 10 —"));
    assert!(preview.contains("Showing first 10 routines"));
    assert!(!preview.contains("internal-"));
    assert!(!preview.contains("0 8 * * *"));
}

#[tokio::test]
async fn routine_optional_display_fields_degrade_safely() {
    let list = completed_preview_for_input_and_output(
        "builtin__trigger_list",
        "builtin.trigger_list",
        serde_json::json!({}),
        serde_json::json!({
            "triggers": [{
                "name": "Future routine",
                "state": "future_state",
                "schedule": {"kind": "future_kind", "expression": "internal"}
            }]
        }),
    )
    .await;
    assert_eq!(
        list.output_preview.as_deref(),
        Some("1 routine found\nFuture routine — unknown, scheduled")
    );

    let create = completed_preview_for_input_and_output(
        "builtin__trigger_create",
        "builtin.trigger_create",
        serde_json::json!({"name": "Malformed timestamp"}),
        serde_json::json!({
            "trigger": {
                "name": "Malformed timestamp",
                "schedule": {"kind": "once"},
                "next_run_at": "not-a-timestamp"
            }
        }),
    )
    .await;
    let create_preview = create.output_preview.as_deref().expect("create preview");
    assert!(create_preview.contains("Schedule: one-time"));
    assert!(!create_preview.contains("Next run:"));
}
