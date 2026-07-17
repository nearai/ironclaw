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

    /// Ask a yes/no `question`, defaulting to yes on a blank answer (`[Y/n]`
    /// framing). Used by onboard's env-detect-and-confirm step: "Found
    /// `<provider>` configured in environment — use it?"
    fn confirm(&mut self, question: &str) -> Result<bool, LlmCredentialPromptError>;

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
        if terminal_supports_arrow_menu() {
            match run_arrow_menu(entries) {
                Ok(ArrowMenuOutcome::Selected(provider_id)) => return Ok(provider_id),
                Ok(ArrowMenuOutcome::Cancelled) => {
                    return Err(LlmCredentialPromptError::Other(anyhow::anyhow!(
                        "onboarding cancelled at provider selection"
                    )));
                }
                Ok(ArrowMenuOutcome::FallBackTyped) => {
                    // Simplest robust hand-off (see this module's doc): drop
                    // straight to the plain numbered-list + line-read prompt
                    // below rather than threading the already-typed
                    // character through a second input mode.
                }
                Err(error) => {
                    tracing::debug!(
                        %error,
                        "arrow-key provider menu unavailable mid-flight; falling back to the \
                         numbered list"
                    );
                }
            }
        }
        provider_menu_typed(entries)
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

    fn confirm(&mut self, question: &str) -> Result<bool, LlmCredentialPromptError> {
        if !std::io::stdin().is_terminal() {
            return Err(LlmCredentialPromptError::NonInteractive);
        }
        print!("{question} [Y/n]: ");
        std::io::stdout()
            .flush()
            .map_err(|error| LlmCredentialPromptError::Other(error.into()))?;
        let mut input = String::new();
        std::io::stdin()
            .read_line(&mut input)
            .map_err(|error| LlmCredentialPromptError::Other(error.into()))?;
        let trimmed = input.trim();
        Ok(trimmed.is_empty()
            || trimmed.eq_ignore_ascii_case("y")
            || trimmed.eq_ignore_ascii_case("yes"))
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

/// The bracketed note shown after a menu entry's description: nothing for a
/// required-key provider (the description already implies a key prompt
/// follows), `" (no API key needed)"` otherwise. Shared by both menu
/// renderers ([`provider_menu_typed`] and [`render_menu`]) so the two can
/// never drift on this text.
///
/// `nearai`'s `ProviderMenuEntry::api_key_required` is `true` here (a
/// menu-level override — see `RebornProviderAdmin::menu_entries`'s doc),
/// even though the raw catalog entry (`providers.json`) marks it optional:
/// reborn has no session-token auth wired, so it is required-key like every
/// other menu entry and gets no special note.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
fn menu_entry_key_note(entry: &ironclaw_reborn_composition::ProviderMenuEntry) -> &'static str {
    if entry.api_key_required {
        ""
    } else {
        " (no API key needed)"
    }
}

/// Plain numbered-list + line-read provider prompt — the ENTIRE original
/// `provider_menu` body (byte-identical besides its `&mut self` → free-fn
/// signature change), extracted so it can serve two roles unchanged: (1) the
/// exact fallback `StdinPromptSource::provider_menu` uses whenever the
/// interactive arrow-key menu can't run (not a TTY — though that's already
/// caught earlier by the caller, raw mode failed to enable, or `TERM=dumb`),
/// and (2) the hand-off target when an operator starts typing during arrow
/// mode instead of using Up/Down (see [`ArrowMenuOutcome::FallBackTyped`]).
/// `resolve_menu_selection` is untouched by this port — only the I/O
/// wrapper moved.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
fn provider_menu_typed(
    entries: &[ironclaw_reborn_composition::ProviderMenuEntry],
) -> Result<String, LlmCredentialPromptError> {
    println!("Select an LLM provider:");
    for (index, entry) in entries.iter().enumerate() {
        let key_note = menu_entry_key_note(entry);
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
                "Unrecognized provider `{trimmed}`; enter a number, provider id, or alias from \
                 the list above."
            );
        }
    }
    Err(LlmCredentialPromptError::Other(anyhow::anyhow!(
        "no valid provider selected after {MAX_ATTEMPTS} attempts"
    )))
}

/// `true` when the current terminal looks capable of the interactive
/// arrow-key menu: both stdin and stdout must be real terminals (not a
/// pipe/redirect — the arrow menu writes cursor-movement escapes to
/// stdout), and `TERM` must not be `dumb` (a terminal that explicitly
/// disclaims escape-sequence support, e.g. some CI log collectors). This is
/// a cheap pre-check only — [`run_arrow_menu`] can still fail if
/// `enable_raw_mode()` itself errors (e.g. stdin/stdout got reassigned
/// between this check and that call), which [`PromptSource::provider_menu`]
/// treats identically to this check returning `false`: fall back to
/// [`provider_menu_typed`].
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
fn terminal_supports_arrow_menu() -> bool {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return false;
    }
    !std::env::var("TERM").is_ok_and(|term| term == "dumb")
}

