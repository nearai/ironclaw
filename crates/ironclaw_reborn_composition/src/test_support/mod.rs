//! Test-only helpers for the Reborn integration-test framework and budget E2E tests.
//!
//! Gated behind the `test-support` feature so production builds never pay the cost
//! of the mock gateway / introspection accessors. Each independent seam family
//! lives in its own submodule; this file is a thin re-export layer so the full
//! public surface is visible at a glance without wading through implementation
//! bodies:
//!
//! 1. [`budget_gateway`] — [`BudgetTestGateway`], [`FailingTestGateway`],
//!    [`ScriptedReply`] — scripted model responses with configurable token
//!    counts for `RebornRuntimeInput::with_model_gateway_override` tests.
//! 2. [`oauth_product_auth`] — [`ScriptedOAuthTokenEgress`],
//!    [`OAuthProductAuthTestBundle`], `build_oauth_product_auth_for_test`,
//!    `build_google_oauth_product_auth_for_test` — real store / real client /
//!    scripted HTTP egress for OAuth connect, refresh, and error-path tests.
//! 3. [`local_dev_boot`] — `build_local_dev_approval_gate_evidence_for_test`,
//!    `build_default_local_dev_database_roots_for_test`,
//!    `mount_local_dev_database_roots_for_test`,
//!    `build_local_dev_secret_store_for_test` — mirror the production
//!    local-dev boot sequence so the integration-test harness
//!    (`tests/support/reborn/`) drives the real local-dev composition paths
//!    without duplicating the wiring logic.
//! 4. [`project_create`] — `project_create` synthetic-capability test support
//!    (E-PROJ seam).
//! 5. [`durable`] — extension-installation, approval-request, and trigger
//!    durable-store test support (E-DURABLE / C-DURABLE seam).
//! 6. [`skill_activation`] — `skill_activate` synthetic-capability test
//!    support (E-SKILL seam).
//! 7. [`user_profile`] — `HostUserProfileSource` test support (E-PROFILE
//!    seam).
//! 8. [`trigger_materializer`] — `materialize_trigger_prompt_for_test`, the
//!    single production-owned trusted-trigger prompt materializer entry
//!    point for the integration-test harness (E-TRIGGERED-SUBMIT seam).
//! 9. [`trace_capture`] — `trace_capture_turn_event_sink_for_test`, the
//!    production `TraceCaptureTurnEventSink` factory for the integration-test
//!    harness (C-TRACECAP seam).
//! 10. [`automation`] — `local_dev_automation_product_facade_for_test`, the
//!     production `RebornAutomationProductFacade` constructor for the
//!     automations-cold-LIST scenario (W5-WEBUI-API-1 Enabler B.2).
//! 11. [`projection`] — `build_webui_event_stream_for_test`, a deliberately
//!     narrowed `ProjectionStream` (turn-lifecycle events only) for the SSE
//!     activity-stream scenario (W5-WEBUI-API-1 Enabler A).

mod automation;
mod budget_gateway;
mod durable;
mod local_dev_boot;
mod oauth_product_auth;
mod outbound_delivery;
mod project_create;
mod projection;
mod skill_activation;
mod trace_capture;
mod trigger_materializer;
mod user_profile;

#[cfg(feature = "test-support")]
pub use automation::local_dev_automation_product_facade_for_test;
pub use budget_gateway::{
    BudgetTestGateway, FailingTestGateway, ScriptedReply, assistant_reply_without_text_for_test,
};
#[cfg(feature = "test-support")]
pub use durable::open_local_dev_extension_installation_store_for_test;
#[cfg(all(feature = "test-support", feature = "libsql"))]
pub use durable::{
    open_local_dev_approval_request_store_for_test,
    open_local_dev_approval_settings_stores_for_test, open_local_dev_trigger_repository_for_test,
};
pub use local_dev_boot::LOCAL_DEV_DB_FILENAME;
#[cfg(any(feature = "libsql", feature = "postgres"))]
pub use local_dev_boot::build_local_dev_secret_store_for_test;
#[cfg(feature = "test-support")]
pub use local_dev_boot::{
    build_default_local_dev_database_roots_for_test,
    build_local_dev_approval_gate_evidence_for_test, mount_local_dev_database_roots_for_test,
};
#[cfg(any(feature = "libsql", feature = "postgres"))]
pub use oauth_product_auth::build_google_oauth_product_auth_for_test;
pub use oauth_product_auth::{
    OAuthProductAuthTestBundle, ScriptedOAuthTokenEgress, build_oauth_product_auth_for_test,
};
#[cfg(feature = "test-support")]
pub use outbound_delivery::{
    OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID, OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID,
    OutboundDeliveryCapabilityTestParts, wrap_outbound_delivery_capabilities_for_test,
};
#[cfg(feature = "test-support")]
pub use project_create::{PROJECT_CREATE_CAPABILITY_ID, wrap_project_create_capability_for_test};
#[cfg(feature = "test-support")]
pub use projection::build_webui_event_stream_for_test;
#[cfg(feature = "test-support")]
pub use skill_activation::{
    SKILL_ACTIVATE_CAPABILITY_ID, SkillActivationTestSource,
    build_local_dev_skill_context_source_for_test, wrap_skill_activation_capability_for_test,
};
#[cfg(feature = "test-support")]
pub use trace_capture::trace_capture_turn_event_sink_for_test;
#[cfg(feature = "test-support")]
pub use trigger_materializer::materialize_trigger_prompt_for_test;
#[cfg(feature = "test-support")]
pub use user_profile::build_user_profile_source_for_test;
