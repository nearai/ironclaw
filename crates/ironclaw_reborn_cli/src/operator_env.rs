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
