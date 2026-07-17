//! Onboarding's LLM-credential prompt seam: where onboard's prompts
//! (provider menu, API key, model) come from, and the production
//! terminal-backed implementation.
//!
//! Injected (`PromptSource`) so `provision_llm_credentials`
//! (`super::llm_credentials::provision_llm_credentials`) is testable with a
//! fixed answer sequence, and so [`StdinPromptSource`] is the *only* place
//! that decides "is this session interactive" — matching the
//! injected-lookup convention `resolve_google_oauth_config` already
//! established (`crate::runtime::resolve_google_oauth_config`, which takes
//! a `lookup` closure rather than reading `std::env` inline) and the "only
//! `main.rs` may exit; `execute()` returns typed errors" rule: this trait's
//! methods return [`LlmCredentialPromptError::NonInteractive`] rather than
//! calling `process::exit`.

use std::io::{IsTerminal, Write as _};

/// Where onboarding's LLM-credential prompts (provider menu, API key,
/// model) come from, plus whether this session can prompt at all
/// ([`Self::is_interactive`]) — the single seam `provision_llm_credentials`'s
/// idempotent-rerun guard and `OnboardCommand::should_install_service` both
/// route through, so terminal detection lives in exactly one place.
pub(crate) trait PromptSource {
    /// `true` when this session can prompt at all (a real terminal is
    /// attached). Checked once up front so a non-interactive session skips
    /// both the LLM-credential prompts and the OS-service install without
    /// either one independently re-deriving "is this interactive".
    fn is_interactive(&self) -> bool;

    /// Prompt for the LLM provider via a numbered menu built from `entries`
    /// (`RebornProviderAdmin::menu_entries`'s output — `nearai` is entry 0
    /// in `providers.json`, so it is always menu item 1). Accepts a menu
    /// number, an exact provider id, or an alias (case-insensitive);
    /// invalid input re-prompts up to 3 attempts, then errors. Returns the
    /// selected entry's canonical provider id.
    ///
    /// Gated with the same `libsql`+`root-llm-provider` cfg as
    /// `ironclaw_reborn_composition::ProviderMenuEntry` itself, matching
    /// `provision_llm_credentials`'s own cfg split (see that function's
    /// feature-off stub, which never calls this method).
    #[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
    fn provider_menu(
        &mut self,
        entries: &[ironclaw_reborn_composition::ProviderMenuEntry],
    ) -> Result<String, LlmCredentialPromptError>;

    /// Prompt for `provider`'s API key with input masked (not echoed).
    fn api_key(&mut self, provider: &str) -> Result<String, LlmCredentialPromptError>;

    /// Prompt for a model override for `provider_id`. `default_model` is
    /// shown as the bracketed default; an empty/whitespace-only answer
    /// means "use the catalog default" (`Ok(None)`), any other answer is
    /// returned trimmed (`Ok(Some(..))`).
    ///
    /// Gated the same as [`Self::provider_menu`] — this trait's two
    /// composition-DTO-touching methods share one cfg reason.
    #[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
    fn model(
        &mut self,
        provider_id: &str,
        default_model: &str,
    ) -> Result<Option<String>, LlmCredentialPromptError>;
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum LlmCredentialPromptError {
    /// stdin is not a terminal (headless CI, a piped/scripted invocation).
    /// Callers should treat this as "skip, don't fail" — see
    /// `OnboardCommand::execute`'s handling next to
    /// `MasterKeyProvisionOutcome::Suppressed`, the same non-fatal shape for
    /// an unavailable interactive input.
    #[error(
        "onboarding LLM credential prompts require an interactive terminal; run \
         `ironclaw-reborn models set-provider <provider>` and set the provider's API key env \
         var instead, or rerun `onboard` from an interactive shell"
    )]
    NonInteractive,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Production [`PromptSource`]: reads the menu selection and model as plain
/// lines, the API key with terminal echo suppressed. The *only* place in
/// this module that checks [`IsTerminal`] or touches the real terminal —
/// everything else goes through the trait, matching the "only `main.rs` may
/// exit" convention (this impl never calls `process::exit`; it returns
/// [`LlmCredentialPromptError::NonInteractive`] and lets the caller decide).
pub(crate) struct StdinPromptSource;

impl PromptSource for StdinPromptSource {
    fn is_interactive(&self) -> bool {
        // Both streams matter: a redirected/piped stdout must not receive
        // the masked `*` characters `api_key`'s raw-mode read writes as the
        // operator types, even when stdin itself is a real terminal (e.g.
        // `ironclaw-reborn onboard > log.txt` in an interactive shell).
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
    }

