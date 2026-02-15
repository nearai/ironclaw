//! Interactive setup wizard for IronClaw.
//!
//! Provides a guided setup experience for:
//! 1. Database connection
//! 2. Security (secrets master key)
//! 3. Inference provider selection
//! 4. Model selection
//! 5. Embeddings
//! 6. Channel configuration (HTTP, Telegram, etc.)
//! 7. Heartbeat (background tasks)
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
pub use wizard::{SetupConfig, SetupWizard};