/// RAII guard for `crossterm::terminal::enable_raw_mode()`: disables raw
/// mode in [`Drop`] so every exit path out of [`run_arrow_menu`] — a normal
/// return, an early `?` on an I/O error mid-loop, or an unwinding panic —
/// leaves the terminal back in cooked mode. [`read_masked_line`] instead
/// pairs `enable_raw_mode`/`disable_raw_mode` manually because it has
/// exactly one exit point right before returning; `run_arrow_menu` has
/// several (Up/Down keep looping, but Enter/Esc/Ctrl-C/typed-fallback/read
/// error all return from different places), so a guard is used instead of
/// repeating that pairing at each one.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
struct RawModeGuard;

#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
impl RawModeGuard {
    fn enable() -> std::io::Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        Ok(Self)
    }
}

#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

/// Classified key input for the interactive provider menu — the terminal
/// loop ([`run_arrow_menu`]) maps a raw `crossterm::event::KeyEvent` down to
/// one of these before handing it to the pure reducer, [`apply_menu_key`].
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuKey {
    Up,
    Down,
    Enter,
    /// Esc, or Ctrl-C.
    Cancel,
    /// Any other key press (digit, letter, punctuation, …) — the signal to
    /// hand off to the typed-line fallback.
    Other,
}

/// One outcome of applying a single [`MenuKey`] to the currently highlighted
/// index — pure and terminal-free so it's unit-tested directly (see this
/// module's `tests`) rather than only indirectly through [`run_arrow_menu`]'s
/// real terminal loop.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuStep {
    /// Up/Down: the new highlighted index (wraps at both ends — Up from
    /// index 0 wraps to the last entry, Down from the last entry wraps to
    /// 0), so an operator can reach any entry either direction without
    /// hitting a dead stop.
    Move(usize),
    /// Enter: the highlighted index was chosen.
    Select(usize),
    Cancel,
    FallBackTyped,
}

/// Pure key-event → selection-state reducer for the interactive provider
/// menu. `highlighted` and `len` (`entries.len()`, always `>= 1` — onboard
/// never calls `provider_menu` with an empty menu) come from the caller;
/// this function has no terminal or process-global state of its own.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
fn apply_menu_key(highlighted: usize, len: usize, key: MenuKey) -> MenuStep {
    debug_assert!(len > 0, "apply_menu_key requires a non-empty menu");
    match key {
        MenuKey::Up => MenuStep::Move((highlighted + len - 1) % len),
        MenuKey::Down => MenuStep::Move((highlighted + 1) % len),
        MenuKey::Enter => MenuStep::Select(highlighted),
        MenuKey::Cancel => MenuStep::Cancel,
        MenuKey::Other => MenuStep::FallBackTyped,
    }
}

/// Outcome of [`run_arrow_menu`]'s interactive loop.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
enum ArrowMenuOutcome {
    Selected(String),
    Cancelled,
    FallBackTyped,
}

/// Drive the interactive Up/Down/Enter provider menu in raw mode. Returns
/// `Ok` for every key-driven outcome (selection, cancellation, or a typed
/// hand-off); an `Err` means raw mode or a terminal I/O call itself failed
/// mid-flight, which the caller ([`StdinPromptSource::provider_menu`])
/// treats the same as [`terminal_supports_arrow_menu`] returning `false`
/// up front: fall back to [`provider_menu_typed`].
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
fn run_arrow_menu(
    entries: &[ironclaw_reborn_composition::ProviderMenuEntry],
) -> std::io::Result<ArrowMenuOutcome> {
    use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
    use crossterm::{cursor, execute, style::Print};

    let _raw_mode = RawModeGuard::enable()?;
    drain_pending_events();

    let mut stdout = std::io::stdout();
    let mut highlighted = 0usize;
    render_menu(&mut stdout, entries, highlighted, false)?;

    loop {
        let Event::Key(KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            ..
        }) = event::read()?
        else {
            continue;
        };
        let key = match code {
            KeyCode::Up => MenuKey::Up,
            KeyCode::Down => MenuKey::Down,
            KeyCode::Enter => MenuKey::Enter,
            KeyCode::Esc => MenuKey::Cancel,
            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => MenuKey::Cancel,
            _ => MenuKey::Other,
        };
        match apply_menu_key(highlighted, entries.len(), key) {
            MenuStep::Move(next) => {
                highlighted = next;
                render_menu(&mut stdout, entries, highlighted, true)?;
            }
            MenuStep::Select(index) => {
                execute!(stdout, cursor::MoveToNextLine(1))?;
                return Ok(ArrowMenuOutcome::Selected(entries[index].id.clone()));
            }
            MenuStep::Cancel => {
                execute!(stdout, cursor::MoveToNextLine(1))?;
                return Ok(ArrowMenuOutcome::Cancelled);
            }
            MenuStep::FallBackTyped => {
                execute!(
                    stdout,
                    cursor::MoveToNextLine(1),
                    Print("switching to typed entry\r\n")
                )?;
                return Ok(ArrowMenuOutcome::FallBackTyped);
            }
        }
    }
}

