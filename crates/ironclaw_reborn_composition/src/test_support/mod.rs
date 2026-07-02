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
//! 5. [`durable`] — extension-installation durable-store test support
//!    (E-DURABLE seam).
//! 6. [`skill_activation`] — `skill_activate` synthetic-capability test
//!    support (E-SKILL seam).
//! 7. [`user_profile`] — `HostUserProfileSource` test support (E-PROFILE
//!    seam).

mod budget_gateway;
mod durable;
mod local_dev_boot;
mod oauth_product_auth;
mod project_create;
mod skill_activation;
mod user_profile;

pub use budget_gateway::{
    BudgetTestGateway, FailingTestGateway, ScriptedReply, assistant_reply_without_text_for_test,
};
#[cfg(feature = "test-support")]
pub use durable::open_local_dev_extension_installation_store_for_test;
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
pub use project_create::{PROJECT_CREATE_CAPABILITY_ID, wrap_project_create_capability_for_test};
#[cfg(feature = "test-support")]
pub use skill_activation::{
    SKILL_ACTIVATE_CAPABILITY_ID, SkillActivationTestSource,
    build_local_dev_skill_context_source_for_test, wrap_skill_activation_capability_for_test,
};
#[cfg(feature = "test-support")]
pub use user_profile::build_user_profile_source_for_test;
