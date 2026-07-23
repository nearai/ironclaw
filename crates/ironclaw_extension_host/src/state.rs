//! The two standard state machines (overview.md §6.1, §6.3), re-exported here
//! so they stay discoverable next to the `ExtensionHost` that drives
//! installation transitions.
//!
//! One installation enum and one auth-account enum, both wire-exposed exactly
//! as declared and rendered generically by the UI. No extension or vendor may
//! introduce a state, so both live in generic lower crates and nothing
//! downstream extends them:
//!
//! - [`InstallationState`] (§6.1) is host-owned installation-lifecycle
//!   vocabulary defined in `ironclaw_host_api::state`. Placing it there lets the
//!   product wire (`ironclaw_product`) and the host both name the same
//!   enum without a new dependency edge (both crates already depend on
//!   `ironclaw_host_api`). Its wire-form and transition tests live with the
//!   definition.
//! - [`AuthAccountState`] (§6.3) is owned by the auth engine and defined next to
//!   it in `ironclaw_auth::AuthAccountState`.

pub use ironclaw_auth::AuthAccountState;
pub use ironclaw_host_api::InstallationState;
