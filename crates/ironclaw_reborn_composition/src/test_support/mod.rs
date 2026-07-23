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
//! 3. [`local_dev_boot`] — `build_approval_gate_evidence_for_test`,
//!    `build_default_local_dev_database_roots_for_test`,
//!    `mount_local_dev_database_roots_for_test`,
//!    `build_secret_store_for_test` — mirror the production
//!    local-dev boot sequence so the integration-test harness
//!    (`tests/support/reborn/`) drives the real local-dev composition paths
//!    without duplicating the wiring logic.
//! 4. [`project_create`] — `project_create` synthetic-capability test support
//!    (E-PROJ seam).
//! 5. [`durable`] — extension-installation, approval-request, trigger,
//!    outbound-preferences, and approval-settings durable-store test support
//!    (E-DURABLE / C-DURABLE / W6-COLD-SPOTS / W5-WEBUI-API-1 seam).
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
//!     automations-cold-LIST scenario (W5-WEBUI-API-1 Enabler B.2), plus
//!     `local_dev_trigger_active_run_lookup_for_test` (the raw
//!     `TriggerActiveRunLookup`, for wiring the `builtin.trigger_list`
//!     capability directly rather than through the facade, #5886).
//! 11. [`projection`] — `build_product_event_stream_for_test`, a deliberately
//!     narrowed `ProjectionStream` (turn-lifecycle events only) for the SSE
//!     activity-stream scenario (W5-WEBUI-API-1 Enabler A).
//! 12. [`refreshing_capability_port`] — `create_refreshing_capability_port_for_test`,
//!     the production `create_refreshing_capability_port` factory
//!     (all wrap layers) driven with harness-injectable parts (harness-port-seam
//!     P1 seam).
//! 13. [`local_dev_capability_io`] — `staged_capability_io_for_test`, the
//!     production `StagedCapabilityIo` constructor (`capability_wiring`'s
//!     `new_with_durable_previews` call), for durable tool-result projection
//!     coverage (issue #5838).
//! 14. [`result_read`] — `wrap_result_read_capability_for_test`, the
//!     production `result_read` synthetic-capability wrap, for the same
//!     durable tool-result projection coverage (issue #5838).
//! 15. [`channel_connection`] — [`ChannelConnectionTestBundle`],
//!     `build_channel_connection_for_test` — the REAL generic
//!     channel-connection facade (§6.4) + OAuth-callback-shaped identity
//!     binding over a composed harness's own stores, late-bound into the
//!     same removal-cleanup slot production fills (C-SLACK-LIFECYCLE seam,
//!     issue #6105).

mod automation;
mod budget_gateway;
#[cfg(feature = "test-support")]
mod channel_connection;
mod durable;
mod local_dev_boot;
mod local_dev_capability_io;
mod oauth_product_auth;
mod outbound_delivery;
mod project_create;
mod projection;
mod refreshing_capability_port;
mod result_read;
mod skill_activation;
mod trace_capture;
mod trigger_materializer;
mod user_profile;

#[cfg(feature = "test-support")]
pub use automation::{
    local_dev_automation_product_facade_for_test, local_dev_trigger_active_run_lookup_for_test,
    register_static_source_delivery_target_for_test,
    set_local_dev_trigger_source_turn_state_for_test,
};
pub use budget_gateway::{
    BudgetTestGateway, FailingTestGateway, ScriptedReply, assistant_reply_without_text_for_test,
};
#[cfg(feature = "test-support")]
pub use channel_connection::{
    ChannelConnectionTestBundle, ChannelConnectionTestConfig, build_channel_connection_for_test,
};
#[cfg(feature = "test-support")]
pub use durable::open_local_dev_extension_installation_store_for_test;
#[cfg(feature = "test-support")]
pub use durable::{
    open_local_dev_approval_request_store_for_test,
    open_local_dev_approval_settings_stores_for_test,
    open_local_dev_outbound_preferences_store_for_test, open_local_dev_trigger_repository_for_test,
};
pub use local_dev_boot::LOCAL_DEV_DB_FILENAME;
pub use local_dev_boot::build_secret_store_for_test;
#[cfg(feature = "test-support")]
pub use local_dev_boot::{
    build_approval_gate_evidence_for_test, build_default_local_dev_database_roots_for_test,
    mount_local_dev_database_roots_for_test,
};
#[cfg(feature = "test-support")]
pub use local_dev_capability_io::{
    staged_capability_io_for_test, staged_capability_io_with_observer_for_test,
};
pub use oauth_product_auth::build_google_oauth_product_auth_for_test;
pub use oauth_product_auth::build_oauth_product_auth_for_test_on_libsql;
pub use oauth_product_auth::build_oauth_product_auth_for_test_on_root;
pub use oauth_product_auth::{
    OAuthProductAuthTestBundle, ScriptedOAuthTokenEgress, build_oauth_product_auth_for_test,
    build_oauth_product_auth_with_identity_for_test,
    handle_oauth_callback_with_channel_identity_binding_for_test,
};
#[cfg(feature = "test-support")]
pub use outbound_delivery::{
    OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID, OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID,
};
#[cfg(feature = "test-support")]
pub use project_create::PROJECT_CREATE_CAPABILITY_ID;
#[cfg(feature = "test-support")]
pub use projection::build_product_event_stream_for_test;
#[cfg(feature = "test-support")]
pub use refreshing_capability_port::{
    ExtensionManagementTestHandle, RefreshingCapabilityPortTestParts,
    build_extension_management_for_test, create_refreshing_capability_port_for_test,
};
#[cfg(feature = "test-support")]
pub use result_read::{RESULT_READ_CAPABILITY_ID, wrap_result_read_capability_for_test};
#[cfg(feature = "test-support")]
pub use skill_activation::{
    SKILL_ACTIVATE_CAPABILITY_ID, SkillActivationTestSource, build_skill_context_source_for_test,
};
#[cfg(feature = "test-support")]
pub use trace_capture::trace_capture_turn_event_sink_for_test;
#[cfg(feature = "test-support")]
pub use trigger_materializer::materialize_trigger_prompt_for_test;
#[cfg(feature = "test-support")]
pub use user_profile::build_user_profile_source_for_test;
