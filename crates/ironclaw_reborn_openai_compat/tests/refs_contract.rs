use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_reborn_openai_compat::{
    InMemoryOpenAiCompatRefStore, OpenAiChatCompletionId, OpenAiCompatActorScope,
    OpenAiCompatBindInternalRefs, OpenAiCompatHttpError, OpenAiCompatIdempotencyKey,
    OpenAiCompatInternalRefs, OpenAiCompatProductActionRef, OpenAiCompatProjectionRef,
    OpenAiCompatPublicId, OpenAiCompatRefLookup, OpenAiCompatRefOperation,
    OpenAiCompatRefReservation, OpenAiCompatRefReservationOutcome, OpenAiCompatRefStore,
    OpenAiCompatRequestFingerprint, OpenAiCompatResourceBinding, OpenAiCompatRouteSurface,
    OpenAiCompatTurnRunRef, OpenAiResponseId,
};
use serde_json::json;

#[test]
fn public_refs_are_typed_opaque_and_serde_validated() {
    let chat = OpenAiChatCompletionId::new("chatcmpl-public_1").expect("chat ref");
    let response = OpenAiResponseId::new("resp_public-1").expect("response ref");

    assert_eq!(
        serde_json::to_value(&chat).expect("chat json"),
        json!("chatcmpl-public_1")
    );
    assert_eq!(
        serde_json::from_value::<OpenAiResponseId>(json!("resp_public-1")).expect("response ref"),
        response
    );

    for invalid in [
        "resp_wrong_for_chat",
        " chatcmpl-leading-space",
        "chatcmpl-trailing-space ",
        "chatcmpl-/path",
        "chatcmpl-secret-token",
        "chatcmpl-\ncontrol",
    ] {
        assert!(
            OpenAiChatCompletionId::new(invalid).is_err(),
            "{invalid:?} must reject"
        );
    }
}

