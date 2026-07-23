use std::ffi::{OsStr, OsString};

const DEFAULT_ENV_PREFIX: &str = "IRONCLAW_";
const LEGACY_ENV_PREFIX: &str = "IRONCLAW_REBORN_";

/// Project the default `IRONCLAW_*` environment contract onto the legacy
/// `IRONCLAW_REBORN_*` names still consumed by lower-level compatibility
/// code.
///
/// This runs once during single-threaded process startup, after `.env` is
/// loaded and before CLI parsing or runtime construction. Default names win
/// when both forms are present. The compatibility projection can be removed
/// after the legacy environment contract reaches end of life.
pub(crate) fn install_legacy_env_aliases() {
    let environment = std::env::vars_os().collect::<Vec<_>>();
    let legacy_fallbacks = environment
        .iter()
        .filter_map(|(name, value)| {
            let default_name = default_name_for_legacy_alias(name)?;
            (!environment
                .iter()
                .any(|(candidate, _)| candidate == &default_name))
            .then(|| (default_name, value.clone()))
        })
        .collect::<Vec<_>>();
    let preferred_aliases = environment
        .iter()
        .filter_map(|(name, value)| {
            legacy_alias_for_default_name(name).map(|alias| (alias, value.clone()))
        })
        .collect::<Vec<_>>();

    for (name, value) in legacy_fallbacks.into_iter().chain(preferred_aliases) {
        // SAFETY: `main` calls this before CLI parsing, runtime construction,
        // or any worker thread is started. No concurrent environment access
        // exists at this point.
        unsafe {
            std::env::set_var(name, value); // env-hermetic: single-threaded startup shim
        }
    }
}

fn legacy_alias_for_default_name(name: &OsStr) -> Option<OsString> {
    let name = name.to_str()?;
    let suffix = name.strip_prefix(DEFAULT_ENV_PREFIX)?;
    if suffix.is_empty() || name.starts_with(LEGACY_ENV_PREFIX) {
        return None;
    }
    Some(format!("{LEGACY_ENV_PREFIX}{suffix}").into())
}

fn default_name_for_legacy_alias(name: &OsStr) -> Option<OsString> {
    let name = name.to_str()?;
    let suffix = name.strip_prefix(LEGACY_ENV_PREFIX)?;
    if suffix.is_empty() || matches!(suffix, "HOME" | "PROFILE") {
        return None;
    }
    Some(format!("{DEFAULT_ENV_PREFIX}{suffix}").into())
}

/// Read an operator-control env var with strict presence semantics.
///
/// These env vars are control-plane knobs: presence is authoritative, not
/// just non-empty content. Treat the var as:
///
/// - unset -> `Ok(None)` (fall through to the config/default layer)
/// - set, empty or all-whitespace -> fatal
/// - set, non-empty -> `Ok(Some(value))` (caller validates content)
pub(crate) fn strict_env_var(name: &str) -> anyhow::Result<Option<String>> {
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

pub(crate) fn strict_bool_env_var(name: &str) -> anyhow::Result<Option<bool>> {
    strict_env_var(name)?
        .map(|raw| parse_bool_env_var(name, &raw))
        .transpose()
}

fn parse_bool_env_var(name: &str, raw: &str) -> anyhow::Result<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" => Ok(true),
        "0" | "false" => Ok(false),
        _ => {
            let display = truncate_env_value_for_display(raw);
            anyhow::bail!("{name} must be one of 1, true, 0, false (got {display:?})")
        }
    }
}

/// Truncate an env-var value to a bounded length before echoing it in an
/// error message. Prevents the value from blowing up startup logs if the
/// operator accidentally pastes a long string into the env slot.
pub(crate) fn truncate_env_value_for_display(raw: &str) -> String {
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
pub(crate) fn strict_env_var_parsed<T>(name: &str) -> anyhow::Result<Option<T>>
where
    T: std::str::FromStr,
    <T as std::str::FromStr>::Err: std::fmt::Display,
{
    let Some(raw) = strict_env_var(name)? else {
        return Ok(None);
    };
    let parsed = raw.trim().parse::<T>().map_err(|e| {
        let display = truncate_env_value_for_display(&raw);
        anyhow::anyhow!(
            "{name} could not be parsed as a valid {}, got {display:?}: {e}",
            std::any::type_name::<T>()
        )
    })?;
    Ok(Some(parsed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_environment_names_project_to_legacy_names() {
        assert_eq!(
            legacy_alias_for_default_name(OsStr::new("IRONCLAW_WEBUI_TOKEN")),
            Some(OsString::from("IRONCLAW_REBORN_WEBUI_TOKEN"))
        );
        assert_eq!(
            legacy_alias_for_default_name(OsStr::new("IRONCLAW_HOME")),
            Some(OsString::from("IRONCLAW_REBORN_HOME"))
        );
    }

    #[test]
    fn legacy_and_unrelated_environment_names_are_not_projected() {
        assert_eq!(
            legacy_alias_for_default_name(OsStr::new("IRONCLAW_REBORN_HOME")),
            None
        );
        assert_eq!(legacy_alias_for_default_name(OsStr::new("HOME")), None);
        assert_eq!(
            default_name_for_legacy_alias(OsStr::new("IRONCLAW_REBORN_WEBUI_TOKEN")),
            Some(OsString::from("IRONCLAW_WEBUI_TOKEN"))
        );
        assert_eq!(
            default_name_for_legacy_alias(OsStr::new("IRONCLAW_REBORN_PROFILE")),
            None
        );
        assert_eq!(default_name_for_legacy_alias(OsStr::new("HOME")), None);
    }

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
