use std::sync::Arc;

use chrono::TimeZone;
use ironclaw_filesystem::{CasExpectation, InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::{
    ids::{AgentId, ProjectId, TenantId, UserId},
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, VirtualPath},
    resource::ResourceScope,
};

use super::*;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 17, 0, 0, 0)
        .single()
        .expect("valid test timestamp")
}

fn lease_expiry() -> DateTime<Utc> {
    now() + chrono::Duration::seconds(30)
}

fn generation_id(value: &str) -> GenerationId {
    GenerationId::new(value).expect("valid generation id")
}

fn suggestion_id(value: &str) -> SuggestionId {
    SuggestionId::new(value).expect("valid suggestion id")
}

fn generation_request(
    generation_name: &str,
    lease_owner: &str,
    lease_expires_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> BeginGenerationRequest {
    BeginGenerationRequest {
        generation_id: generation_id(generation_name),
        public_id: format!("public-{generation_name}"),
        accept_key: format!("accept-{generation_name}"),
        client_action_id: Some(format!("action-{generation_name}")),
        prompt_schema_version: 1,
        lease_owner: lease_owner.to_string(),
        lease_expires_at,
        now,
    }
}

fn scope() -> ResourceScope {
    ResourceScope::local_default(
        UserId::new("suggestions-test-user").expect("valid test user"),
        ironclaw_host_api::ids::InvocationId::new(),
    )
    .expect("valid test scope")
}

#[test]
fn document_path_is_stable_for_tenant_and_user_scope() {
    let scope = ResourceScope {
        tenant_id: TenantId::new("tenant-store").expect("valid tenant"),
        user_id: UserId::new("user-store").expect("valid user"),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: ironclaw_host_api::ids::InvocationId::new(),
    };

    assert_eq!(
        document_path(&scope).expect("valid document path").as_str(),
        "/suggestions/contexts/935ee36455821846b6581f37b908afd5d19507c558cdc817cf403efe8f594c69/doc.json"
    );
}

fn context_scope(agent_id: &str, project_id: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("suggestions-test-tenant").expect("valid tenant"),
        user_id: UserId::new("suggestions-test-user").expect("valid user"),
        agent_id: Some(AgentId::new(agent_id).expect("valid agent")),
        project_id: Some(ProjectId::new(project_id).expect("valid project")),
        mission_id: None,
        thread_id: None,
        invocation_id: ironclaw_host_api::ids::InvocationId::new(),
    }
}

fn store() -> FilesystemSuggestionsStore<InMemoryBackend> {
    let backend = Arc::new(InMemoryBackend::new());
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/suggestions").expect("valid mount alias"),
        VirtualPath::new("/tenants/test/users/test/suggestions").expect("valid mount root"),
        MountPermissions::read_write(),
    )])
    .expect("valid mount view");
    FilesystemSuggestionsStore::new(Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts)))
}

fn card(id: &str) -> SuggestionRecord {
    SuggestionRecord {
        id: suggestion_id(id),
        title: "Title".to_string(),
        description: "Description".to_string(),
        suggested_prompt: "Do the thing".to_string(),
        icon: "generic".to_string(),
        sources: vec!["Gmail".to_string()],
        generation_id: generation_id("generation-a"),
        created_at: now(),
        updated_at: now(),
        dismissed_at: None,
        start_reservation: None,
        binding: None,
    }
}

#[test]
fn internal_ids_keep_the_persisted_json_string_shape() {
    let document = SuggestionDocument {
        schema_version: SUGGESTION_DOCUMENT_SCHEMA_VERSION,
        generation: GenerationState::Ready {
            generation_id: generation_id("generation-a"),
            completed_at: now(),
            client_action_id: None,
        },
        generation_history: vec![GenerationHistoryEntry {
            generation_id: generation_id("legacy-generation"),
            // Legacy terminal records may not have had a client key.  The
            // terminal state still belongs in durable history so migrations
            // and audit reads do not silently lose it.
            client_action_id: None,
            state: GenerationHistoryState::Failed {
                failed_at: now(),
                reason: "legacy failure".to_string(),
            },
        }],
        suggestions: vec![card("card-a")],
        created_at: now(),
        updated_at: now(),
    };

    let json = serde_json::to_value(&document).expect("document serializes");
    assert_eq!(json["generation"]["ready"]["generation_id"], "generation-a");
    assert_eq!(json["suggestions"][0]["id"], "card-a");
    assert_eq!(json["suggestions"][0]["generation_id"], "generation-a");
    let decoded: SuggestionDocument = serde_json::from_value(json).expect("string ids deserialize");
    assert_eq!(decoded, document);
}

