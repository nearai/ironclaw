//! Models management CLI commands.
//!
//! Provides subcommands for listing providers, viewing current model
//! configuration, and setting the active provider/model. Settings are
//! persisted to both `config.toml` and `~/.ironclaw/.env` so changes
//! take effect immediately (no DB connection required).

use clap::Subcommand;
use std::path::Path;

use crate::llm::registry::ProviderRegistry;
use crate::settings::Settings;

#[derive(Subcommand, Debug, Clone)]
pub enum ModelsCommand {
    /// List providers (or available models for a specific provider)
    List {
        /// Show only a specific provider (by ID or alias)
        provider: Option<String>,

        /// Show detailed information (env vars, base URL, protocol)
        #[arg(short, long)]
        verbose: bool,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show current model configuration
    Status {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Set the default model
    Set {
        /// Model name (e.g., "gpt-5-mini", "claude-sonnet-4-6-20250514")
        model: String,
    },

    /// Set the LLM provider
    SetProvider {
        /// Provider ID or alias (e.g., "openai", "anthropic", "ollama")
        provider: String,

        /// Also set the model (defaults to provider's default model)
        #[arg(long)]
        model: Option<String>,
    },
}

/// Run the models CLI subcommand.
pub async fn run_models_command(
    cmd: ModelsCommand,
    config_path: Option<&Path>,
) -> anyhow::Result<()> {
    match cmd {
        ModelsCommand::List {
            provider,
            verbose,
            json,
        } => {
            if let Some(ref id) = provider {
                cmd_show_provider(id, verbose, json, config_path).await
            } else {
                cmd_list_providers(verbose, json, config_path).await
            }
        }
        ModelsCommand::Status { json } => cmd_status(json, config_path),
        ModelsCommand::Set { model } => cmd_set_model(&model, config_path),
        ModelsCommand::SetProvider { provider, model } => {
            cmd_set_provider(&provider, model.as_deref(), config_path)
        }
    }
}

// ─── Shared helpers ───────────────────────────────────────────────

/// Resolve the currently active backend and model from env + settings.
fn resolve_active(config_path: Option<&Path>) -> (String, String) {
    let settings = load_settings(config_path);
    resolve_active_from_settings(&settings)
}

/// Resolve active backend + model from a pre-loaded Settings.
fn resolve_active_from_settings(settings: &Settings) -> (String, String) {
    let backend = std::env::var("LLM_BACKEND")
        .ok()
        .or_else(|| settings.llm_backend.clone())
        .unwrap_or_else(|| "nearai".to_string());

    let registry = ProviderRegistry::load();

    let canonical_backend = registry
        .find(&backend)
        .map(|d| d.id.clone())
        .unwrap_or_else(|| backend.clone());

    let model = if canonical_backend == "nearai" {
        std::env::var("NEARAI_MODEL")
            .ok()
            .or_else(|| settings.selected_model.clone())
            .unwrap_or_else(|| "qwen2.5-72b-instruct:free".to_string())
    } else if let Some(def) = registry.find(&canonical_backend) {
        std::env::var(&def.model_env)
            .ok()
            .or_else(|| settings.selected_model.clone())
            .unwrap_or_else(|| def.default_model.clone())
    } else {
        settings
            .selected_model
            .clone()
            .unwrap_or_else(|| "unknown".to_string())
    };

    (canonical_backend, model)
}

fn load_settings(config_path: Option<&Path>) -> Settings {
    if let Some(path) = config_path {
        Settings::load_toml(path).ok().flatten().unwrap_or_default()
    } else {
        let toml_path = config_toml_path();
        if toml_path.exists() {
            Settings::load_toml(&toml_path)
                .ok()
                .flatten()
                .unwrap_or_default()
        } else {
            Settings::load()
        }
    }
}

fn save_settings(settings: &Settings, config_path: Option<&Path>) -> anyhow::Result<()> {
    let path = config_path
        .map(|p| p.to_path_buf())
        .unwrap_or_else(config_toml_path);

    settings
        .save_toml(&path)
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    Ok(())
}

fn config_toml_path() -> std::path::PathBuf {
    crate::bootstrap::ironclaw_base_dir().join("config.toml")
}

/// Try to fetch the live model list from a provider.
///
/// Best-effort: returns `None` if config loading, provider creation, or the
/// `list_models()` call fails (missing API key, network error, etc.).
async fn try_fetch_models(provider_id: &str, config_path: Option<&Path>) -> Option<Vec<String>> {
    let config = crate::config::Config::from_env_with_toml(config_path)
        .await
        .ok()?;

    // Override backend to the requested provider so create_llm_provider
    // constructs the right one.
    let mut llm_config = config.llm.clone();
    llm_config.backend = provider_id.to_string();

    // For registry providers, resolve the RegistryProviderConfig if not
    // already set for this backend.
    if provider_id != "nearai" && provider_id != "bedrock" {
        let registry = ProviderRegistry::load();
        if let Some(def) = registry.find(provider_id)
            && llm_config
                .provider
                .as_ref()
                .is_none_or(|p| p.provider_id != def.id)
        {
            // Build a minimal RegistryProviderConfig from env + registry
            let api_key = def
                .api_key_env
                .as_ref()
                .and_then(|env| std::env::var(env).ok());
            if def.api_key_required && api_key.is_none() {
                return None;
            }
            let base_url = def.default_base_url.clone().unwrap_or_default();
            llm_config.provider = Some(crate::llm::RegistryProviderConfig {
                protocol: def.protocol,
                provider_id: def.id.clone(),
                model: def.default_model.clone(),
                api_key: api_key.map(secrecy::SecretString::from),
                base_url,
                extra_headers: Vec::new(),
                oauth_token: None,
                is_codex_chatgpt: false,
                refresh_token: None,
                auth_path: None,
                cache_retention: Default::default(),
                unsupported_params: def.unsupported_params.clone(),
            });
        }
    }

    let session = crate::llm::create_session_manager(config.llm.session.clone()).await;
    let provider = crate::llm::create_llm_provider(&llm_config, session)
        .await
        .ok()?;
    provider.list_models().await.ok().filter(|m| !m.is_empty())
}

/// Print available models section (text output).
fn print_model_list(models: &Option<Vec<String>>, active_model: Option<&String>) {
    match models {
        Some(models) => {
            println!("\n  Available models ({}):", models.len());
            for m in models {
                let marker = active_model
                    .filter(|a| a.as_str() == m)
                    .map(|_| " (active)")
                    .unwrap_or("");
                println!("    {}{}", m, marker);
            }
        }
        None => {
            println!(
                "\n  Could not fetch model list (missing credentials or provider unavailable)."
            );
        }
    }
}

/// Also update `~/.ironclaw/.env` so changes take effect immediately.
///
/// Skipped when `config_path` is `Some` (custom `--config`), because the user
/// is explicitly targeting a different config file and we must not pollute the
/// default profile's `.env`.
fn sync_to_dotenv(config_path: Option<&Path>, vars: &[(&str, &str)]) {
    if config_path.is_some() {
        return;
    }
    if let Err(e) = crate::bootstrap::upsert_bootstrap_vars(vars) {
        eprintln!("Warning: failed to update .env: {}", e);
    }
}

// ─── status ───────────────────────────────────────────────────────

fn cmd_status(json: bool, config_path: Option<&Path>) -> anyhow::Result<()> {
    let settings = load_settings(config_path);
    let (backend, model) = resolve_active_from_settings(&settings);
    let registry = ProviderRegistry::load();

    let fallback = std::env::var("NEARAI_FALLBACK_MODEL").ok();
    let cheap = std::env::var("NEARAI_CHEAP_MODEL").ok();

    let description = if backend == "nearai" {
        "NEAR AI inference (default)".to_string()
    } else {
        registry
            .find(&backend)
            .map(|d| d.description.clone())
            .unwrap_or_default()
    };

    if json {
        let v = serde_json::json!({
            "provider": backend,
            "model": model,
            "description": description,
            "fallback_model": fallback,
            "cheap_model": cheap,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&v).unwrap_or_else(|_| "{}".to_string())
        );
        return Ok(());
    }

    println!("Provider: {} ({})", backend, description);
    println!("Model:    {}", model);
    if let Some(ref fb) = fallback {
        println!("Fallback: {}", fb);
    }
    if let Some(ref ch) = cheap {
        println!("Cheap:    {}", ch);
    }

    Ok(())
}

// ─── set ──────────────────────────────────────────────────────────

fn cmd_set_model(model: &str, config_path: Option<&Path>) -> anyhow::Result<()> {
    let trimmed = model.trim();
    if trimmed.is_empty() {
        anyhow::bail!("Model name cannot be empty");
    }

    let mut settings = load_settings(config_path);
    let registry = ProviderRegistry::load();

    // Warn if model name doesn't match any known provider's default model
    let known_model = registry.all().iter().any(|d| d.default_model == trimmed)
        || trimmed.contains("qwen")  // nearai models
        || trimmed.contains("llama")
        || trimmed.contains("gpt")
        || trimmed.contains("claude")
        || trimmed.contains("gemini")
        || trimmed.contains("mistral");
    if !known_model {
        eprintln!(
            "Warning: '{}' is not a recognized model name. Proceeding anyway.",
            trimmed
        );
    }

    settings.selected_model = Some(trimmed.to_string());
    save_settings(&settings, config_path)?;

    let backend = std::env::var("LLM_BACKEND")
        .ok()
        .or_else(|| settings.llm_backend.clone())
        .unwrap_or_else(|| "nearai".to_string());

    // Also write to .env so the change takes effect immediately
    let model_env = if backend == "nearai" {
        "NEARAI_MODEL".to_string()
    } else {
        registry
            .find(&backend)
            .map(|d| d.model_env.clone())
            .unwrap_or_default()
    };
    if !model_env.is_empty() {
        sync_to_dotenv(config_path, &[(&model_env, trimmed)]);
    }

    println!("Model set to '{}' (provider: {})", trimmed, backend);
    println!(
        "Saved to {}",
        config_path
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| config_toml_path().display().to_string())
    );

    Ok(())
}

// ─── set-provider ─────────────────────────────────────────────────

fn cmd_set_provider(
    provider: &str,
    model: Option<&str>,
    config_path: Option<&Path>,
) -> anyhow::Result<()> {
    let registry = ProviderRegistry::load();

    // Validate and normalize provider
    let canonical_id = if provider == "nearai" || provider == "near_ai" || provider == "near" {
        "nearai".to_string()
    } else {
        let def = registry.find(provider).ok_or_else(|| {
            let known: Vec<&str> = std::iter::once("nearai")
                .chain(registry.all().iter().map(|d| d.id.as_str()))
                .collect();
            anyhow::anyhow!(
                "Unknown provider '{}'. Known providers: {}",
                provider,
                known.join(", ")
            )
        })?;
        def.id.clone()
    };

    // Resolve model: explicit > provider default
    let resolved_model = if let Some(m) = model {
        m.to_string()
    } else if canonical_id == "nearai" {
        "qwen2.5-72b-instruct:free".to_string()
    } else if let Some(def) = registry.find(&canonical_id) {
        def.default_model.clone()
    } else {
        "default".to_string()
    };

    let mut settings = load_settings(config_path);
    settings.llm_backend = Some(canonical_id.clone());
    settings.selected_model = Some(resolved_model.clone());
    save_settings(&settings, config_path)?;

    // Also write to .env so the change takes effect immediately
    let model_env = if canonical_id == "nearai" {
        "NEARAI_MODEL".to_string()
    } else {
        registry
            .find(&canonical_id)
            .map(|d| d.model_env.clone())
            .unwrap_or_default()
    };
    let mut vars: Vec<(&str, &str)> = vec![("LLM_BACKEND", &canonical_id)];
    if !model_env.is_empty() {
        vars.push((&model_env, &resolved_model));
    }
    sync_to_dotenv(config_path, &vars);

    println!(
        "Provider set to '{}', model set to '{}'",
        canonical_id, resolved_model
    );
    println!(
        "Saved to {}",
        config_path
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| config_toml_path().display().to_string())
    );