    #[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
    fn provider_menu(
        &mut self,
        entries: &[ironclaw_reborn_composition::ProviderMenuEntry],
    ) -> Result<String, LlmCredentialPromptError> {
        if !std::io::stdin().is_terminal() {
            return Err(LlmCredentialPromptError::NonInteractive);
        }
        println!("Select an LLM provider:");
        for (index, entry) in entries.iter().enumerate() {
            let key_note = if entry.api_key_required {
                ""
            } else {
                " (no API key needed)"
            };
            println!(
                "  {}. {} — {}{key_note}",
                index + 1,
                entry.display_name,
                entry.description
            );
        }
        const MAX_ATTEMPTS: u8 = 3;
        for attempt in 1..=MAX_ATTEMPTS {
            print!("Provider [1-{}]: ", entries.len());
            std::io::stdout()
                .flush()
                .map_err(|error| LlmCredentialPromptError::Other(error.into()))?;
            let mut input = String::new();
            std::io::stdin()
                .read_line(&mut input)
                .map_err(|error| LlmCredentialPromptError::Other(error.into()))?;
            let trimmed = input.trim();
            if let Some(provider_id) = resolve_menu_selection(entries, trimmed) {
                return Ok(provider_id);
            }
            if attempt < MAX_ATTEMPTS {
                println!(
                    "Unrecognized provider `{trimmed}`; enter a number, provider id, or alias \
                     from the list above."
                );
            }
        }
        Err(LlmCredentialPromptError::Other(anyhow::anyhow!(
            "no valid provider selected after {MAX_ATTEMPTS} attempts"
        )))
    }

    fn api_key(&mut self, provider: &str) -> Result<String, LlmCredentialPromptError> {
        if !std::io::stdin().is_terminal() {
            return Err(LlmCredentialPromptError::NonInteractive);
        }
        // Re-prompt on a blank/whitespace-only answer rather than persisting
        // an empty key — a mis-timed Enter or accidental paste-then-clear
        // must never end up stored as `llm_provider_<id>_api_key`, silently
        // leaving the provider "configured" with a key that will fail every
        // request.
        const MAX_ATTEMPTS: u8 = 3;
        for attempt in 1..=MAX_ATTEMPTS {
            print!("{provider} API key (input hidden): ");
            std::io::stdout()
                .flush()
                .map_err(|error| LlmCredentialPromptError::Other(error.into()))?;
            let key = read_masked_line()
                .map_err(|error| LlmCredentialPromptError::Other(error.into()))?;
            println!();
            if !key.trim().is_empty() {
                return Ok(key);
            }
            if attempt < MAX_ATTEMPTS {
                println!("API key must not be blank; please try again.");
            }
        }
        Err(LlmCredentialPromptError::Other(anyhow::anyhow!(
            "no non-blank API key entered after {MAX_ATTEMPTS} attempts"
        )))
    }

    #[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
    fn model(
        &mut self,
        provider_id: &str,
        default_model: &str,
    ) -> Result<Option<String>, LlmCredentialPromptError> {
        if !std::io::stdin().is_terminal() {
            return Err(LlmCredentialPromptError::NonInteractive);
        }
        print!("{provider_id} model [{default_model}]: ");
        std::io::stdout()
            .flush()
            .map_err(|error| LlmCredentialPromptError::Other(error.into()))?;
        let mut input = String::new();
        std::io::stdin()
            .read_line(&mut input)
            .map_err(|error| LlmCredentialPromptError::Other(error.into()))?;
        let trimmed = input.trim();
        Ok(if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        })
    }
}

/// Resolve one line of menu input against `entries`: a 1-based menu number,
/// an exact provider id, or an alias — all case-insensitive for the id/alias
/// forms. Returns the selected entry's canonical provider id, or `None` when
/// nothing matches.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
fn resolve_menu_selection(
    entries: &[ironclaw_reborn_composition::ProviderMenuEntry],
    input: &str,
) -> Option<String> {
    if let Ok(number) = input.parse::<usize>() {
        if number >= 1 && number <= entries.len() {
            return Some(entries[number - 1].id.clone());
        }
        return None;
    }
    entries
        .iter()
        .find(|entry| {
            entry.id.eq_ignore_ascii_case(input)
                || entry
                    .aliases
                    .iter()
                    .any(|alias| alias.eq_ignore_ascii_case(input))
        })
        .map(|entry| entry.id.clone())
}