#[test]
fn request_fingerprint_is_stable_digest_not_payload_storage() {
    let body = br#"{"input":"RAW_PROMPT_SENTINEL secret-token"}"#;
    let same = OpenAiCompatRequestFingerprint::from_body_bytes(body);
    let same_again = OpenAiCompatRequestFingerprint::from_body_bytes(body);
    let different = OpenAiCompatRequestFingerprint::from_body_bytes(br#"{"input":"other"}"#);

    assert_eq!(same, same_again);
    assert_ne!(same, different);
    assert!(same.as_str().starts_with("sha256:"));
    assert!(!same.as_str().contains("RAW_PROMPT_SENTINEL"));
    assert!(!same.as_str().contains("secret-token"));
}

#[test]
fn ref_mapping_deserialization_revalidates_nested_refs() {
    let mapping = json!({
        "public_id": {"kind": "response", "id": "resp_valid"},
        "owner": {
            "tenant_id": "tenant-a",
            "user_id": "user-a",
            "agent_id": "agent-a",
            "project_id": "project-a"
        },
        "surface": "responses_v1",
        "request_fingerprint": OpenAiCompatRequestFingerprint::from_body_bytes(b"body"),
        "idempotency_key": "client-key",
        "binding": {
            "state": "bound",
            "internal_refs": {
                "product_action_ref": "action:ok",
                "turn_run_ref": "run:ok"
            }
        }
    });
    serde_json::from_value::<ironclaw_reborn_openai_compat::OpenAiCompatResourceMapping>(mapping)
        .expect("valid mapping");

    let invalid = json!({
        "public_id": {"kind": "response", "id": "resp_valid"},
        "owner": {
            "tenant_id": "tenant-a",
            "user_id": "user-a"
        },
        "surface": "responses_v1",
        "request_fingerprint": OpenAiCompatRequestFingerprint::from_body_bytes(b"body"),
        "idempotency_key": " client-key ",
        "binding": {
            "state": "bound",
            "internal_refs": {
                "product_action_ref": "action:ok"
            }
        }
    });
    let err = serde_json::from_value::<ironclaw_reborn_openai_compat::OpenAiCompatResourceMapping>(
        invalid,
    )
    .expect_err("nested idempotency key must be revalidated");
    assert!(err.to_string().contains("leading or trailing whitespace"));

    let inconsistent = json!({
        "public_id": {"kind": "chat_completion", "id": "chatcmpl-valid"},
        "owner": {
            "tenant_id": "tenant-a",
            "user_id": "user-a"
        },
        "surface": "responses_v1",
        "request_fingerprint": OpenAiCompatRequestFingerprint::from_body_bytes(b"body"),
        "binding": {
            "state": "pending"
        }
    });
    let err = serde_json::from_value::<ironclaw_reborn_openai_compat::OpenAiCompatResourceMapping>(
        inconsistent,
    )
    .expect_err("public ref kind must match the route surface");
    assert!(err.to_string().contains("mapping is inconsistent"));
}

#[tokio::test]
async fn idempotency_key_replays_same_fingerprint_and_conflicts_on_different_body() {
    let store = InMemoryOpenAiCompatRefStore::new();
    let owner = actor_scope("alice");
    let key = OpenAiCompatIdempotencyKey::new("client-key-1").expect("key");
    let first_fingerprint = OpenAiCompatRequestFingerprint::from_body_bytes(br#"{"input":"a"}"#);
    let second_fingerprint = OpenAiCompatRequestFingerprint::from_body_bytes(br#"{"input":"b"}"#);

    let first = store
        .reserve(OpenAiCompatRefReservation::new(
            owner.clone(),
            OpenAiCompatRouteSurface::ResponsesV1,
            first_fingerprint.clone(),
            Some(key.clone()),
        ))
        .await
        .expect("reserve");
    let first_mapping = match first {
        OpenAiCompatRefReservationOutcome::Created(mapping) => mapping,
        other => panic!("expected created, got {other:?}"),
    };

    let replay = store
        .reserve(OpenAiCompatRefReservation::new(
            owner.clone(),
            OpenAiCompatRouteSurface::ResponsesV1,
            first_fingerprint,
            Some(key.clone()),
        ))
        .await
        .expect("replay");
    let replayed = match replay {
        OpenAiCompatRefReservationOutcome::Replayed(mapping) => mapping,
        other => panic!("expected replay, got {other:?}"),
    };
    assert_eq!(replayed.public_id, first_mapping.public_id);

    let conflict = store
        .reserve(OpenAiCompatRefReservation::new(
            owner,
            OpenAiCompatRouteSurface::ResponsesV1,
            second_fingerprint,
            Some(key),
        ))
        .await
        .expect("conflict outcome");
    assert!(matches!(
        conflict,
        OpenAiCompatRefReservationOutcome::Conflict(_)
    ));

    let error = OpenAiCompatHttpError::conflict(Some("idempotency_key".to_string()));
    assert_eq!(error.status_code(), 409);
    assert!(!error.retryable());
    let rendered = serde_json::to_string(error.body()).expect("error body");
    assert!(!rendered.contains("client-key-1"));
}

#[tokio::test]
async fn absent_idempotency_key_creates_new_public_ref_each_time() {
    let store = InMemoryOpenAiCompatRefStore::new();
    let owner = actor_scope("alice");
    let fingerprint = OpenAiCompatRequestFingerprint::from_body_bytes(br#"{"input":"a"}"#);

    let first = reserve_created(
        &store,
        owner.clone(),
        OpenAiCompatRouteSurface::ChatCompletions,
        fingerprint.clone(),
        None,
    )
    .await;
    let second = reserve_created(
        &store,
        owner,
        OpenAiCompatRouteSurface::ChatCompletions,
        fingerprint,
        None,
    )
    .await;

    assert_ne!(first.public_id, second.public_id);
    assert!(matches!(
        first.public_id,
        OpenAiCompatPublicId::ChatCompletion(_)
    ));
    assert!(first.public_id.as_str().starts_with("chatcmpl-"));
}

#[tokio::test]
async fn lookup_and_cancel_authorization_do_not_leak_cross_actor_existence() {
    let store = InMemoryOpenAiCompatRefStore::new();
    let alice = actor_scope("alice");
    let bob = actor_scope("bob");
    let mapping = reserve_created(
        &store,
        alice.clone(),
        OpenAiCompatRouteSurface::ResponsesApi,
        OpenAiCompatRequestFingerprint::from_body_bytes(br#"{"input":"a"}"#),
        Some(OpenAiCompatIdempotencyKey::new("client-key").expect("key")),
    )
    .await;

    let internal_refs = OpenAiCompatInternalRefs::new(
        OpenAiCompatProductActionRef::new("action:internal-1").expect("action ref"),
    )
    .with_turn_run_ref(OpenAiCompatTurnRunRef::new("run:internal-1").expect("run ref"))
    .with_projection_ref(
        OpenAiCompatProjectionRef::new("projection:internal-1").expect("projection ref"),
    );
    let bound = store
        .bind_internal_refs(OpenAiCompatBindInternalRefs::new(
            alice.clone(),
            mapping.public_id.clone(),
            internal_refs,
        ))
        .await
        .expect("bind")
        .expect("authorized bind");
    assert!(matches!(
        bound.binding,
        OpenAiCompatResourceBinding::Bound { .. }
    ));

    let alice_lookup = store
        .lookup_authorized(OpenAiCompatRefLookup::new(
            alice,
            mapping.public_id.clone(),
            OpenAiCompatRefOperation::Retrieve,
        ))
        .await
        .expect("lookup")
        .expect("alice can see mapping");
    assert_eq!(alice_lookup.public_id, mapping.public_id);

    let bob_cancel = store
        .lookup_authorized(OpenAiCompatRefLookup::new(
            bob,
            mapping.public_id,
            OpenAiCompatRefOperation::Cancel,
        ))
        .await
        .expect("lookup");
    let missing = store
        .lookup_authorized(OpenAiCompatRefLookup::new(
            actor_scope("bob"),
            OpenAiCompatPublicId::Response(OpenAiResponseId::new("resp_missing").expect("id")),
            OpenAiCompatRefOperation::Cancel,
        ))
        .await
        .expect("lookup");

    assert_eq!(bob_cancel, None);
    assert_eq!(missing, None);

    let unauthorized_shape = OpenAiCompatHttpError::not_found(Some("response_id".to_string()));
    let missing_shape = OpenAiCompatHttpError::not_found(Some("response_id".to_string()));
    assert_eq!(
        serde_json::to_value(unauthorized_shape.body()).expect("unauthorized"),
        serde_json::to_value(missing_shape.body()).expect("missing")
    );
}

#[tokio::test]
async fn generated_public_ref_does_not_embed_internal_refs() {
    let store = InMemoryOpenAiCompatRefStore::new();
    let owner = actor_scope("alice");
    let mapping = reserve_created(
        &store,
        owner.clone(),
        OpenAiCompatRouteSurface::ResponsesV1,
        OpenAiCompatRequestFingerprint::from_body_bytes(br#"{"input":"a"}"#),
        None,
    )
    .await;
    let bound = store
        .bind_internal_refs(OpenAiCompatBindInternalRefs::new(
            owner,
            mapping.public_id.clone(),
            OpenAiCompatInternalRefs::new(
                OpenAiCompatProductActionRef::new("action:raw-internal").expect("action ref"),
            )
            .with_turn_run_ref(OpenAiCompatTurnRunRef::new("run:raw-internal").expect("run ref")),
        ))
        .await
        .expect("bind")
        .expect("authorized");

    assert!(!bound.public_id.as_str().contains("raw-internal"));
    assert!(bound.public_id.as_str().starts_with("resp_"));
}

async fn reserve_created(
    store: &InMemoryOpenAiCompatRefStore,
    owner: OpenAiCompatActorScope,
    surface: OpenAiCompatRouteSurface,
    request_fingerprint: OpenAiCompatRequestFingerprint,
    idempotency_key: Option<OpenAiCompatIdempotencyKey>,
) -> ironclaw_reborn_openai_compat::OpenAiCompatResourceMapping {
    match store
        .reserve(OpenAiCompatRefReservation::new(
            owner,
            surface,
            request_fingerprint,
            idempotency_key,
        ))
        .await
        .expect("reserve")
    {
        OpenAiCompatRefReservationOutcome::Created(mapping) => mapping,
        other => panic!("expected created, got {other:?}"),
    }
}

fn actor_scope(user: &str) -> OpenAiCompatActorScope {
    OpenAiCompatActorScope::new(
        TenantId::new("tenant-a").expect("tenant"),
        UserId::new(format!("user-{user}")).expect("user"),
        Some(AgentId::new("agent-a").expect("agent")),
        Some(ProjectId::new("project-a").expect("project")),
    )
}