#[tokio::test]
async fn replacement_archives_terminal_state_without_action_key() {
    let store = store();
    let document = SuggestionDocument {
        schema_version: SUGGESTION_DOCUMENT_SCHEMA_VERSION,
        generation: GenerationState::Ready {
            generation_id: generation_id("generation-a"),
            completed_at: now(),
            client_action_id: None,
        },
        generation_history: Vec::new(),
        suggestions: vec![card("card-a")],
        created_at: now(),
        updated_at: now(),
    };
    store
        .filesystem
        .put(
            &scope(),
            &document_path(&scope()).expect("valid document path"),
            document_entry(&document).expect("valid document entry"),
            CasExpectation::Absent,
        )
        .await
        .expect("seed ready document");

    let mut replacement = generation_request("generation-b", "owner-b", lease_expiry(), now());
    replacement.client_action_id = None;
    let claim = store
        .begin_generation(&scope(), replacement)
        .await
        .expect("replacement generation begins");
    assert!(claim.is_acquired());
    let current = store
        .read(&scope())
        .await
        .expect("document reads")
        .expect("document exists");
    assert_eq!(current.generation_history.len(), 1);
    assert_eq!(
        current.generation_history[0].generation_id,
        generation_id("generation-a")
    );
    assert!(current.generation_history[0].client_action_id.is_none());
    assert!(matches!(
        current.generation_history[0].state,
        GenerationHistoryState::Ready { completed_at } if completed_at == now()
    ));
}

#[test]
fn persisted_cards_require_icon_and_sources() {
    let document = SuggestionDocument {
        schema_version: SUGGESTION_DOCUMENT_SCHEMA_VERSION,
        generation: GenerationState::Ready {
            generation_id: generation_id("generation-a"),
            completed_at: now(),
            client_action_id: None,
        },
        generation_history: Vec::new(),
        suggestions: vec![card("card-a")],
        created_at: now(),
        updated_at: now(),
    };

    for field in ["icon", "sources"] {
        let mut json = serde_json::to_value(&document).expect("document serializes");
        json["suggestions"][0]
            .as_object_mut()
            .expect("suggestion object")
            .remove(field);
        assert!(serde_json::from_value::<SuggestionDocument>(json).is_err());
    }
}

#[test]
fn internal_ids_reject_invalid_persisted_strings() {
    for value in [serde_json::json!(""), serde_json::json!("bad\nvalue")] {
        assert!(serde_json::from_value::<SuggestionId>(value.clone()).is_err());
        assert!(serde_json::from_value::<GenerationId>(value).is_err());
    }

    let too_long = serde_json::json!("x".repeat(129));
    assert!(serde_json::from_value::<SuggestionId>(too_long.clone()).is_err());
    assert!(serde_json::from_value::<GenerationId>(too_long).is_err());
}

#[test]
fn pending_state_reads_legacy_unused_fields_without_reemitting_them() {
    let run_id = TurnRunId::new();
    let legacy = serde_json::json!({
        "schema_version": SUGGESTION_DOCUMENT_SCHEMA_VERSION,
        "generation": {
            "pending": {
                "generation_id": "generation-a",
                "public_id": "public-generation-a",
                "accept_key": "accept-generation-a",
                "client_action_id": "action-generation-a",
                "prompt_schema_version": 1,
                "run_id": run_id,
            }
        },
        "suggestions": [],
        "created_at": now(),
        "updated_at": now(),
    });

    let decoded: SuggestionDocument =
        serde_json::from_value(legacy).expect("legacy pending state deserializes");
    assert_eq!(
        decoded.generation,
        GenerationState::Pending {
            generation_id: generation_id("generation-a"),
            public_id: "public-generation-a".to_string(),
            client_action_id: Some(
                GenerationActionId::new("action-generation-a").expect("valid action id"),
            ),
            run_id,
        }
    );

    let current = serde_json::to_value(decoded).expect("pending state serializes");
    assert!(current["generation"]["pending"].get("accept_key").is_none());
    assert!(
        current["generation"]["pending"]
            .get("prompt_schema_version")
            .is_none()
    );
}

async fn ready_store(store: &FilesystemSuggestionsStore<InMemoryBackend>) {
    ready_store_at(store, &scope()).await;
}

async fn ready_store_at(
    store: &FilesystemSuggestionsStore<InMemoryBackend>,
    target_scope: &ResourceScope,
) {
    let state = store
        .begin_generation(
            target_scope,
            generation_request("generation-a", "owner-a", lease_expiry(), now()),
        )
        .await
        .expect("generation begins");
    assert!(state.is_acquired());
    assert!(matches!(state.state(), GenerationState::Generating { .. }));
    let run_id = TurnRunId::new();
    store
        .bind_generation_run(
            target_scope,
            &generation_id("generation-a"),
            "owner-a",
            run_id,
            now(),
        )
        .await
        .expect("generation run binds");
    store
        .complete_generation_for_run(
            target_scope,
            "public-generation-a",
            run_id,
            vec![card("card-a")],
            now(),
        )
        .await
        .expect("generation completes");
}

#[tokio::test]
async fn concurrent_begin_converges_on_one_generation() {
    let store = Arc::new(store());
    let first = {
        let store = Arc::clone(&store);
        tokio::spawn(async move {
            store
                .begin_generation(
                    &scope(),
                    generation_request("generation-a", "owner-a", lease_expiry(), now()),
                )
                .await
        })
    };
    let second = {
        let store = Arc::clone(&store);
        tokio::spawn(async move {
            let mut request = generation_request("generation-b", "owner-b", lease_expiry(), now());
            request.client_action_id = Some("action-generation-a".to_string());
            store.begin_generation(&scope(), request).await
        })
    };
    let first = first
        .await
        .expect("first task joins")
        .expect("first begin succeeds");
    let second = second
        .await
        .expect("second task joins")
        .expect("second begin succeeds");
    assert_eq!(first.state(), second.state());
    assert_ne!(first.is_acquired(), second.is_acquired());
    assert!(matches!(first.state(), GenerationState::Generating { .. }));
}

