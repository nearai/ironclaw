//! Error types for harness-manifest parsing and validation.

use ironclaw_reborn_config::InlineSecretError;
use thiserror::Error;

/// Anything that can go wrong turning manifest source into a validated
/// [`crate::HarnessManifest`].
#[derive(Debug, Error)]
pub enum HarnessError {
    #[error("harness manifest is not valid: {0}")]
    Toml(#[from] toml::de::Error),

    #[error(
        "unsupported api_version `{found}`: this build understands `{}`",
        crate::SUPPORTED_API_VERSION
    )]
    UnsupportedApiVersion { found: String },

    #[error("at `{path}`: {source}")]
    InlineSecret {
        path: String,
        #[source]
        source: InlineSecretError,
    },

    #[error(
        "at `{path}`: secret handle `{handle}` is invalid ({reason}); \
         names must be lowercase, start with a letter, and use only `a-z0-9_-.`"
    )]
    InvalidSecretHandle {
        path: String,
        handle: String,
        reason: String,
    },

    #[error("at `{path}`: identifier `{value}` is invalid ({reason})")]
    InvalidIdentifier {
        path: String,
        value: String,
        reason: String,
    },
}
