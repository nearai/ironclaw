//! Consolidated scope-isolation parity suites. Structural move: members
//! are the former tests/<name>.rs binaries, verbatim (only their mount
//! headers moved here); one binary now links instead of 11. See
//! crates/loop/ironclaw_hooks/tests/parity_matrix.rs for the mounting
//! convention this file follows.
#[allow(dead_code)]
#[path = "support/reborn_parity_qa/mod.rs"]
mod parity_qa_support;
#[allow(dead_code)]
#[path = "integration/support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
mod support;

#[path = "reborn_scope_isolation_suite/reborn_adapter_installation_scope_isolation_parity.rs"]
mod reborn_adapter_installation_scope_isolation_parity;
#[path = "reborn_scope_isolation_suite/reborn_agent_scope_isolation_parity.rs"]
mod reborn_agent_scope_isolation_parity;
#[path = "reborn_scope_isolation_suite/reborn_direct_chat_user_scope_isolation_parity.rs"]
mod reborn_direct_chat_user_scope_isolation_parity;
#[path = "reborn_scope_isolation_suite/reborn_http_network_scope_isolation_parity.rs"]
mod reborn_http_network_scope_isolation_parity;
#[path = "reborn_scope_isolation_suite/reborn_identity_project_scope_isolation_parity.rs"]
mod reborn_identity_project_scope_isolation_parity;
#[path = "reborn_scope_isolation_suite/reborn_identity_prompt_scope_isolation_parity.rs"]
mod reborn_identity_prompt_scope_isolation_parity;
#[path = "reborn_scope_isolation_suite/reborn_identity_tenant_scope_isolation_parity.rs"]
mod reborn_identity_tenant_scope_isolation_parity;
#[path = "reborn_scope_isolation_suite/reborn_project_scope_isolation_parity.rs"]
mod reborn_project_scope_isolation_parity;
#[path = "reborn_scope_isolation_suite/reborn_tenant_binding_scope_isolation_parity.rs"]
mod reborn_tenant_binding_scope_isolation_parity;
#[path = "reborn_scope_isolation_suite/reborn_thread_binding_isolation_parity.rs"]
mod reborn_thread_binding_isolation_parity;
#[path = "reborn_scope_isolation_suite/reborn_wrong_scope_access_isolation_parity.rs"]
mod reborn_wrong_scope_access_isolation_parity;