#[tokio::test]
async fn expired_request_lease_replays_the_original_accept_identity_before_run_binding() {
    let store = store();
    let first = store
        .begin_generation(
            &scope(),
            generation_request("generation-a", "owner-a", now(), now()),
        )
        .await
        .expect("first lease is created");
    assert!(first.is_acquired());

    let takeover_at = now() + chrono::Duration::seconds(1);
    let reclaimed = store
        .begin_generation(
            &scope(),
            generation_request(
                "generation-b",
                "owner-b",
                takeover_at + chrono::Duration::seconds(30),
                takeover_at,
            ),
        )
        .await
        .expect("expired pre-submit claim is recoverable");
    assert!(reclaimed.is_acquired());
    assert!(matches!(
        reclaimed.state(),
        GenerationState::Generating {
            generation_id: current_generation_id,
            public_id,
            accept_key,
            client_action_id,
            lease_owner,
            lease_expires_at,
            ..
        } if current_generation_id == &generation_id("generation-a")
            && public_id == "public-generation-a"
            && accept_key == "accept-generation-a"
            && client_action_id
                .as_ref()
                .is_some_and(|id| id.as_str() == "action-generation-a")
            && lease_owner == "owner-b"
            && *lease_expires_at == takeover_at + chrono::Duration::seconds(30)
    ));
    let accepted_run_id = TurnRunId::new();
    let pending = store
        .bind_generation_run(
            &scope(),
            &generation_id("generation-a"),
            "owner-b",
            accepted_run_id,
            takeover_at,
        )
        .await
        .expect("the recovered owner can bind the replayed accepted run");
    assert!(matches!(
        pending.generation,
        GenerationState::Pending {
            ref public_id,
            run_id,
            ..
        } if public_id == "public-generation-a" && run_id == accepted_run_id
    ));
    let settled = store
        .complete_generation_for_run(
            &scope(),
            "public-generation-a",
            accepted_run_id,
            vec![card("recovered-card")],
            takeover_at,
        )
        .await
        .expect("the original public id still settles after recovery")
        .expect("the recovered generation remains current");
    assert!(matches!(
        settled.generation,
        GenerationState::Ready {
            generation_id: ref current_generation_id,
            ..
        } if current_generation_id == &generation_id("generation-a")
    ));
    assert_eq!(settled.suggestions[0].id, suggestion_id("recovered-card"));
}

#[tokio::test]
async fn current_owner_can_fail_generation() {
    let store = store();
    store
        .begin_generation(
            &scope(),
            generation_request("generation-a", "owner-a", lease_expiry(), now()),
        )
        .await
        .expect("generation begins");

    let failed = store
        .fail_generation(
            &scope(),
            &generation_id("generation-a"),
            "owner-a",
            "provider failed".to_string(),
            now(),
        )
        .await
        .expect("failure records")
        .expect("current failure returns document");
    assert!(matches!(
        failed.generation,
        GenerationState::Failed {
            ref generation_id,
            ref reason,
            ..
        } if generation_id == &GenerationId::new("generation-a").expect("valid generation id") && reason == "provider failed"
    ));
}

#[tokio::test]
async fn failure_reasons_are_bounded_and_control_free() {
    let store = store();
    store
        .begin_generation(
            &scope(),
            generation_request("generation-a", "owner-a", lease_expiry(), now()),
        )
        .await
        .expect("generation begins");

    let failed = store
        .fail_generation(
            &scope(),
            &generation_id("generation-a"),
            "owner-a",
            format!("{}\n", "x".repeat(200)),
            now(),
        )
        .await
        .expect("failure records")
        .expect("current failure returns document");
    let GenerationState::Failed { reason, .. } = failed.generation else {
        panic!("failure must settle generation");
    };
    assert_eq!(reason.chars().count(), MAX_FAILURE_REASON_CHARS);
    assert!(reason.chars().all(|character| !character.is_control()));
}

#[tokio::test]
async fn failed_generation_can_be_reclaimed() {
    let store = store();
    store
        .begin_generation(
            &scope(),
            generation_request("generation-a", "owner-a", lease_expiry(), now()),
        )
        .await
        .expect("generation begins");
    store
        .fail_generation(
            &scope(),
            &generation_id("generation-a"),
            "owner-a",
            "provider failed".to_string(),
            now(),
        )
        .await
        .expect("failure records");

    let reclaimed = store
        .begin_generation(
            &scope(),
            generation_request(
                "generation-b",
                "owner-b",
                now() + chrono::Duration::seconds(30),
                now() + chrono::Duration::seconds(1),
            ),
        )
        .await
        .expect("failed generation is reclaimable");
    assert!(reclaimed.is_acquired());
    assert!(matches!(
        reclaimed.state(),
        GenerationState::Generating {
            generation_id,
            lease_owner,
            ..
        } if generation_id == &GenerationId::new("generation-b").expect("valid generation id") && lease_owner == "owner-b"
    ));
}

