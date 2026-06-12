//! Error types for blueprint parsing, validation, and lockfile resolution.

use ironclaw_host_api::HostApiError;
use ironclaw_reborn_config::InlineSecretError;
use thiserror::Error;

/// Extract the human-readable rule from a host-api validation error, dropping
/// the upstream `kind` label — a skill validated via the extension grammar
/// must not report itself as an "extension", and a secret handle carries its
/// own context in [`BlueprintError::InvalidSecretHandle`].
pub(crate) fn host_api_reason(err: HostApiError) -> String {
    match err {
        HostApiError::InvalidId { reason, .. } => reason,
        other => other.to_string(),
    }
}

/// Anything that can go wrong turning blueprint source into a validated
/// [`crate::Blueprint`] AST or a [`crate::Lockfile`].
///
/// Every variant is fail-loud and points at the offending location: the
/// epic requires unknown keys, inline secrets, and version mismatches to be
/// hard errors with a path, never silently ignored.
#[derive(Debug, Error)]
pub enum BlueprintError {
    /// TOML did not parse, or a typed field had the wrong shape / an unknown
    /// key. `toml`'s own error already carries the source span.
    #[error("blueprint is not valid: {0}")]
    Toml(#[from] toml::de::Error),

    /// `api_version` was missing, malformed, or names a major this build
    /// cannot reconcile. Schema majors are forever — a mismatch is a hard
    /// stop, not a best-effort parse.
    #[error(
        "unsupported api_version `{found}`: this build understands `{}` \
         (a new major requires a migration path, see epic #3036)",
        crate::SUPPORTED_API_VERSION
    )]
    UnsupportedApiVersion { found: String },

    /// A value that should be a `${secret:<name>}` handle contained inline
    /// secret material, or some other string in the document looked like a
    /// pasted credential.
    #[error("at `{path}`: {source}")]
    InlineSecret {
        path: String,
        #[source]
        source: InlineSecretError,
    },

    /// A `${secret:<name>}` handle had an invalid name segment. The grammar
    /// is owned by `SecretHandle` in `ironclaw_host_api`; `reason` carries its
    /// exact rule so this message never drifts from the real validator.
    #[error("at `{path}`: secret handle `{handle}` is invalid ({reason})")]
    InvalidSecretHandle {
        path: String,
        handle: String,
        reason: String,
    },

    /// A scope / extension / skill / mission / project identifier was empty
    /// or used disallowed characters. The grammars are owned by the typed IDs
    /// in `ironclaw_host_api`; `reason` carries the rule that was violated.
    #[error("at `{path}`: identifier `{value}` is invalid ({reason})")]
    InvalidIdentifier {
        path: String,
        value: String,
        reason: String,
    },

    /// An `extensions[].version` requirement was not a valid semver range.
    #[error("at `{path}`: version requirement `{value}` is invalid ({reason})")]
    InvalidVersionReq {
        path: String,
        value: String,
        reason: String,
    },

    /// A `text_ref` / `brief_ref` file reference escaped the blueprint root
    /// or used an absolute path. File refs are always root-relative so the
    /// lockfile is reproducible across machines.
    #[error("at `{path}`: file reference `{reference}` is invalid ({reason})")]
    InvalidFileRef {
        path: String,
        reference: String,
        reason: String,
    },

    /// A referenced file could not be read while building the lockfile.
    #[error("at `{path}`: cannot read file reference `{reference}`: {reason}")]
    FileRefRead {
        path: String,
        reference: String,
        reason: String,
    },
}
