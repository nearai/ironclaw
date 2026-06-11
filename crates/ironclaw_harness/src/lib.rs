//! Harness-manifest format and parser — slice 4 of epic
//! [#3036](https://github.com/nearai/ironclaw/issues/3036).
//!
//! A **harness** is a named use-case composition: a prompt overlay, runtime
//! constraints, required extensions/skills, a capability-surface filter, and a
//! seeded memory layout. It is a higher-level concept than an extension (which
//! declares capabilities) or a skill (which extends prompts) — it composes them
//! into a mode with its own lifecycle. This crate owns the manifest *format*
//! and its parser; the typed harness repo, activation gating, and prompt-overlay
//! composition are later slices.
//!
//! Invariants enforced here mirror the blueprint parser:
//!
//! - `api_version` locks the schema major ([`SUPPORTED_API_VERSION`]).
//! - Unknown keys are a hard error (`deny_unknown_fields`).
//! - Inline secret material is rejected; only `${secret:<name>}` handles pass.
//!
//! Shared sub-shapes ([`ironclaw_blueprint::CapabilitySurface`],
//! [`PromptOverlay`](ironclaw_blueprint::PromptOverlay),
//! [`RequiredRef`](ironclaw_blueprint::RequiredRef)) are reused from the
//! blueprint crate rather than redefined.
//!
//! ```
//! let src = r#"
//! api_version = "ironclaw.harness/v1"
//! kind = "Harness"
//! id = "red-team"
//! name = "Red Team Operator"
//! trust = "user_trusted"
//!
//! [prompt_overlay]
//! text_ref = "prompts/red-team-system.md"
//!
//! [runtime_constraints]
//! max_profile = "Sandboxed"
//! "#;
//! let harness = ironclaw_harness::parse(src).expect("valid manifest");
//! assert_eq!(harness.id, "red-team");
//! ```

mod error;
mod parser;
mod schema;
mod secret_scan;

pub use error::HarnessError;
pub use parser::{SUPPORTED_API_VERSION, parse};
pub use schema::{HarnessKind, HarnessManifest, RuntimeConstraints};