#[tokio::test]
async fn completion_retains_old_card_metadata_while_appending_current_cards() {
    let store = store();
    let mut retained = card("old-card");
    retained.generation_id = generation_id("generation-b");
    retained.dismissed_at = Some(now());
    let retained_binding = SuggestionBinding {
        thread_id: ThreadId::new("thread-retained").expect("valid thread"),
        run_id: TurnRunId::new(),
    };
    retained.binding = Some(retained_binding.clone());
    let document = SuggestionDocument {
        schema_version: SUGGESTION_DOCUMENT_SCHEMA_VERSION,
        generation: GenerationState::Generating {
            generation_id: generation_id("generation-b"),
            public_id: "public-b".to_string(),
            accept_key: "accept-b".to_string(),
            client_action_id: Some(
                GenerationActionId::new("action-generation-b").expect("valid action id"),
            ),
            prompt_schema_version: 1,
            run_id: None,
            lease_owner: "owner-b".to_string(),
            lease_expires_at: lease_expiry(),
        },
        generation_history: Vec::new(),
        suggestions: vec![retained],
        created_at: now(),
        updated_at: now(),
    };
    store
        .filesystem
        .put(
            &scope(),
            &document_path(&scope()).expect("valid document path"),
            document_entry(&document).expect("valid document entry"),
            CasExpectation::Absent,
        )
        .await
        .expect("seeds generating document");

    let mut generated = card("card-a");
    generated.generation_id = generation_id("generation-b");
    let run_id = TurnRunId::new();
    store
        .bind_generation_run(
            &scope(),
            &generation_id("generation-b"),
            "owner-b",
            run_id,
            now(),
        )
        .await
        .expect("accepted run binds");
    let document = store
        .complete_generation_for_run(&scope(), "public-b", run_id, vec![generated], now())
        .await
        .expect("completion succeeds")
        .expect("current completion persists");
    assert_eq!(document.suggestions.len(), 2);
    assert!(document.suggestions[0].dismissed_at.is_some());
    assert_eq!(document.suggestions[0].binding, Some(retained_binding));
    assert!(document.suggestions[1].dismissed_at.is_none());
    assert!(document.suggestions[1].binding.is_none());
}

#[tokio::test]
async fn start_reservation_rejects_a_card_from_a_stale_generation() {
    let store = store();
    let document = SuggestionDocument {
        schema_version: SUGGESTION_DOCUMENT_SCHEMA_VERSION,
        generation: GenerationState::Ready {
            generation_id: generation_id("generation-b"),
            completed_at: now(),
            client_action_id: None,
        },
        generation_history: Vec::new(),
        suggestions: vec![card("stale-card")],
        created_at: now(),
        updated_at: now(),
    };
    store
        .filesystem
        .put(
            &scope(),
            &document_path(&scope()).expect("path"),
            document_entry(&document).expect("entry"),
            CasExpectation::Absent,
        )
        .await
        .expect("seed");
    let error = store
        .reserve_start(
            &scope(),
            &suggestion_id("stale-card"),
            SuggestionStartReservation {
                thread_action_id: "thread-action".to_string(),
                turn_action_id: "turn-action".to_string(),
                agent_id: None,
                project_id: None,
            },
            now(),
        )
        .await
        .expect_err("stale generation must fail inside the CAS mutation");
    assert!(matches!(
        error,
        SuggestionsStoreError::SuggestionNotFound { .. }
    ));
}

#[tokio::test]
async fn documents_are_shared_across_agent_and_project_context_for_same_user() {
    let store = store();
    let scope_a = context_scope("agent-a", "project-a");
    let scope_b = context_scope("agent-b", "project-a");
    let scope_c = context_scope("agent-a", "project-b");

    store
        .begin_generation(
            &scope_a,
            generation_request("generation-a", "owner-a", lease_expiry(), now()),
        )
        .await
        .expect("first context generation begins");

    assert!(
        store
            .read(&scope_b)
            .await
            .expect("agent-b read succeeds")
            .is_some()
    );
    assert!(
        store
            .read(&scope_c)
            .await
            .expect("project-b read succeeds")
            .is_some()
    );
}

#[tokio::test]
async fn new_generation_replaces_visible_cards_replays_and_conflicts_by_action_key() {
    let store = store();
    ready_store(&store).await;

    let first = store
        .begin_generation(
            &scope(),
            BeginGenerationRequest {
                generation_id: generation_id("generation-b"),
                public_id: "public-generation-b".to_string(),
                accept_key: "accept-generation-b".to_string(),
                client_action_id: Some("new-action".to_string()),
                prompt_schema_version: 1,
                lease_owner: "owner-b".to_string(),
                lease_expires_at: lease_expiry(),
                now: now(),
            },
        )
        .await
        .expect("new generation begins");
    assert!(first.is_acquired());
    assert!(matches!(first.state(), GenerationState::Generating { .. }));
    assert_eq!(
        store
            .read(&scope())
            .await
            .expect("read succeeds")
            .expect("document exists")
            .suggestions
            .len(),
        1
    );

    let replay = store
        .begin_generation(
            &scope(),
            BeginGenerationRequest {
                generation_id: generation_id("replayed-generation"),
                public_id: "replayed-public".to_string(),
                accept_key: "replayed-accept".to_string(),
                client_action_id: Some("new-action".to_string()),
                prompt_schema_version: 1,
                lease_owner: "replay-owner".to_string(),
                lease_expires_at: lease_expiry(),
                now: now(),
            },
        )
        .await
        .expect("same action replays");
    assert!(!replay.is_acquired());
    assert_eq!(replay.state(), first.state());

    let conflict = store
        .begin_generation(
            &scope(),
            BeginGenerationRequest {
                generation_id: generation_id("conflicting-generation"),
                public_id: "conflicting-public".to_string(),
                accept_key: "conflicting-accept".to_string(),
                client_action_id: Some("other-action".to_string()),
                prompt_schema_version: 1,
                lease_owner: "owner-c".to_string(),
                lease_expires_at: lease_expiry(),
                now: now(),
            },
        )
        .await
        .expect_err("different action cannot replace a running generation");
    assert!(matches!(
        conflict,
        SuggestionsStoreError::GenerationInProgress {
            generation_id: existing_generation,
            ..
        } if existing_generation == generation_id("generation-b")
    ));
}