/// Read one line with terminal echo suppressed, showing `*` per character.
///
/// Ported from v1's `src/setup/prompts.rs` (`secret_input`/
/// `read_secret_line`) — same crossterm raw-mode key-event loop, including
/// its leading `drain_pending_events()` call, which discards any keystrokes
/// buffered before raw mode was entered (e.g. a stray Enter left over from
/// the previous plain-line `provider_menu()` prompt) so they can't be
/// replayed into the masked read and silently corrupt the captured key —
/// per this repo's "porting = copy, never depend" convention (v1 is read
/// for shape, not imported; `ironclaw_secrets::keychain::os_keychain_suppressed`
/// was ported into the Reborn stack the same way for the master-key work
/// `super::master_key` already does).
fn read_masked_line() -> std::io::Result<String> {
    use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
    use crossterm::{execute, style::Print, terminal};

    let mut input = String::new();
    terminal::enable_raw_mode()?;
    let result = (|| -> std::io::Result<()> {
        drain_pending_events();
        loop {
            if let Event::Key(KeyEvent {
                code,
                modifiers,
                kind: KeyEventKind::Press,
                ..
            }) = event::read()?
            {
                match code {
                    KeyCode::Enter => break,
                    KeyCode::Backspace if !input.is_empty() => {
                        input.pop();
                        execute!(std::io::stdout(), Print("\x08 \x08"))?;
                        std::io::stdout().flush()?;
                    }
                    KeyCode::Backspace => {}
                    KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::Interrupted,
                            "Ctrl-C",
                        ));
                    }
                    KeyCode::Char(c) => {
                        input.push(c);
                        execute!(std::io::stdout(), Print('*'))?;
                        std::io::stdout().flush()?;
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    })();
    terminal::disable_raw_mode()?;
    result?;
    Ok(input)
}

/// Discard any terminal input events buffered before raw mode was entered,
/// so a stray keystroke (typically a leftover Enter from a preceding
/// plain-line prompt) can never be replayed into the masked read that
/// follows. Ported from v1's `src/setup/prompts.rs::drain_pending_events`.
fn drain_pending_events() {
    use crossterm::event;
    while event::poll(std::time::Duration::ZERO).unwrap_or(false) {
        let _ = event::read();
    }
}

#[cfg(all(test, feature = "libsql", feature = "root-llm-provider"))]
mod tests {
    use ironclaw_reborn_composition::ProviderMenuEntry;

    use super::*;

    fn entries() -> Vec<ProviderMenuEntry> {
        vec![
            ProviderMenuEntry {
                id: "nearai".to_string(),
                display_name: "NEAR AI".to_string(),
                api_key_required: false,
                description: "multi-model access via NEAR account".to_string(),
                aliases: vec!["near_ai".to_string(), "near".to_string()],
            },
            ProviderMenuEntry {
                id: "openai".to_string(),
                display_name: "OpenAI".to_string(),
                api_key_required: true,
                description: "OpenAI GPT models (direct API)".to_string(),
                aliases: vec!["open_ai".to_string()],
            },
        ]
    }

    /// (i) Selection by 1-based menu number.
    #[test]
    fn resolve_menu_selection_by_number() {
        let entries = entries();
        assert_eq!(
            resolve_menu_selection(&entries, "1"),
            Some("nearai".to_string())
        );
        assert_eq!(
            resolve_menu_selection(&entries, "2"),
            Some("openai".to_string())
        );
    }

    /// (i) Selection by exact provider id, case-insensitively.
    #[test]
    fn resolve_menu_selection_by_id() {
        let entries = entries();
        assert_eq!(
            resolve_menu_selection(&entries, "openai"),
            Some("openai".to_string())
        );
        assert_eq!(
            resolve_menu_selection(&entries, "OpenAI"),
            Some("openai".to_string())
        );
    }

    /// (i) Selection by alias, case-insensitively.
    #[test]
    fn resolve_menu_selection_by_alias() {
        let entries = entries();
        assert_eq!(
            resolve_menu_selection(&entries, "open_ai"),
            Some("openai".to_string())
        );
        assert_eq!(
            resolve_menu_selection(&entries, "NEAR"),
            Some("nearai".to_string())
        );
    }

    /// (ii)/(v) Garbage input, an out-of-range number, and a menu-excluded
    /// provider id (not present in `entries`, e.g. `bedrock`) all fail to
    /// resolve — the caller (`provision_llm_credentials`) is responsible
    /// for the retry-then-error behavior; this pins the pure matching logic
    /// underneath it.
    #[test]
    fn resolve_menu_selection_rejects_unknown_input() {
        let entries = entries();
        assert_eq!(resolve_menu_selection(&entries, "0"), None);
        assert_eq!(resolve_menu_selection(&entries, "99"), None);
        assert_eq!(resolve_menu_selection(&entries, "garbage"), None);
        assert_eq!(resolve_menu_selection(&entries, "bedrock"), None);
    }
}
