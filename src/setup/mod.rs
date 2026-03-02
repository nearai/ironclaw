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
//! assistant interaction (see `workspace/mod.rs` First Contact block).
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
#[cfg(any(feature = "postgres", feature = "libsql"))]
pub mod onboarding_chat;
pub mod profile_evolution;
mod prompts;
#[cfg(any(feature = "postgres", feature = "libsql"))]
mod wizard;

pub use channels::{
    ChannelSetupError, SecretsContext, setup_http, setup_telegram, setup_tunnel,
    validate_telegram_token,
};
pub use prompts::{
    confirm, input, optional_input, print_error, print_header, print_info, print_step,
    print_success, secret_input, select_many, select_one,
};
#[cfg(any(feature = "postgres", feature = "libsql"))]
pub use wizard::{SetupConfig, SetupError, SetupWizard};