#[tokio::test]
async fn keyless_request_does_not_replay_an_active_keyless_generation() {
    let store = store();
    let mut first = generation_request("generation-a", "owner-a", lease_expiry(), now());
    first.client_action_id = None;
    store
        .begin_generation(&scope(), first)
        .await
        .expect("first keyless generation begins");

    let mut second = generation_request("generation-b", "owner-b", lease_expiry(), now());
    second.client_action_id = None;
    let error = store
        .begin_generation(&scope(), second)
        .await
        .expect_err("a keyless caller cannot replay another keyless request");

    assert!(matches!(
        error,
        SuggestionsStoreError::GenerationInProgress {
            generation_id: existing_generation,
            client_action_id: None,
        } if existing_generation == generation_id("generation-a")
    ));
}

#[tokio::test]
async fn completed_action_replays_after_replacement_without_acquiring_or_hiding_current() {
    let store = store();
    ready_store(&store).await;

    let generation_b = generation_id("generation-b");
    store
        .begin_generation(
            &scope(),
            BeginGenerationRequest {
                generation_id: generation_b.clone(),
                public_id: "public-generation-b".to_string(),
                accept_key: "accept-generation-b".to_string(),
                client_action_id: Some("action-generation-b".to_string()),
                prompt_schema_version: 1,
                lease_owner: "owner-b".to_string(),
                lease_expires_at: lease_expiry(),
                now: now(),
            },
        )
        .await
        .expect("replacement generation begins");
    let run_id = TurnRunId::new();
    store
        .bind_generation_run(&scope(), &generation_b, "owner-b", run_id, now())
        .await
        .expect("replacement run binds");
    let mut replacement = card("card-b");
    replacement.generation_id = generation_b;
    store
        .complete_generation_for_run(
            &scope(),
            "public-generation-b",
            run_id,
            vec![replacement],
            now(),
        )
        .await
        .expect("replacement generation completes");

    let replay = store
        .begin_generation(
            &scope(),
            BeginGenerationRequest {
                generation_id: generation_id("late-generation-a-retry"),
                public_id: "late-public-generation-a-retry".to_string(),
                accept_key: "late-accept-generation-a-retry".to_string(),
                client_action_id: Some("action-generation-a".to_string()),
                prompt_schema_version: 1,
                lease_owner: "late-owner".to_string(),
                lease_expires_at: lease_expiry(),
                now: now(),
            },
        )
        .await
        .expect("completed action history replays");
    assert!(!replay.is_acquired());
    assert!(replay.is_historical_replay());
    assert!(matches!(
        replay.state(),
        GenerationState::Ready {
            generation_id: current_generation_id,
            client_action_id,
            ..
        } if current_generation_id == &generation_id("generation-a")
            && client_action_id.as_ref().is_some_and(|id| id.as_str() == "action-generation-a")
    ));

    let current = store
        .read(&scope())
        .await
        .expect("current document reads")
        .expect("current document exists");
    assert!(matches!(
        current.generation,
        GenerationState::Ready {
            generation_id: ref current_generation_id,
            ..
        } if current_generation_id == &generation_id("generation-b")
    ));
    assert_eq!(current.suggestions.len(), 2);
    assert_eq!(current.generation_history.len(), 1);
}

#[tokio::test]
async fn historical_cards_are_retained_but_cannot_be_dismissed_or_started() {
    let store = store();
    ready_store(&store).await;

    let generation_b = generation_id("generation-b");
    store
        .begin_generation(
            &scope(),
            BeginGenerationRequest {
                generation_id: generation_b.clone(),
                public_id: "public-generation-b".to_string(),
                accept_key: "accept-generation-b".to_string(),
                client_action_id: Some("action-generation-b".to_string()),
                prompt_schema_version: 1,
                lease_owner: "owner-b".to_string(),
                lease_expires_at: lease_expiry(),
                now: now(),
            },
        )
        .await
        .expect("replacement generation begins");
    let run_id = TurnRunId::new();
    store
        .bind_generation_run(&scope(), &generation_b, "owner-b", run_id, now())
        .await
        .expect("replacement run binds");
    let mut replacement = card("card-b");
    replacement.generation_id = generation_b;
    store
        .complete_generation_for_run(
            &scope(),
            "public-generation-b",
            run_id,
            vec![replacement],
            now(),
        )
        .await
        .expect("replacement generation completes");

    let dismiss_error = store
        .dismiss(&scope(), &suggestion_id("card-a"), now())
        .await
        .expect_err("historical cards cannot be newly dismissed");
    assert!(matches!(
        dismiss_error,
        SuggestionsStoreError::SuggestionNotFound { .. }
    ));
    let start_error = store
        .reserve_start(
            &scope(),
            &suggestion_id("card-a"),
            SuggestionStartReservation {
                thread_action_id: "historical-thread".to_string(),
                turn_action_id: "historical-turn".to_string(),
                agent_id: None,
                project_id: None,
            },
            now(),
        )
        .await
        .expect_err("historical cards cannot be newly started");
    assert!(matches!(
        start_error,
        SuggestionsStoreError::SuggestionNotFound { .. }
    ));
}