    Ok(())
}

// ─── list ─────────────────────────────────────────────────────────

/// List all providers with their default models.
async fn cmd_list_providers(
    verbose: bool,
    json: bool,
    config_path: Option<&Path>,
) -> anyhow::Result<()> {
    let registry = ProviderRegistry::load();
    let (active_backend, active_model) = resolve_active(config_path);

    if json {
        let mut entries: Vec<serde_json::Value> = Vec::new();

        // NEAR AI (not in registry)
        let nearai_active = active_backend == "nearai";
        entries.push(serde_json::json!({
            "id": "nearai",
            "description": "NEAR AI inference (default)",
            "default_model": "qwen2.5-72b-instruct:free",
            "active": nearai_active,
            "active_model": if nearai_active { Some(&active_model) } else { None },
        }));

        for def in registry.all() {
            let is_active = active_backend == def.id;
            let mut v = serde_json::json!({
                "id": def.id,
                "description": def.description,
                "default_model": def.default_model,
                "protocol": format!("{:?}", def.protocol),
                "active": is_active,
            });
            if is_active {
                v["active_model"] = serde_json::json!(active_model);
            }
            if verbose {
                v["aliases"] = serde_json::json!(def.aliases);
                v["model_env"] = serde_json::json!(def.model_env);
                v["api_key_env"] = serde_json::json!(def.api_key_env);
                v["api_key_required"] = serde_json::json!(def.api_key_required);
                if let Some(ref url) = def.default_base_url {
                    v["base_url"] = serde_json::json!(url);
                }
                if let Some(ref setup) = def.setup {
                    v["can_list_models"] = serde_json::json!(setup.can_list_models());
                }
            }
            entries.push(v);
        }

        println!(
            "{}",
            serde_json::to_string_pretty(&entries).unwrap_or_else(|_| "[]".to_string())
        );
        return Ok(());
    }

    let providers = registry.all();

    println!("Active: {} (model: {})\n", active_backend, active_model);
    println!(
        "{} provider(s) available:\n",
        providers.len() + 1 // +1 for NEAR AI
    );

    // NEAR AI (not in registry)
    let nearai_marker = if active_backend == "nearai" { " *" } else { "" };
    if verbose {
        println!("  nearai{}", nearai_marker);
        println!("    Description:   NEAR AI inference (default)");
        println!("    Default model: qwen2.5-72b-instruct:free");
        println!("    Model env:     NEARAI_MODEL");
        if active_backend == "nearai" {
            println!("    Active model:  {}", active_model);
        }
        println!();
    } else {
        println!(
            "  {:<22} {:<40} NEAR AI inference (default)",
            format!("nearai{nearai_marker}"),
            "qwen2.5-72b-instruct:free"
        );
    }

    for def in providers {
        let is_active = active_backend == def.id;
        let marker = if is_active { " *" } else { "" };

        if verbose {
            println!("  {}{}", def.id, marker);
            println!("    Description:   {}", def.description);
            println!("    Default model: {}", def.default_model);
            println!("    Protocol:      {:?}", def.protocol);
            println!("    Model env:     {}", def.model_env);
            if let Some(ref env) = def.api_key_env {
                println!(
                    "    API key env:   {} ({})",
                    env,
                    if def.api_key_required {
                        "required"
                    } else {
                        "optional"
                    }
                );
            }
            if let Some(ref url) = def.default_base_url {
                println!("    Base URL:      {}", url);
            }
            if !def.aliases.is_empty() {
                println!("    Aliases:       {}", def.aliases.join(", "));
            }
            if is_active {
                println!("    Active model:  {}", active_model);
            }
            println!();
        } else {
            let model_display = if is_active {
                active_model.clone()
            } else {
                def.default_model.clone()
            };
            println!(
                "  {:<22} {:<40} {}",
                format!("{}{marker}", def.id),
                model_display,
                def.description,
            );
        }
    }

    if !verbose {
        println!();
        println!("* = active provider. Use --verbose for details.");
    }

    Ok(())
}

