//! Shared environment-variable helpers for layering runtime settings.
//!
//! `IRONCLAW_*` runtime knobs use **strict presence** semantics: a
//! set-but-blank slot is a fatal misconfiguration, not a silent fall-through
//! to the config/default layer. A present-but-blank env slot is almost always
//! a bug — a shell typo, a half-set deployment template, or a credential
//! injector that failed to populate the slot — and falling through silently
//! would drop the operator's intended override with no visible signal.

use std::fmt::Display;
use std::str::FromStr;

/// Read a runtime env var with **strict** presence semantics.
///
/// - unset → `Ok(None)` (fall through to the config/default layer)
/// - set, empty or all-whitespace → fatal (operator must unset or fix)
/// - set, non-empty → `Ok(Some(value))` (caller validates content)
///
/// Distinct from `optional_nonempty_env` used by optional-config callers
/// (OAuth, etc.), which intentionally collapses present-blank to absent.
pub(super) fn strict_env_var(name: &str) -> anyhow::Result<Option<String>> {
    match std::env::var(name) {
        Ok(value) => {
            if value.trim().is_empty() {
                anyhow::bail!(
                    "{name} is set but empty or whitespace-only; either unset it or provide a valid value"
                );
            }
            Ok(Some(value))
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => anyhow::bail!(
            "{name} contains non-UTF-8 bytes; either unset it or provide a valid value"
        ),
    }
}

/// Truncate an env-var value to a bounded length before echoing it in an
/// error message. Prevents the value from blowing up startup logs if the
/// operator accidentally pastes a long string (e.g. a credential) into the
/// env slot. Char-aware so we cannot split a multi-byte UTF-8 codepoint.
pub(super) fn truncate_env_value_for_display(raw: &str) -> String {
    const MAX_CHARS: usize = 64;
    let mut iter = raw.chars();
    let truncated: String = iter.by_ref().take(MAX_CHARS).collect();
    if iter.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

/// Strict env var parsed into `T`.
///
/// - unset → `Ok(None)`
/// - set-but-blank / non-UTF-8 → fatal ([`strict_env_var`] semantics)
/// - set, unparseable as `T` → fatal, echoing the operator's (truncated) value
/// - set, parses → `Ok(Some(parsed))`
pub(super) fn strict_env_var_parsed<T>(name: &str) -> anyhow::Result<Option<T>>
where
    T: FromStr,
    <T as FromStr>::Err: Display,
{
    let Some(raw) = strict_env_var(name)? else {
        return Ok(None);
    };
    let parsed = raw.trim().parse::<T>().map_err(|e| {
        let display = truncate_env_value_for_display(&raw);
        anyhow::anyhow!("{name} must be a valid non-negative integer, got {display:?}: {e}")
    })?;
    Ok(Some(parsed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_keeps_short_values_intact() {
        assert_eq!(truncate_env_value_for_display("12"), "12");
    }

    #[test]
    fn truncate_appends_ellipsis_when_over_limit() {
        let long = "x".repeat(100);
        let out = truncate_env_value_for_display(&long);
        assert!(out.ends_with('…'));
        // 64 chars + the ellipsis.
        assert_eq!(out.chars().count(), 65);
    }
}