#[tokio::test]
async fn bind_generation_run_rejects_a_conflicting_run_id() {
    let store = store();
    let generation = generation_id("generation-b");
    store
        .begin_generation(
            &scope(),
            BeginGenerationRequest {
                generation_id: generation.clone(),
                public_id: "public-generation-b".to_string(),
                accept_key: "accept-generation-b".to_string(),
                client_action_id: Some("new-action".to_string()),
                prompt_schema_version: 1,
                lease_owner: "owner-b".to_string(),
                lease_expires_at: lease_expiry(),
                now: now(),
            },
        )
        .await
        .expect("generation begins");

    let first_run = TurnRunId::new();
    store
        .bind_generation_run(&scope(), &generation, "owner-b", first_run, now())
        .await
        .expect("first run binds");

    let conflict = store
        .bind_generation_run(&scope(), &generation, "owner-b", TurnRunId::new(), now())
        .await
        .expect_err("a generation cannot be claimed by a second run");
    assert!(matches!(
        conflict,
        SuggestionsStoreError::GenerationNotCurrent { generation_id }
            if generation_id == generation
    ));
}

#[tokio::test]
async fn submitted_generation_settles_by_public_and_typed_run_identity() {
    let store = store();
    ready_store(&store).await;
    let generation = generation_id("generation-b");
    store
        .begin_generation(
            &scope(),
            BeginGenerationRequest {
                generation_id: generation.clone(),
                public_id: "public-generation-b".to_string(),
                accept_key: "accept-generation-b".to_string(),
                client_action_id: Some("new-action".to_string()),
                prompt_schema_version: 1,
                lease_owner: "owner-b".to_string(),
                lease_expires_at: lease_expiry(),
                now: now(),
            },
        )
        .await
        .expect("generation begins");
    let run_id = TurnRunId::new();
    let pending = store
        .bind_generation_run(&scope(), &generation, "owner-b", run_id, now())
        .await
        .expect("accepted run binds");
    assert!(
        matches!(pending.generation, GenerationState::Pending { run_id: bound, .. } if bound == run_id)
    );

    let mut replacement = card("card-b");
    replacement.generation_id = generation.clone();
    let completed = store
        .complete_generation_for_run(
            &scope(),
            "public-generation-b",
            run_id,
            vec![replacement],
            now(),
        )
        .await
        .expect("matching run settles")
        .expect("matching run updates document");
    assert!(matches!(
        completed.generation,
        GenerationState::Ready { .. }
    ));
    assert_eq!(completed.suggestions.len(), 2);
    assert_eq!(completed.suggestions[1].id, suggestion_id("card-b"));

    let stale = store
        .complete_generation_for_run(
            &scope(),
            "public-generation-b",
            TurnRunId::new(),
            vec![card("stale")],
            now() + chrono::Duration::days(2),
        )
        .await
        .expect("stale settlement is ignored");
    assert!(stale.is_none());
}

#[tokio::test]
async fn terminal_run_can_settle_before_request_binds_its_run_id() {
    let store = store();
    ready_store(&store).await;
    let generation = generation_id("generation-b");
    store
        .begin_generation(
            &scope(),
            BeginGenerationRequest {
                generation_id: generation.clone(),
                public_id: "public-generation-b".to_string(),
                accept_key: "accept-generation-b".to_string(),
                client_action_id: Some("new-action".to_string()),
                prompt_schema_version: 1,
                lease_owner: "owner-b".to_string(),
                lease_expires_at: lease_expiry(),
                now: now(),
            },
        )
        .await
        .expect("generation begins");
    let run_id = TurnRunId::new();
    let mut replacement = card("card-b");
    replacement.generation_id = generation;

    let completed = store
        .complete_generation_for_run(
            &scope(),
            "public-generation-b",
            run_id,
            vec![replacement],
            now(),
        )
        .await
        .expect("matching terminal run settles");
    assert!(matches!(
        completed.map(|document| document.generation),
        Some(GenerationState::Ready { .. })
    ));
}

