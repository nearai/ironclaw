-- Reborn product workflow durable state.
--
-- Two tables backing the ProductAdapter / ProductWorkflow stack:
--   * product_inbound_actions: idempotency ledger keyed by
--     (adapter_id, installation_id, source_binding_key, external_event_id).
--     Webhook retries with the same external_event_id replay the prior
--     outcome instead of double-dispatching.
--   * product_bindings: maps external (adapter, installation, conversation,
--     actor) tuples to canonical Reborn (tenant, user, thread, agent_id?,
--     project_id?). Created lazily on first inbound from a previously-unseen
--     conversation.
--
-- Phase enum values mirror ActionPhase's serde rename_all = "snake_case":
--   received | dispatched | settled | deduplicated_replay.

CREATE TABLE IF NOT EXISTS product_inbound_actions (
    action_id TEXT PRIMARY KEY,
    adapter_id TEXT NOT NULL,
    installation_id TEXT NOT NULL,
    source_binding_key TEXT NOT NULL,
    external_event_id TEXT NOT NULL,
    phase TEXT NOT NULL,
    dispatch_kind_json TEXT,
    outcome_json TEXT,
    received_at TIMESTAMPTZ NOT NULL,
    settled_at TIMESTAMPTZ,
    UNIQUE (adapter_id, installation_id, source_binding_key, external_event_id)
);

CREATE INDEX IF NOT EXISTS idx_product_inbound_actions_phase
    ON product_inbound_actions(phase, received_at);

CREATE TABLE IF NOT EXISTS product_bindings (
    adapter_id TEXT NOT NULL,
    installation_id TEXT NOT NULL,
    external_conversation_fingerprint TEXT NOT NULL,
    external_actor_kind TEXT NOT NULL,
    external_actor_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    agent_id TEXT,
    project_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (
        adapter_id,
        installation_id,
        external_conversation_fingerprint,
        external_actor_kind,
        external_actor_id
    )
);

CREATE INDEX IF NOT EXISTS idx_product_bindings_thread
    ON product_bindings(thread_id);

CREATE INDEX IF NOT EXISTS idx_product_bindings_user
    ON product_bindings(user_id);
