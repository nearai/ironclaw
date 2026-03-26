//! Interactive setup wizard for IronClaw.
//!
//! Provides a guided setup experience for:
//! 1. Database connection
//! 2. Security (secrets master key)
//! 3. Inference provider selection
//! 4. Model selection
//! 5. Embeddings
//! 6. Channel configuration (HTTP, Telegram, etc.)
//! 7. Extensions (tool installation from registry)
//! 8. Heartbeat (background tasks)
//!
//! Personal onboarding happens conversationally during the user's first
//! assistant interaction (see `workspace/mod.rs` bootstrap block).
//!
//! # Example
//!
//! ```ignore
//! use ironclaw::setup::SetupWizard;
//!
//! let mut wizard = SetupWizard::new();
//! wizard.run().await?;
//! ```

mod channels;
pub mod profile_evolution;
mod prompts;
#[cfg(any(feature = "postgres", feature = "libsql"))]
mod wizard;

pub use channels::{ChannelSetupError, SecretsContext, setup_http, setup_tunnel};
pub use prompts::{
    confirm, input, optional_input, print_error, print_header, print_info, print_step,
    print_success, secret_input, select_many, select_one,
};
#[cfg(any(feature = "postgres", feature = "libsql"))]
pub use wizard::{SetupConfig, SetupError, SetupWizard};

/// Check if onboarding is needed and return the reason.
///
/// Reads environment variables (`DATABASE_URL`, `LIBSQL_PATH`,
/// `ONBOARD_COMPLETED`, `NEARAI_API_KEY`) and checks for the default
/// session file on disk. Not safe to call concurrently with `env::set_var`.
#[cfg(any(feature = "postgres", feature = "libsql"))]
pub fn check_onboard_needed() -> Option<&'static str> {
    let has_db = std::env::var("DATABASE_URL").is_ok()
        || std::env::var("LIBSQL_PATH").is_ok()
        || crate::config::default_libsql_path().exists();

    if !has_db {
        return Some("Database not configured");
    }

    if std::env::var("ONBOARD_COMPLETED")
        .map(|v| v == "true")
        .unwrap_or(false)
    {
        return None;
    }

    if std::env::var("NEARAI_API_KEY").is_err() {
        let session_path = crate::config::default_session_path();
        if !session_path.exists() {
            return Some("First run");
        }
    }

    None
}