/// Render (or, when `redraw` is `true`, re-render in place) the numbered
/// provider list with `highlighted` marked by a leading `>`. The header
/// line is printed once on the very first draw and never redrawn; each
/// `redraw` call moves the cursor back up exactly `entries.len()` lines
/// (the entry lines only — the header stays put above them) and clears
/// downward before reprinting, so the list appears to update in place
/// rather than scrolling.
#[cfg(all(feature = "libsql", feature = "root-llm-provider"))]
fn render_menu(
    stdout: &mut std::io::Stdout,
    entries: &[ironclaw_reborn_composition::ProviderMenuEntry],
    highlighted: usize,
    redraw: bool,
) -> std::io::Result<()> {
    use crossterm::{cursor, execute, style::Print, terminal};

    if redraw {
        execute!(
            stdout,
            cursor::MoveUp(entries.len() as u16),
            terminal::Clear(terminal::ClearType::FromCursorDown)
        )?;
    } else {
        execute!(
            stdout,
            Print(
                "Select an LLM provider (Up/Down + Enter; Esc to cancel; or type a number, id, \
                 or alias):\r\n"
            )
        )?;
    }
    for (index, entry) in entries.iter().enumerate() {
        let marker = if index == highlighted { ">" } else { " " };
        let key_note = menu_entry_key_note(entry);
        execute!(
            stdout,
            Print(format!(
                "{marker} {}. {} — {}{key_note}\r\n",
                index + 1,
                entry.display_name,
                entry.description
            ))
        )?;
    }
    stdout.flush()
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
                // Menu-level override (see `RebornProviderAdmin::menu_entries`'s
                // doc): reborn has no session-token auth wired, so nearai is
                // required-key on the onboard menu even though the raw
                // catalog entry marks it optional.
                api_key_required: true,
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

    /// Down from the last entry wraps to the first, and Up from the first
    /// wraps to the last — an operator can reach any entry from any
    /// starting point going either direction, never hitting a dead stop at
    /// either end of the list.
    #[test]
    fn apply_menu_key_up_down_wrap_at_both_ends() {
        assert_eq!(apply_menu_key(0, 3, MenuKey::Down), MenuStep::Move(1));
        assert_eq!(apply_menu_key(1, 3, MenuKey::Down), MenuStep::Move(2));
        assert_eq!(apply_menu_key(2, 3, MenuKey::Down), MenuStep::Move(0));
        assert_eq!(apply_menu_key(0, 3, MenuKey::Up), MenuStep::Move(2));
        assert_eq!(apply_menu_key(2, 3, MenuKey::Up), MenuStep::Move(1));
    }

    /// A single-entry menu's Up/Down both stay put (wrap onto themselves) —
    /// the `0 + 1 - 1 = 0` / `0 + 1 = 1 % 1 = 0` arithmetic must not panic
    /// or index out of range at the degenerate `len == 1` case.
    #[test]
    fn apply_menu_key_single_entry_menu_stays_put() {
        assert_eq!(apply_menu_key(0, 1, MenuKey::Up), MenuStep::Move(0));
        assert_eq!(apply_menu_key(0, 1, MenuKey::Down), MenuStep::Move(0));
    }

    /// Enter selects whichever index is currently highlighted.
    #[test]
    fn apply_menu_key_enter_selects_highlighted_index() {
        assert_eq!(apply_menu_key(0, 3, MenuKey::Enter), MenuStep::Select(0));
        assert_eq!(apply_menu_key(2, 3, MenuKey::Enter), MenuStep::Select(2));
    }

    /// Cancel (Esc or Ctrl-C, already classified into `MenuKey::Cancel` by
    /// the terminal loop) always cancels regardless of the highlighted
    /// index.
    #[test]
    fn apply_menu_key_cancel_is_position_independent() {
        assert_eq!(apply_menu_key(0, 3, MenuKey::Cancel), MenuStep::Cancel);
        assert_eq!(apply_menu_key(2, 3, MenuKey::Cancel), MenuStep::Cancel);
    }

    /// Any other key press hands off to the typed-line fallback rather than
    /// being silently ignored or erroring — see
    /// `ArrowMenuOutcome::FallBackTyped`'s doc for why dropping straight to
    /// `provider_menu_typed` (rather than threading the pressed character
    /// through) is the chosen "simplest robust" behavior.
    #[test]
    fn apply_menu_key_other_falls_back_to_typed_entry() {
        assert_eq!(
            apply_menu_key(1, 3, MenuKey::Other),
            MenuStep::FallBackTyped
        );
    }
}