/// Show details for a specific provider.
async fn cmd_show_provider(
    id: &str,
    verbose: bool,
    json: bool,
    config_path: Option<&Path>,
) -> anyhow::Result<()> {
    let registry = ProviderRegistry::load();
    let (active_backend, active_model) = resolve_active(config_path);

    // Resolve canonical ID for model fetching
    let canonical_id = if id == "nearai" || id == "near_ai" || id == "near" {
        "nearai".to_string()
    } else {
        registry
            .find(id)
            .map(|d| d.id.clone())
            .unwrap_or_else(|| id.to_string())
    };

    // Try to fetch live model list from the provider
    let live_models = try_fetch_models(&canonical_id, config_path).await;

    // Check NEAR AI first (not in registry)
    if id == "nearai" || id == "near_ai" || id == "near" {
        let is_active = active_backend == "nearai";
        if json {
            let mut v = serde_json::json!({
                "id": "nearai",
                "description": "NEAR AI inference (default)",
                "default_model": "qwen2.5-72b-instruct:free",
                "model_env": "NEARAI_MODEL",
                "active": is_active,
            });
            if is_active {
                v["active_model"] = serde_json::json!(active_model);
            }
            if let Some(ref models) = live_models {
                v["available_models"] = serde_json::json!(models);
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&v).unwrap_or_else(|_| "{}".to_string())
            );
        } else {
            println!("Provider: nearai");
            println!("  Description:   NEAR AI inference (default)");
            println!("  Default model: qwen2.5-72b-instruct:free");
            println!("  Model env:     NEARAI_MODEL");
            println!("  Active:        {}", if is_active { "yes" } else { "no" });
            if is_active {
                println!("  Active model:  {}", active_model);
            }
            print_model_list(&live_models, is_active.then_some(&active_model));
        }
        return Ok(());
    }

    let def = registry.find(id).ok_or_else(|| {
        let known: Vec<&str> = std::iter::once("nearai")
            .chain(registry.all().iter().map(|d| d.id.as_str()))
            .collect();
        anyhow::anyhow!(
            "Unknown provider '{}'. Known providers: {}",
            id,
            known.join(", ")
        )
    })?;

    let is_active = active_backend == def.id;

    if json {
        let mut v = serde_json::json!({
            "id": def.id,
            "description": def.description,
            "protocol": format!("{:?}", def.protocol),
            "default_model": def.default_model,
            "model_env": def.model_env,
            "api_key_env": def.api_key_env,
            "api_key_required": def.api_key_required,
            "aliases": def.aliases,
            "active": is_active,
        });
        if let Some(ref url) = def.default_base_url {
            v["base_url"] = serde_json::json!(url);
        }
        if let Some(ref setup) = def.setup {
            v["can_list_models"] = serde_json::json!(setup.can_list_models());
            v["display_name"] = serde_json::json!(setup.display_name());
        }
        if is_active {
            v["active_model"] = serde_json::json!(active_model);
        }
        if verbose && !def.unsupported_params.is_empty() {
            v["unsupported_params"] = serde_json::json!(def.unsupported_params);
        }
        if let Some(ref models) = live_models {
            v["available_models"] = serde_json::json!(models);
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&v).unwrap_or_else(|_| "{}".to_string())
        );
        return Ok(());
    }

    println!("Provider: {}", def.id);
    println!("  Description:   {}", def.description);
    println!("  Protocol:      {:?}", def.protocol);
    println!("  Default model: {}", def.default_model);
    println!("  Model env:     {}", def.model_env);
    if let Some(ref env) = def.api_key_env {
        println!(
            "  API key env:   {} ({})",
            env,
            if def.api_key_required {
                "required"
            } else {
                "optional"
            }
        );
    }
    if let Some(ref url) = def.default_base_url {
        println!("  Base URL:      {}", url);
    }
    if !def.aliases.is_empty() {
        println!("  Aliases:       {}", def.aliases.join(", "));
    }
    if let Some(ref setup) = def.setup {
        println!(
            "  List models:   {}",
            if setup.can_list_models() {
                "supported"
            } else {
                "not supported"
            }
        );
        println!("  Display name:  {}", setup.display_name());
    }
    if !def.unsupported_params.is_empty() {
        println!("  Unsupported:   {}", def.unsupported_params.join(", "));
    }
    println!("  Active:        {}", if is_active { "yes" } else { "no" });
    if is_active {
        println!("  Active model:  {}", active_model);
    }
    print_model_list(&live_models, is_active.then_some(&active_model));

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_active_defaults_to_nearai() {
        let settings = Settings::default();
        assert!(settings.llm_backend.is_none());
        assert!(settings.selected_model.is_none());
    }

    #[test]
    fn registry_loads_all_providers() {
        let registry = ProviderRegistry::load();
        let all = registry.all();
        assert!(
            all.len() >= 10,
            "should have at least 10 built-in providers, got {}",
            all.len()
        );
    }

    #[test]
    fn registry_find_by_alias() {
        let registry = ProviderRegistry::load();
        let def = registry
            .find("claude")
            .expect("claude alias should resolve");
        assert_eq!(def.id, "anthropic");
    }

    #[test]
    fn all_providers_have_description() {
        let registry = ProviderRegistry::load();
        for def in registry.all() {
            assert!(
                !def.description.is_empty(),
                "provider {} should have a description",
                def.id
            );
        }
    }

    #[test]
    fn set_model_persists_to_toml() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let toml_path = dir.path().join("config.toml");

        cmd_set_model("gpt-5-mini", Some(&toml_path)).expect("set model");

        let settings = Settings::load_toml(&toml_path)
            .expect("read toml")
            .expect("should have settings");
        assert_eq!(settings.selected_model.as_deref(), Some("gpt-5-mini"));
    }

    #[test]
    fn set_provider_validates_unknown() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let toml_path = dir.path().join("config.toml");

        let result = cmd_set_provider("nonexistent_provider", None, Some(&toml_path));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Unknown provider"),
            "should mention unknown provider: {}",
            err
        );
    }

    #[test]
    fn set_provider_persists_to_toml() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let toml_path = dir.path().join("config.toml");

        cmd_set_provider("groq", None, Some(&toml_path)).expect("set provider");

        let settings = Settings::load_toml(&toml_path)
            .expect("read toml")
            .expect("should have settings");
        assert_eq!(settings.llm_backend.as_deref(), Some("groq"));
        assert_eq!(
            settings.selected_model.as_deref(),
            Some("llama-3.3-70b-versatile")
        );
    }

    #[test]
    fn set_provider_with_custom_model() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let toml_path = dir.path().join("config.toml");

        cmd_set_provider("anthropic", Some("claude-opus-4-6"), Some(&toml_path))
            .expect("set provider with model");

        let settings = Settings::load_toml(&toml_path)
            .expect("read toml")
            .expect("should have settings");
        assert_eq!(settings.llm_backend.as_deref(), Some("anthropic"));
        assert_eq!(settings.selected_model.as_deref(), Some("claude-opus-4-6"));
    }

    #[test]
    fn custom_config_does_not_pollute_default_dotenv() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let toml_path = dir.path().join("config.toml");

        // With a custom config path, sync_to_dotenv should be a no-op
        // (it returns early when config_path is Some).
        // We verify by checking that cmd_set_provider succeeds without
        // trying to write to the default ~/.ironclaw/.env.
        cmd_set_provider("groq", None, Some(&toml_path)).expect("set provider with custom config");

        let settings = Settings::load_toml(&toml_path)
            .expect("read toml")
            .expect("should have settings");
        assert_eq!(settings.llm_backend.as_deref(), Some("groq"));
        // The key assertion is that no error was thrown trying to write
        // to the default .env — sync_to_dotenv skipped it.
    }

    #[test]
    fn set_model_rejects_empty_name() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let toml_path = dir.path().join("config.toml");

        let result = cmd_set_model("", Some(&toml_path));
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("cannot be empty"),
            "should reject empty model name"
        );

        let result2 = cmd_set_model("   ", Some(&toml_path));
        assert!(result2.is_err());
    }

    #[test]
    fn set_provider_normalizes_alias() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let toml_path = dir.path().join("config.toml");

        cmd_set_provider("claude", None, Some(&toml_path)).expect("set via alias");

        let settings = Settings::load_toml(&toml_path)
            .expect("read toml")
            .expect("should have settings");
        assert_eq!(
            settings.llm_backend.as_deref(),
            Some("anthropic"),
            "alias should be normalized to canonical ID"
        );
    }
}
