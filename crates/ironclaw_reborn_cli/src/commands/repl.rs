use std::io::{self, IsTerminal, Write};

use clap::Args;

use crate::context::RebornCliContext;
use crate::runtime::RuntimeInputOptions;

/// Start an interactive Reborn CLI session backed by the composed runtime.
#[derive(Debug, Args)]
pub(crate) struct ReplCommand {
    /// Confirm trusted-laptop host filesystem access for local-dev-yolo.
    #[arg(long = "confirm-host-access")]
    confirm_host_access: bool,

    /// Skip the first-run model-setup prompt shown when no LLM is configured.
    #[arg(long = "no-setup")]
    no_setup: bool,
}

impl ReplCommand {
    pub(crate) fn execute(self, context: RebornCliContext) -> anyhow::Result<()> {
        crate::runtime::init_tracing();
        // First-run model setup: only when interactive and not opted out. Runs
        // before the runtime starts so the chosen provider/key are live for the
        // session. Skipped for pipes/CI so scripted `repl` keeps working.
        if !self.no_setup && io::stdin().is_terminal() {
            maybe_run_model_setup(&context)?;
        }
        crate::runtime::execute(
            context,
            None,
            RuntimeInputOptions {
                confirm_host_access: self.confirm_host_access,
            },
        )
    }
}

/// Parse a provider-menu answer. `None` = skip; otherwise a 1-based index in
/// range.
fn parse_provider_choice(input: &str, count: usize) -> Option<usize> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    trimmed
        .parse::<usize>()
        .ok()
        .filter(|n| *n >= 1 && *n <= count)
}

#[cfg(not(feature = "root-llm-provider"))]
fn maybe_run_model_setup(_context: &RebornCliContext) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(feature = "root-llm-provider")]
fn maybe_run_model_setup(context: &RebornCliContext) -> anyhow::Result<()> {
    use ironclaw_reborn_composition::RebornProviderAdmin;

    // An env-driven selection (`LLM_BACKEND=...`) is already active — don't nag.
    if std::env::var("LLM_BACKEND")
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
    {
        return Ok(());
    }

    let admin = RebornProviderAdmin::new(context.boot_config().clone());
    if let Some(selection) = admin.status()?.default {
        // A provider is already selected. Prompt for its API key only when the
        // provider actually *requires* one and the env var is missing —
        // otherwise we'd nag for providers that need no key (Ollama) or use a
        // different credential (nearai / OAuth providers authenticate via a
        // session token or login, not `*_API_KEY`).
        if let Some(env_var) = selection.api_key_env.as_deref()
            && std::env::var_os(env_var).is_none()
            && provider_requires_api_key(&admin, selection.provider_id.as_deref())
        {
            prompt_and_set_api_key(env_var, selection.provider_id.as_deref());
        }
        return Ok(());
    }

    let providers = admin.list(None, false)?.providers;
    if providers.is_empty() {
        return Ok(());
    }

    println!("\nNo AI model configured yet. Choose a provider to get started");
    println!(
        "(press Enter to skip and set one later with `ironclaw-reborn models set-provider`):\n"
    );
    for (idx, provider) in providers.iter().enumerate() {
        println!(
            "  {}) {:<20} {}",
            idx + 1,
            provider.id,
            provider.description
        );
    }
    // Re-prompt on an invalid entry; only an empty line skips and EOF exits.
    let index = loop {
        print!("\nProvider [1-{} / Enter to skip]: ", providers.len());
        io::stdout().flush()?;
        let mut answer = String::new();
        if io::stdin().read_line(&mut answer)? == 0 {
            return Ok(()); // EOF
        }
        if answer.trim().is_empty() {
            return Ok(()); // skip
        }
        if let Some(index) = parse_provider_choice(&answer, providers.len()) {
            break index;
        }
        println!("please enter a number between 1 and {}.", providers.len());
    };
    let provider = &providers[index - 1];

    print!("Model [Enter for default `{}`]: ", provider.default_model);
    io::stdout().flush()?;
    let mut model = String::new();
    io::stdin().read_line(&mut model)?;
    let model = model.trim();
    let model_arg = (!model.is_empty()).then_some(model);

    let outcome = admin.set_provider(&provider.id, model_arg)?;
    println!(
        "configured: provider `{}`, model `{}`",
        outcome.provider_id, outcome.model
    );

    if outcome.missing_api_key
        && let Some(env_var) = outcome.api_key_env.as_deref()
    {
        prompt_and_set_api_key(env_var, Some(&outcome.provider_id));
    }
    println!();
    Ok(())
}

/// Whether the given provider genuinely requires an API key (vs. Ollama, or a
/// session-token / OAuth provider like nearai). Looks up the provider's
/// metadata; defaults to `false` so an unknown/unreadable provider never nags.
#[cfg(feature = "root-llm-provider")]
fn provider_requires_api_key(
    admin: &ironclaw_reborn_composition::RebornProviderAdmin,
    provider_id: Option<&str>,
) -> bool {
    let Some(id) = provider_id else {
        return false;
    };
    admin
        .list(Some(id), true)
        .ok()
        .and_then(|list| list.providers.into_iter().next())
        .and_then(|provider| provider.metadata)
        .map(|metadata| metadata.api_key_required)
        .unwrap_or(false)
}

/// Prompt for an API key and set it in the process env for this session.
#[cfg(feature = "root-llm-provider")]
fn prompt_and_set_api_key(env_var: &str, provider: Option<&str>) {
    let label = provider.unwrap_or("the selected provider");
    print!(
        "\n{label} needs an API key. Enter value for `{env_var}` (used this session; Enter to skip): "
    );
    if io::stdout().flush().is_err() {
        return;
    }
    let mut key = String::new();
    if io::stdin().read_line(&mut key).is_err() {
        return;
    }
    let key = key.trim();
    if key.is_empty() {
        println!("no key entered — set `{env_var}` in your environment before chatting.");
        return;
    }
    // SAFETY: single-threaded CLI startup, before the runtime spawns any threads
    // that read process environment variables.
    unsafe { std::env::set_var(env_var, key) };
    println!("key set for this session. To persist it, add `export {env_var}=...` to your shell.");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_choice_parses_index_and_skip() {
        assert_eq!(parse_provider_choice("1", 3), Some(1));
        assert_eq!(parse_provider_choice(" 3 \n", 3), Some(3));
        assert_eq!(parse_provider_choice("", 3), None); // Enter = skip
        assert_eq!(parse_provider_choice("\n", 3), None);
        assert_eq!(parse_provider_choice("0", 3), None); // out of range
        assert_eq!(parse_provider_choice("4", 3), None); // out of range
        assert_eq!(parse_provider_choice("openai", 3), None); // non-numeric
    }
}
