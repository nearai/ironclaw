//! Interactive setup wizard for NEAR Agent.
//!
//! Provides a guided setup experience for:
//! - NEAR AI authentication
//! - Model selection
//! - Channel configuration (HTTP, Telegram, etc.)
//!
//! # Example
//!
//! ```ignore
//! use near_agent::setup::SetupWizard;
//!
//! let mut wizard = SetupWizard::new();
//! wizard.run().await?;
//! ```

mod channels;
mod prompts;
mod wizard;

pub use channels::{SecretsContext, setup_http, setup_telegram, validate_telegram_token};
pub use prompts::{confirm, print_header, print_step, secret_input, select_many, select_one};
pub use wizard::{SetupConfig, SetupWizard};