#[tokio::test]
async fn submitted_generation_failure_retains_cards_and_requires_matching_run() {
    let store = store();
    ready_store(&store).await;
    let generation = generation_id("generation-b");
    store
        .begin_generation(
            &scope(),
            BeginGenerationRequest {
                generation_id: generation.clone(),
                public_id: "public-generation-b".to_string(),
                accept_key: "accept-generation-b".to_string(),
                client_action_id: Some("new-action".to_string()),
                prompt_schema_version: 1,
                lease_owner: "owner-b".to_string(),
                lease_expires_at: lease_expiry(),
                now: now(),
            },
        )
        .await
        .expect("generation begins");
    let run_id = TurnRunId::new();
    store
        .bind_generation_run(&scope(), &generation, "owner-b", run_id, now())
        .await
        .expect("accepted run binds");

    let failed = store
        .fail_generation_for_run(
            &scope(),
            "public-generation-b",
            run_id,
            "provider failed".to_string(),
            now(),
        )
        .await
        .expect("matching run failure settles")
        .expect("matching run updates document");
    assert!(matches!(failed.generation, GenerationState::Failed { .. }));
    assert_eq!(failed.suggestions.len(), 1);

    let stale = store
        .fail_generation_for_run(
            &scope(),
            "public-generation-b",
            TurnRunId::new(),
            "stale failure".to_string(),
            now(),
        )
        .await
        .expect("stale failure is ignored");
    assert!(stale.is_none());
}

#[tokio::test]
async fn terminal_failure_can_settle_before_request_binds_its_run_id() {
    let store = store();
    ready_store(&store).await;
    let generation = generation_id("generation-b");
    store
        .begin_generation(
            &scope(),
            BeginGenerationRequest {
                generation_id: generation,
                public_id: "public-generation-b".to_string(),
                accept_key: "accept-generation-b".to_string(),
                client_action_id: Some("new-action".to_string()),
                prompt_schema_version: 1,
                lease_owner: "owner-b".to_string(),
                lease_expires_at: lease_expiry(),
                now: now(),
            },
        )
        .await
        .expect("generation begins");

    let failed = store
        .fail_generation_for_run(
            &scope(),
            "public-generation-b",
            TurnRunId::new(),
            "suggestion generation run failed".to_string(),
            now(),
        )
        .await
        .expect("matching terminal failure settles");
    assert!(matches!(
        failed.map(|document| document.generation),
        Some(GenerationState::Failed { .. })
    ));
}

#[tokio::test]
async fn generation_failure_retains_cards_and_replays_terminal_action() {
    let store = store();
    ready_store(&store).await;
    let generation = generation_id("generation-b");
    store
        .begin_generation(
            &scope(),
            BeginGenerationRequest {
                generation_id: generation.clone(),
                public_id: "public-generation-b".to_string(),
                accept_key: "accept-generation-b".to_string(),
                client_action_id: Some("new-action".to_string()),
                prompt_schema_version: 1,
                lease_owner: "owner-b".to_string(),
                lease_expires_at: lease_expiry(),
                now: now(),
            },
        )
        .await
        .expect("generation begins");
    let failed = store
        .fail_generation(
            &scope(),
            &generation,
            "owner-b",
            "provider failed".to_string(),
            now() + chrono::Duration::days(1),
        )
        .await
        .expect("failure persists")
        .expect("current failure returns document");
    assert!(matches!(failed.generation, GenerationState::Failed { .. }));
    assert_eq!(failed.suggestions.len(), 1);

    let replay = store
        .begin_generation(
            &scope(),
            BeginGenerationRequest {
                generation_id: generation_id("replayed-generation"),
                public_id: "replayed-public".to_string(),
                accept_key: "replayed-accept".to_string(),
                client_action_id: Some("new-action".to_string()),
                prompt_schema_version: 1,
                lease_owner: "replay-owner".to_string(),
                lease_expires_at: now(),
                now: now() + chrono::Duration::days(2),
            },
        )
        .await
        .expect("same failed action replays");
    assert!(!replay.is_acquired());
    assert_eq!(replay.state(), &failed.generation);
}

#[tokio::test]
async fn start_reservation_replays_and_rejects_a_conflicting_operation() {
    let store = store();
    ready_store(&store).await;
    let reservation = SuggestionStartReservation {
        thread_action_id: "thread-action-a".to_string(),
        turn_action_id: "turn-action-a".to_string(),
        agent_id: None,
        project_id: None,
    };

    let first = store
        .reserve_start(
            &scope(),
            &suggestion_id("card-a"),
            reservation.clone(),
            now(),
        )
        .await
        .expect("start is reserved");
    let replay = store
        .reserve_start(
            &scope(),
            &suggestion_id("card-a"),
            reservation.clone(),
            now(),
        )
        .await
        .expect("same start reservation replays");
    assert_eq!(first, SuggestionStartClaim::Reserved(reservation.clone()));
    assert_eq!(replay, SuggestionStartClaim::Reserved(reservation.clone()));

    let conflict = store
        .reserve_start(
            &scope(),
            &suggestion_id("card-a"),
            SuggestionStartReservation {
                thread_action_id: "thread-action-b".to_string(),
                turn_action_id: "turn-action-b".to_string(),
                agent_id: None,
                project_id: None,
            },
            now(),
        )
        .await
        .expect_err("a different operation cannot replace the reservation");
    assert!(matches!(
        conflict,
        SuggestionsStoreError::StartReservationConflict { .. }
    ));

    let binding = SuggestionBinding {
        thread_id: ThreadId::new("thread-started").expect("valid thread"),
        run_id: TurnRunId::new(),
    };
    store
        .complete_start(
            &scope(),
            &suggestion_id("card-a"),
            &reservation,
            binding.clone(),
            now(),
        )
        .await
        .expect("reserved start completes");
    let completed_replay = store
        .reserve_start(&scope(), &suggestion_id("card-a"), reservation, now())
        .await
        .expect("completed start replays binding");
    assert_eq!(completed_replay, SuggestionStartClaim::Bound(binding));
}

#[tokio::test]
async fn start_reservation_rejects_a_changed_target_context() {
    let store = store();
    let scope_a = context_scope("agent-a", "project-a");
    let scope_b = context_scope("agent-b", "project-b");
    ready_store_at(&store, &scope_a).await;
    let reservation_a = SuggestionStartReservation {
        thread_action_id: "thread-action-stable".to_string(),
        turn_action_id: "turn-action-stable".to_string(),
        agent_id: scope_a.agent_id.clone(),
        project_id: scope_a.project_id.clone(),
    };
    store
        .reserve_start(
            &scope_a,
            &suggestion_id("card-a"),
            reservation_a.clone(),
            now(),
        )
        .await
        .expect("first context reserves the start");

    let reservation_b = SuggestionStartReservation {
        thread_action_id: reservation_a.thread_action_id.clone(),
        turn_action_id: reservation_a.turn_action_id.clone(),
        agent_id: scope_b.agent_id.clone(),
        project_id: scope_b.project_id.clone(),
    };
    let conflict = store
        .reserve_start(&scope_b, &suggestion_id("card-a"), reservation_b, now())
        .await
        .expect_err("a changed target context cannot replay the reservation");
    assert!(matches!(
        conflict,
        SuggestionsStoreError::StartReservationConflict { .. }
    ));

    let document = store
        .read(&scope_a)
        .await
        .expect("reservation reads")
        .expect("document remains durable");
    assert_eq!(
        document.suggestions[0]
            .start_reservation
            .as_ref()
            .expect("reservation remains")
            .agent_id,
        scope_a.agent_id
    );
    assert_eq!(
        document.suggestions[0]
            .start_reservation
            .as_ref()
            .expect("reservation remains")
            .project_id,
        scope_a.project_id
    );
}

#[tokio::test]
async fn replacement_generation_retains_reserved_card_before_start_completion() {
    let store = store();
    ready_store(&store).await;
    let reservation = SuggestionStartReservation {
        thread_action_id: "thread-action-a".to_string(),
        turn_action_id: "turn-action-a".to_string(),
        agent_id: None,
        project_id: None,
    };
    store
        .reserve_start(
            &scope(),
            &suggestion_id("card-a"),
            reservation.clone(),
            now(),
        )
        .await
        .expect("start reservation succeeds");

    store
        .begin_generation(
            &scope(),
            generation_request("generation-b", "owner-b", lease_expiry(), now()),
        )
        .await
        .expect("replacement generation starts");
    let document = store
        .read(&scope())
        .await
        .expect("reads replacement state")
        .expect("document remains durable");
    assert_eq!(document.suggestions.len(), 1);
    assert!(matches!(
        document.generation,
        GenerationState::Generating { .. }
    ));

    let binding = SuggestionBinding {
        thread_id: ThreadId::new("thread-started").expect("valid thread id"),
        run_id: TurnRunId::new(),
    };
    let completed = store
        .complete_start(
            &scope(),
            &suggestion_id("card-a"),
            &reservation,
            binding.clone(),
            now(),
        )
        .await
        .expect("an accepted start must complete after replacement");
    assert_eq!(completed, binding);
}

#[tokio::test]
async fn dismissed_suggestion_cannot_be_reserved_for_start() {
    let store = store();
    ready_store(&store).await;
    store
        .dismiss(&scope(), &suggestion_id("card-a"), now())
        .await
        .expect("dismisses card");

    let error = store
        .reserve_start(
            &scope(),
            &suggestion_id("card-a"),
            SuggestionStartReservation {
                thread_action_id: "thread-action".to_string(),
                turn_action_id: "turn-action".to_string(),
                agent_id: None,
                project_id: None,
            },
            now(),
        )
        .await
        .expect_err("dismissed card cannot start");
    assert!(matches!(
        error,
        SuggestionsStoreError::SuggestionDismissed { .. }
    ));
}

#[tokio::test]
async fn pending_start_binding_survives_concurrent_dismissal() {
    let store = store();
    ready_store(&store).await;
    let reservation = SuggestionStartReservation {
        thread_action_id: "thread-action-a".to_string(),
        turn_action_id: "turn-action-a".to_string(),
        agent_id: None,
        project_id: None,
    };
    store
        .reserve_start(
            &scope(),
            &suggestion_id("card-a"),
            reservation.clone(),
            now(),
        )
        .await
        .expect("start reservation succeeds");
    let first = SuggestionBinding {
        thread_id: ThreadId::new("thread-a").expect("valid thread"),
        run_id: TurnRunId::new(),
    };
    store
        .dismiss(&scope(), &suggestion_id("card-a"), now())
        .await
        .expect("dismisses card while its start is reserved");

    assert_eq!(
        store
            .complete_start(
                &scope(),
                &suggestion_id("card-a"),
                &reservation,
                first.clone(),
                now(),
            )
            .await
            .expect("reserved start still records its durable binding"),
        first
    );

    let document = store
        .read(&scope())
        .await
        .expect("reads document")
        .expect("document retained");
    assert_eq!(document.suggestions.len(), 1);
    assert_eq!(document.suggestions[0].binding, Some(first));
    assert!(document.suggestions[0].dismissed_at.is_some());
}
