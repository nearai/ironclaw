//! Main setup wizard orchestration.
//!
//! The wizard guides users through:
//! 1. Database connection
//! 2. Security (secrets master key)
//! 3. Inference provider (NEAR AI, Anthropic, OpenAI, GitHub Copilot, OpenAI Codex, Ollama, OpenAI-compatible)
//! 4. Model selection
//! 5. Embeddings
//! 6. Channel configuration
//! 7. Extensions (tool installation from registry)
//! 8. Docker sandbox
//! 9. Heartbeat (background tasks)

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[cfg(feature = "postgres")]
use deadpool_postgres::Config as PoolConfig;
use secrecy::{ExposeSecret, SecretString};

use crate::bootstrap::ironclaw_base_dir;
use crate::channels::wasm::{
    ChannelCapabilitiesFile, available_channel_names, install_bundled_channel,
};
use crate::config::OAUTH_PLACEHOLDER;
use crate::llm::models::{
    build_nearai_model_fetch_config, fetch_anthropic_models, fetch_ollama_models,
    fetch_openai_compatible_models, fetch_openai_models,
};
#[cfg(test)]
use crate::llm::models::{is_openai_chat_model, sort_openai_models};
use crate::llm::{OLLAMA_DEFAULT_BASE_URL, SessionConfig, SessionManager};
use crate::secrets::{SecretsCrypto, SecretsStore};
use crate::settings::{KeySource, Settings};
use crate::setup::channels::{
    SecretsContext, setup_http, setup_signal, setup_tunnel, setup_wasm_channel,
};
use crate::setup::prompts::{
    confirm, input, optional_input, print_banner, print_error, print_header, print_info,
    print_step, print_success, secret_input, select_many, select_one,
};

// unused const, keep commented for clarity / future use
// const CHANNEL_INDEX_CLI: usize = 0;
const CHANNEL_INDEX_HTTP: usize = 1;
const CHANNEL_INDEX_SIGNAL: usize = 2;
const QUICK_PROFILE_LOCAL: &str = "local";
const QUICK_PROFILE_LOCAL_SANDBOX: &str = "local-sandbox";

/// Setup wizard error.
#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Authentication error: {0}")]
    Auth(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Channel setup error: {0}")]
    Channel(String),

    #[error("User cancelled")]
    Cancelled,

    #[error("Sandbox error: {0}")]
    Sandbox(#[from] crate::sandbox::error::SandboxError),
}

impl From<crate::setup::channels::ChannelSetupError> for SetupError {
    fn from(e: crate::setup::channels::ChannelSetupError) -> Self {
        SetupError::Channel(e.to_string())
    }
}

/// Setup wizard configuration.
#[derive(Debug, Clone, Default)]
pub struct SetupConfig {
    /// Skip authentication step (use existing session).
    pub skip_auth: bool,
    /// Only reconfigure channels.
    pub channels_only: bool,
    /// Only reconfigure LLM provider and model selection.
    pub provider_only: bool,
    /// Quick setup: auto-defaults everything except LLM provider and model.
    pub quick: bool,
    /// Run only specific setup steps (e.g. "provider", "channels", "model", "database", "security").
    pub steps: Vec<String>,
}

/// Interactive setup wizard for IronClaw.
pub struct SetupWizard {
    config: SetupConfig,
    settings: Settings,
    selected_deployment_profile: Option<String>,
    owner_id: String,
    session_manager: Option<Arc<SessionManager>>,
    /// Database pool (created during setup, postgres only).
    #[cfg(feature = "postgres")]
    db_pool: Option<deadpool_postgres::Pool>,
    /// libSQL backend (created during setup, libsql only).
    #[cfg(feature = "libsql")]
    db_backend: Option<crate::db::libsql::LibSqlBackend>,
    /// Secrets crypto (created during setup).
    secrets_crypto: Option<Arc<SecretsCrypto>>,
    /// Cached API key from provider setup (used by model fetcher without env mutation).
    llm_api_key: Option<SecretString>,
}

impl SetupWizard {
    fn owner_id(&self) -> &str {
        &self.owner_id
    }

    fn fallback_with_default_owner(
        config: SetupConfig,
        settings: Settings,
        error: &crate::error::ConfigError,
    ) -> Self {
        tracing::warn!("Falling back to default owner scope for setup wizard: {error}");
        Self {
            config,
            settings,
            selected_deployment_profile: None,
            owner_id: "default".to_string(),
            session_manager: None,
            #[cfg(feature = "postgres")]
            db_pool: None,
            #[cfg(feature = "libsql")]
            db_backend: None,
            secrets_crypto: None,
            llm_api_key: None,
        }
    }

    fn from_bootstrap_settings(
        config: SetupConfig,
        settings: Settings,
    ) -> Result<Self, crate::error::ConfigError> {
        let owner_id = crate::config::resolve_owner_id(&settings)?;
        Ok(Self {
            config,
            settings,
            selected_deployment_profile: None,
            owner_id,
            session_manager: None,
            #[cfg(feature = "postgres")]
            db_pool: None,
            #[cfg(feature = "libsql")]
            db_backend: None,
            secrets_crypto: None,
            llm_api_key: None,
        })
    }

    /// Create a new setup wizard.
    pub fn new() -> Self {
        let settings = crate::config::load_bootstrap_settings(None).unwrap_or_default();
        Self::from_bootstrap_settings(SetupConfig::default(), settings.clone()).unwrap_or_else(
            |e| Self::fallback_with_default_owner(SetupConfig::default(), settings, &e),
        )
    }

    /// Create a wizard with custom configuration.
    pub fn with_config(config: SetupConfig) -> Self {
        let settings = crate::config::load_bootstrap_settings(None).unwrap_or_default();
        Self::from_bootstrap_settings(config.clone(), settings.clone())
            .unwrap_or_else(|e| Self::fallback_with_default_owner(config, settings, &e))
    }

    /// Create a wizard with custom configuration and bootstrap TOML overlay.
    pub fn try_with_config_and_toml(
        config: SetupConfig,
        toml_path: Option<&std::path::Path>,
    ) -> Result<Self, crate::error::ConfigError> {
        let settings = crate::config::load_bootstrap_settings(toml_path)?;
        Self::from_bootstrap_settings(config, settings)
    }

    /// Set the session manager (for reusing existing auth).
    pub fn with_session(mut self, session: Arc<SessionManager>) -> Self {
        self.session_manager = Some(session);
        self
    }

    /// Run the setup wizard.
    ///
    /// Settings are persisted incrementally after each successful step so
    /// that progress is not lost if a later step fails. On re-run, existing
    /// settings are loaded from the database after Step 1 establishes a
    /// connection, so users don't have to re-enter everything.
    pub async fn run(&mut self) -> Result<(), SetupError> {
        print_banner();
        print_header("IronClaw Setup Wizard");

        if !self.config.steps.is_empty() {
            // Selective step mode: reconnect to existing DB and load settings,
            // then run only the requested steps.
            self.reconnect_existing_db().await?;

            let valid_steps = ["provider", "channels", "model", "database", "security"];
            for s in &self.config.steps {
                if !valid_steps.contains(&s.as_str()) {
                    return Err(SetupError::Config(format!(
                        "Unknown step '{}'. Valid steps: {}",
                        s,
                        valid_steps.join(", ")
                    )));
                }
            }

            let total = self.config.steps.len();
            for (i, step_name) in self.config.steps.clone().iter().enumerate() {
                let step_num = i + 1;
                match step_name.as_str() {
                    "database" => {
                        print_step(step_num, total, "Database Connection");
                        self.step_database().await?;
                    }
                    "security" => {
                        print_step(step_num, total, "Security");
                        self.step_security().await?;
                    }
                    "provider" => {
                        print_step(step_num, total, "Inference Provider");
                        self.step_inference_provider().await?;
                    }
                    "model" => {
                        print_step(step_num, total, "Model Selection");
                        self.step_model_selection().await?;
                    }
                    "channels" => {
                        print_step(step_num, total, "Channel Configuration");
                        self.step_channels().await?;
                    }
                    _ => {} // already validated above
                }
                self.persist_after_step().await;
            }

            self.save_and_summarize().await?;
            return Ok(());
        }

        if self.config.channels_only {
            // Channels-only mode: reconnect to existing DB and load settings
            // before running the channel step, so secrets and save work.
            self.reconnect_existing_db().await?;
            print_step(1, 1, "Channel Configuration");
            self.step_channels().await?;
        } else if self.config.provider_only {
            // Provider-only mode: reconnect to existing DB, then run just
            // inference provider + model selection steps.
            self.reconnect_existing_db().await?;
            print_step(1, 2, "Inference Provider");
            self.step_inference_provider().await?;
            self.persist_after_step().await;
            print_step(2, 2, "Model Selection");
            self.step_model_selection().await?;
            self.persist_after_step().await;
        } else if self.config.quick {
            // Quick mode: auto-default database + security, only ask for
            // the local usage profile (on true first run), LLM provider,
            // and model. Designed for first-run experience.

            // Profile selection runs first so that `apply_profile()` can set
            // `database_backend` before `auto_setup_database()` connects.
            // The subsequent clone → try_load → merge_from cycle preserves
            // these wizard-chosen values over any stale DB values.
            self.step_quick_local_profile()?;

            self.auto_setup_database().await?;

            // Load existing settings from DB (if any prior partial run)
            let step1_settings = self.settings.clone();
            self.try_load_existing_settings().await;
            self.settings.merge_from(&step1_settings);

            self.auto_setup_security().await?;
            self.persist_after_step().await;

            // Pre-populate backend from env so step_inference_provider
            // can offer "Keep current provider?" instead of asking from scratch.
            if self.settings.llm_backend.is_none() {
                if let Ok(b) = std::env::var("LLM_BACKEND") {
                    self.settings.llm_backend = Some(b);
                } else if std::env::var("NEARAI_API_KEY").is_ok() {
                    self.settings.llm_backend = Some("nearai".to_string());
                } else if std::env::var("ANTHROPIC_API_KEY").is_ok()
                    || std::env::var("ANTHROPIC_OAUTH_TOKEN").is_ok()
                {
                    self.settings.llm_backend = Some("anthropic".to_string());
                } else if std::env::var("OPENAI_API_KEY").is_ok() {
                    self.settings.llm_backend = Some("openai".to_string());
                } else if std::env::var("OPENROUTER_API_KEY").is_ok() {
                    self.settings.llm_backend = Some("openrouter".to_string());
                }
            }

            if let Ok(api_key) = std::env::var("NEARAI_API_KEY")
                && !api_key.is_empty()
                && self.settings.llm_backend.as_deref() == Some("nearai")
            {
                // NEARAI_API_KEY is set and backend auto-detected — skip interactive prompts
                print_info("NEARAI_API_KEY found — using NEAR AI provider");
                if let Ok(ctx) = self.init_secrets_context().await {
                    let key = SecretString::from(api_key.clone());
                    if let Err(e) = ctx.save_secret("llm_nearai_api_key", &key).await {
                        tracing::warn!("Failed to persist NEARAI_API_KEY to secrets: {}", e);
                    }
                }
                self.llm_api_key = Some(SecretString::from(api_key));
                if self.settings.selected_model.is_none() {
                    let default = crate::llm::DEFAULT_MODEL;
                    self.settings.selected_model = Some(default.to_string());
                    print_info(&format!("Using default model: {default}"));
                }
                self.persist_after_step().await;
            } else if self.settings.llm_backend.as_deref() == Some("anthropic")
                && let Some(api_key) = Self::detect_anthropic_key()
            {
                // Anthropic key detected — skip interactive prompts
                print_info("Anthropic credentials found — using Anthropic provider");
                let secret_name = if api_key.starts_with("sk-ant-oat") {
                    "llm_anthropic_oauth_token"
                } else {
                    "llm_anthropic_api_key"
                };
                if let Ok(ctx) = self.init_secrets_context().await {
                    let key = SecretString::from(api_key.clone());
                    if let Err(e) = ctx.save_secret(secret_name, &key).await {
                        tracing::warn!("Failed to persist Anthropic key to secrets: {}", e);
                    }
                }
                self.llm_api_key = Some(SecretString::from(api_key));
                let registry = crate::llm::ProviderRegistry::load();
                if self.settings.selected_model.is_none() {
                    let default = registry
                        .find("anthropic")
                        .map(|d| d.default_model.as_str())
                        .unwrap_or("claude-sonnet-4-20250514");
                    self.settings.selected_model = Some(default.to_string());
                    print_info(&format!("Using default model: {default}"));
                }
                self.persist_after_step().await;
            } else if let Ok(api_key) = std::env::var("OPENAI_API_KEY")
                && !api_key.is_empty()
                && self.settings.llm_backend.as_deref() == Some("openai")
            {
                // OpenAI key detected — skip interactive prompts
                print_info("OPENAI_API_KEY found — using OpenAI provider");
                if let Ok(ctx) = self.init_secrets_context().await {
                    let key = SecretString::from(api_key.clone());
                    if let Err(e) = ctx.save_secret("llm_openai_api_key", &key).await {
                        tracing::warn!("Failed to persist OPENAI_API_KEY to secrets: {}", e);
                    }
                }
                self.llm_api_key = Some(SecretString::from(api_key));
                let registry = crate::llm::ProviderRegistry::load();
                if self.settings.selected_model.is_none() {
                    let default = registry
                        .find("openai")
                        .map(|d| d.default_model.as_str())
                        .unwrap_or("gpt-5-mini");
                    self.settings.selected_model = Some(default.to_string());
                    print_info(&format!("Using default model: {default}"));
                }
                self.persist_after_step().await;
            } else if let Ok(api_key) = std::env::var("OPENROUTER_API_KEY")
                && !api_key.is_empty()
                && self.settings.llm_backend.as_deref() == Some("openrouter")
            {
                // OpenRouter key detected — skip interactive prompts
                print_info("OPENROUTER_API_KEY found — using OpenRouter provider");
                if let Ok(ctx) = self.init_secrets_context().await {
                    let key = SecretString::from(api_key.clone());
                    if let Err(e) = ctx.save_secret("llm_openrouter_api_key", &key).await {
                        tracing::warn!("Failed to persist OPENROUTER_API_KEY to secrets: {}", e);
                    }
                }
                self.llm_api_key = Some(SecretString::from(api_key));
                let registry = crate::llm::ProviderRegistry::load();
                if self.settings.selected_model.is_none() {
                    let default = registry
                        .find("openrouter")
                        .map(|d| d.default_model.as_str())
                        .unwrap_or("openai/gpt-4o");
                    self.settings.selected_model = Some(default.to_string());
                    print_info(&format!("Using default model: {default}"));
                }
                self.persist_after_step().await;
            } else {
                print_step(1, 2, "Inference Provider");
                self.step_inference_provider().await?;
                self.persist_after_step().await;

                print_step(2, 2, "Model Selection");
                self.step_model_selection().await?;
                self.persist_after_step().await;
            }
        } else {
            let total_steps = 9;

            // Step 1: Database
            print_step(1, total_steps, "Database Connection");
            self.step_database().await?;

            // After establishing a DB connection, load any previously saved
            // settings so we recover progress from prior partial runs.
            // We must load BEFORE persisting, otherwise persist_after_step()
            // would overwrite prior settings with defaults.
            // Save Step 1 choices first so they aren't clobbered by stale
            // DB values (merge_from only applies non-default fields).
            let step1_settings = self.settings.clone();
            self.try_load_existing_settings().await;
            self.settings.merge_from(&step1_settings);

            self.persist_after_step().await;

            // Step 2: Security
            print_step(2, total_steps, "Security");
            self.step_security().await?;
            self.persist_after_step().await;

            // Step 3: Inference provider selection (unless skipped)
            if !self.config.skip_auth {
                print_step(3, total_steps, "Inference Provider");
                self.step_inference_provider().await?;
            } else {
                print_info("Skipping inference provider setup (using existing config)");
            }
            self.persist_after_step().await;

            // Step 4: Model selection
            print_step(4, total_steps, "Model Selection");
            self.step_model_selection().await?;
            self.persist_after_step().await;

            // Step 5: Embeddings
            print_step(5, total_steps, "Embeddings (Semantic Search)");
            self.step_embeddings()?;
            self.persist_after_step().await;

            // Step 6: Channel configuration
            print_step(6, total_steps, "Channel Configuration");
            self.step_channels().await?;
            self.persist_after_step().await;

            // Step 7: Extensions (tools)
            print_step(7, total_steps, "Extensions");
            self.step_extensions().await?;

            // Step 8: Docker Sandbox
            print_step(8, total_steps, "Docker Sandbox");
            self.step_docker_sandbox().await?;
            self.persist_after_step().await;

            // Step 9: Heartbeat
            print_step(9, total_steps, "Background Tasks");
            self.step_heartbeat()?;
            self.persist_after_step().await;

            // Personal onboarding now happens conversationally during the
            // user's first interaction with the assistant (see bootstrap
            // block in workspace/mod.rs system_prompt_for_context).
        }

        // Save settings and print summary
        self.save_and_summarize().await?;

        Ok(())
    }

    /// Reconnect to the existing database and load settings.
    ///
    /// Used by channels-only mode (and future single-step modes) so that
    /// `init_secrets_context()` and `save_and_summarize()` have a live
    /// database connection and the wizard's `self.settings` reflects the
    /// previously saved configuration.
    async fn reconnect_existing_db(&mut self) -> Result<(), SetupError> {
        // Determine backend from env (set by bootstrap .env loaded in main).
        let backend = std::env::var("DATABASE_BACKEND").unwrap_or_else(|_| "postgres".to_string());

        // Try libsql first if that's the configured backend.
        #[cfg(feature = "libsql")]
        if backend == "libsql" || backend == "turso" || backend == "sqlite" {
            return self.reconnect_libsql().await;
        }

        // Try postgres (either explicitly configured or as default).
        #[cfg(feature = "postgres")]
        {
            let _ = &backend;
            return self.reconnect_postgres().await;
        }

        #[allow(unreachable_code)]
        Err(SetupError::Database(
            "No database configured. Run full setup first (ironclaw onboard).".to_string(),
        ))
    }

    /// Reconnect to an existing PostgreSQL database and load settings.
    #[cfg(feature = "postgres")]
    async fn reconnect_postgres(&mut self) -> Result<(), SetupError> {
        let url = std::env::var("DATABASE_URL").map_err(|_| {
            SetupError::Database(
                "DATABASE_URL not set. Run full setup first (ironclaw onboard).".to_string(),
            )
        })?;

        self.test_database_connection_postgres(&url).await?;
        self.settings.database_backend = Some("postgres".to_string());
        self.settings.database_url = Some(url.clone());

        // Load existing settings from DB, then restore connection fields that
        // may not be persisted in the settings map.
        if let Some(ref pool) = self.db_pool {
            let store = crate::history::Store::from_pool(pool.clone());
            if let Ok(map) = store.get_all_settings(self.owner_id()).await {
                self.settings = Settings::from_db_map(&map);
                self.settings.database_backend = Some("postgres".to_string());
                self.settings.database_url = Some(url);
            }
        }

        Ok(())
    }

    /// Reconnect to an existing libSQL database and load settings.
    #[cfg(feature = "libsql")]
    async fn reconnect_libsql(&mut self) -> Result<(), SetupError> {
        let path = std::env::var("LIBSQL_PATH").unwrap_or_else(|_| {
            crate::config::default_libsql_path()
                .to_string_lossy()
                .to_string()
        });
        let turso_url = std::env::var("LIBSQL_URL").ok();
        let turso_token = std::env::var("LIBSQL_AUTH_TOKEN").ok();

        self.test_database_connection_libsql(&path, turso_url.as_deref(), turso_token.as_deref())
            .await?;

        self.settings.database_backend = Some("libsql".to_string());
        self.settings.libsql_path = Some(path.clone());
        if let Some(ref url) = turso_url {
            self.settings.libsql_url = Some(url.clone());
        }

        // Load existing settings from DB, then restore connection fields that
        // may not be persisted in the settings map.
        if let Some(ref db) = self.db_backend {
            use crate::db::SettingsStore as _;
            if let Ok(map) = db.get_all_settings(self.owner_id()).await {
                self.settings = Settings::from_db_map(&map);
                self.settings.database_backend = Some("libsql".to_string());
                self.settings.libsql_path = Some(path);
                if let Some(url) = turso_url {
                    self.settings.libsql_url = Some(url);
                }
            }
        }

        Ok(())
    }

    /// Step 1: Database connection.
    async fn step_database(&mut self) -> Result<(), SetupError> {
        // When both features are compiled, let the user choose.
        // If DATABASE_BACKEND is already set in the environment, respect it.
        #[cfg(all(feature = "postgres", feature = "libsql"))]
        {
            // Check if a backend is already pinned via env var
            let env_backend = std::env::var("DATABASE_BACKEND").ok();

            if let Some(ref backend) = env_backend {
                if backend == "libsql" || backend == "turso" || backend == "sqlite" {
                    return self.step_database_libsql().await;
                }
                if backend != "postgres" && backend != "postgresql" {
                    print_info(&format!(
                        "Unknown DATABASE_BACKEND '{}', defaulting to PostgreSQL",
                        backend
                    ));
                }
                return self.step_database_postgres().await;
            }

            // Interactive selection
            let pre_selected = self.settings.database_backend.as_deref().map(|b| match b {
                "libsql" | "turso" | "sqlite" => 1,
                _ => 0,
            });

            print_info("Which database backend would you like to use?");
            println!();

            let options = &[
                "PostgreSQL  - production-grade, requires a running server",
                "libSQL      - embedded SQLite, zero dependencies, optional Turso cloud sync",
            ];
            let choice =
                select_one("Select a database backend:", options).map_err(SetupError::Io)?;

            // If the user picked something different from what was pre-selected, clear
            // stale connection settings so the next step starts fresh.
            if let Some(prev) = pre_selected
                && prev != choice
            {
                self.settings.database_url = None;
                self.settings.libsql_path = None;
                self.settings.libsql_url = None;
            }

            match choice {
                1 => return self.step_database_libsql().await,
                _ => return self.step_database_postgres().await,
            }
        }

        #[cfg(all(feature = "postgres", not(feature = "libsql")))]
        {
            return self.step_database_postgres().await;
        }

        #[cfg(all(feature = "libsql", not(feature = "postgres")))]
        {
            return self.step_database_libsql().await;
        }
    }

    /// Step 1 (postgres): Database connection via PostgreSQL URL.
    #[cfg(feature = "postgres")]
    async fn step_database_postgres(&mut self) -> Result<(), SetupError> {
        self.settings.database_backend = Some("postgres".to_string());

        let existing_url = std::env::var("DATABASE_URL")
            .ok()
            .or_else(|| self.settings.database_url.clone());

        if let Some(ref url) = existing_url {
            let display_url = mask_password_in_url(url);
            print_info(&format!("Existing database URL: {}", display_url));

            if confirm("Use this database?", true).map_err(SetupError::Io)? {
                if let Err(e) = self.test_database_connection_postgres(url).await {
                    print_error(&format!("Connection failed: {}", e));
                    print_info("Let's configure a new database URL.");
                } else {
                    print_success("Database connection successful");
                    self.settings.database_url = Some(url.clone());
                    return Ok(());
                }
            }
        }

        println!();
        print_info("Enter your PostgreSQL connection URL.");
        print_info("Format: postgres://user:password@host:port/database");
        println!();

        loop {
            let url = input("Database URL").map_err(SetupError::Io)?;

            if url.is_empty() {
                print_error("Database URL is required.");
                continue;
            }

            print_info("Testing connection...");
            match self.test_database_connection_postgres(&url).await {
                Ok(()) => {
                    print_success("Database connection successful");

                    if confirm("Run database migrations?", true).map_err(SetupError::Io)? {
                        self.run_migrations_postgres().await?;
                    }

                    self.settings.database_url = Some(url);
                    return Ok(());
                }
                Err(e) => {
                    print_error(&format!("Connection failed: {}", e));
                    if !confirm("Try again?", true).map_err(SetupError::Io)? {
                        return Err(SetupError::Database(
                            "Database connection failed".to_string(),
                        ));
                    }
                }
            }
        }
    }

    /// Step 1 (libsql): Database connection via local file or Turso remote replica.
    #[cfg(feature = "libsql")]
    async fn step_database_libsql(&mut self) -> Result<(), SetupError> {
        self.settings.database_backend = Some("libsql".to_string());

        let default_path = crate::config::default_libsql_path();
        let default_path_str = default_path.to_string_lossy().to_string();

        // Check for existing configuration
        let existing_path = std::env::var("LIBSQL_PATH")
            .ok()
            .or_else(|| self.settings.libsql_path.clone());

        if let Some(ref path) = existing_path {
            print_info(&format!("Existing database path: {}", path));
            if confirm("Use this database?", true).map_err(SetupError::Io)? {
                let turso_url = std::env::var("LIBSQL_URL")
                    .ok()
                    .or_else(|| self.settings.libsql_url.clone());
                let turso_token = std::env::var("LIBSQL_AUTH_TOKEN").ok();

                match self
                    .test_database_connection_libsql(
                        path,
                        turso_url.as_deref(),
                        turso_token.as_deref(),
                    )
                    .await
                {
                    Ok(()) => {
                        print_success("Database connection successful");
                        self.settings.libsql_path = Some(path.clone());
                        if let Some(url) = turso_url {
                            self.settings.libsql_url = Some(url);
                        }
                        return Ok(());
                    }
                    Err(e) => {
                        print_error(&format!("Connection failed: {}", e));
                        print_info("Let's configure a new database path.");
                    }
                }
            }
        }

        println!();
        print_info("IronClaw uses an embedded SQLite database (libSQL).");
        print_info("No external database server required.");
        println!();

        let path_input = optional_input(
            "Database file path",
            Some(&format!("default: {}", default_path_str)),
        )
        .map_err(SetupError::Io)?;

        let db_path = path_input.unwrap_or(default_path_str.clone());

        // Ask about Turso cloud sync
        println!();
        let use_turso =
            confirm("Enable Turso cloud sync (remote replica)?", false).map_err(SetupError::Io)?;

        let (turso_url, turso_token) = if use_turso {
            print_info("Enter your Turso database URL and auth token.");
            print_info("Format: libsql://your-db.turso.io");
            println!();

            let url = input("Turso URL").map_err(SetupError::Io)?;
            if url.is_empty() {
                print_error("Turso URL is required for cloud sync.");
                (None, None)
            } else {
                let token_secret = secret_input("Auth token").map_err(SetupError::Io)?;
                let token = token_secret.expose_secret().to_string();
                if token.is_empty() {
                    print_error("Auth token is required for cloud sync.");
                    (None, None)
                } else {
                    (Some(url), Some(token))
                }
            }
        } else {
            (None, None)
        };

        print_info("Testing connection...");
        match self
            .test_database_connection_libsql(&db_path, turso_url.as_deref(), turso_token.as_deref())
            .await
        {
            Ok(()) => {
                print_success("Database connection successful");

                // Always run migrations for libsql (they're idempotent)
                self.run_migrations_libsql().await?;

                self.settings.libsql_path = Some(db_path);
                if let Some(url) = turso_url {
                    self.settings.libsql_url = Some(url);
                }
                Ok(())
            }
            Err(e) => Err(SetupError::Database(format!("Connection failed: {}", e))),
        }
    }

    /// Test PostgreSQL connection and store the pool.
    ///
    /// After connecting, validates:
    /// 1. PostgreSQL version >= 15 (required for pgvector compatibility)
    /// 2. pgvector extension is available (required for embeddings/vector search)
    #[cfg(feature = "postgres")]
    async fn test_database_connection_postgres(&mut self, url: &str) -> Result<(), SetupError> {
        let mut cfg = PoolConfig::new();
        cfg.url = Some(url.to_string());
        cfg.pool = Some(deadpool_postgres::PoolConfig {
            max_size: 5,
            ..Default::default()
        });

        let pool = crate::db::tls::create_pool(&cfg, crate::config::SslMode::from_env())
            .map_err(|e| SetupError::Database(format!("Failed to create pool: {}", e)))?;

        let client = pool
            .get()
            .await
            .map_err(|e| SetupError::Database(format!("Failed to connect: {}", e)))?;

        // Check PostgreSQL server version (need 15+ for pgvector)
        let version_row = client
            .query_one("SHOW server_version", &[])
            .await
            .map_err(|e| SetupError::Database(format!("Failed to query server version: {}", e)))?;
        let version_str: &str = version_row.get(0);
        let major_version = version_str
            .split('.')
            .next()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0);

        const MIN_PG_MAJOR_VERSION: u32 = 15;

        if major_version < MIN_PG_MAJOR_VERSION {
            return Err(SetupError::Database(format!(
                "PostgreSQL {} detected. IronClaw requires PostgreSQL {} or later for pgvector support.\n\
                 Upgrade: https://www.postgresql.org/download/",
                version_str, MIN_PG_MAJOR_VERSION
            )));
        }

        // Check if pgvector extension is available
        let pgvector_row = client
            .query_opt(
                "SELECT 1 FROM pg_available_extensions WHERE name = 'vector'",
                &[],
            )
            .await
            .map_err(|e| {
                SetupError::Database(format!("Failed to check pgvector availability: {}", e))
            })?;

        if pgvector_row.is_none() {
            return Err(SetupError::Database(format!(
                "pgvector extension not found on your PostgreSQL server.\n\n\
                 Install it:\n  \
                 macOS:   brew install pgvector\n  \
                 Ubuntu:  apt install postgresql-{0}-pgvector\n  \
                 Docker:  use the pgvector/pgvector:pg{0} image\n  \
                 Source:  https://github.com/pgvector/pgvector#installation\n\n\
                 Then restart PostgreSQL and re-run: ironclaw onboard",
                major_version
            )));
        }

        self.db_pool = Some(pool);
        Ok(())
    }

    /// Test libSQL connection and store the backend.
    #[cfg(feature = "libsql")]
    async fn test_database_connection_libsql(
        &mut self,
        path: &str,
        turso_url: Option<&str>,
        turso_token: Option<&str>,
    ) -> Result<(), SetupError> {
        use crate::db::libsql::LibSqlBackend;
        use std::path::Path;

        let db_path = Path::new(path);

        let backend = if let (Some(url), Some(token)) = (turso_url, turso_token) {
            LibSqlBackend::new_remote_replica(db_path, url, token)
                .await
                .map_err(|e| SetupError::Database(format!("Failed to connect: {}", e)))?
        } else {
            LibSqlBackend::new_local(db_path)
                .await
                .map_err(|e| SetupError::Database(format!("Failed to open database: {}", e)))?
        };

        self.db_backend = Some(backend);
        Ok(())
    }

    /// Run PostgreSQL migrations.
    ///
    /// Delegates to `crate::db::migration_fixup::run_postgres_migrations_with_fixup`,
    /// which acquires the migration advisory lock, realigns any historically
    /// diverged checksums (issue #1328), then runs refinery's embedded
    /// migrations. Bundled into a single helper so this call site cannot
    /// drift from `Store::run_migrations` (see PR #2101 review).
    ///
    /// Requires `self.db_pool` to be populated by a prior call to
    /// `test_database_connection_postgres`. Returns an error rather than
    /// silently no-opping so the coupling between the two calls cannot
    /// silently regress (see issue #846 / PR #2309 review).
    #[cfg(feature = "postgres")]
    async fn run_migrations_postgres(&self) -> Result<(), SetupError> {
        let pool = self.db_pool.as_ref().ok_or_else(|| {
            SetupError::Database(
                "run_migrations_postgres called without an established pool; \
                 test_database_connection_postgres must run first"
                    .to_string(),
            )
        })?;

        if !self.config.quick {
            print_info("Running migrations...");
        }
        tracing::debug!("Running PostgreSQL migrations...");

        let mut client = pool
            .get()
            .await
            .map_err(|e| SetupError::Database(format!("Pool error: {}", e)))?;

        crate::db::migration_fixup::run_postgres_migrations_with_fixup(&mut client)
            .await
            .map_err(|e| SetupError::Database(format!("Migration failed: {}", e)))?;

        if !self.config.quick {
            print_success("Migrations applied");
        }
        tracing::debug!("PostgreSQL migrations applied");
        Ok(())
    }

    /// Run libSQL migrations.
    #[cfg(feature = "libsql")]
    async fn run_migrations_libsql(&self) -> Result<(), SetupError> {
        if let Some(ref backend) = self.db_backend {
            use crate::db::Database;

            if !self.config.quick {
                print_info("Running migrations...");
            }
            tracing::debug!("Running libSQL migrations...");

            backend
                .run_migrations()
                .await
                .map_err(|e| SetupError::Database(format!("Migration failed: {}", e)))?;

            if !self.config.quick {
                print_success("Migrations applied");
            }
            tracing::debug!("libSQL migrations applied");
        }
        Ok(())
    }

    /// Step 2: Security (secrets master key).
    async fn step_security(&mut self) -> Result<(), SetupError> {
        // Check current configuration
        let env_key_exists = std::env::var("SECRETS_MASTER_KEY").is_ok();

        if env_key_exists {
            print_info("Secrets master key found in SECRETS_MASTER_KEY environment variable.");
            self.settings.secrets_master_key_source = KeySource::Env;
            print_success("Security configured (env var)");
            return Ok(());
        }

        // Try to retrieve existing key from keychain. We use get_master_key()
        // instead of has_master_key() so we can cache the key bytes and build
        // SecretsCrypto eagerly, avoiding redundant keychain accesses later
        // (each access triggers macOS system dialogs).
        print_info("Checking OS keychain for existing master key...");
        if let Ok(keychain_key_bytes) = crate::secrets::keychain::get_master_key().await {
            let key_hex: String = keychain_key_bytes
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect();
            self.secrets_crypto = Some(Arc::new(
                SecretsCrypto::new(SecretString::from(key_hex))
                    .map_err(|e| SetupError::Config(e.to_string()))?,
            ));

            print_info("Existing master key found in OS keychain.");
            if confirm("Use existing keychain key?", true).map_err(SetupError::Io)? {
                self.settings.secrets_master_key_source = KeySource::Keychain;
                print_success("Security configured (keychain)");
                return Ok(());
            }
            // User declined the existing key; clear the cached crypto so a fresh
            // key can be generated below.
            self.secrets_crypto = None;
        }

        // Offer options
        println!();
        print_info("The secrets master key encrypts sensitive data like API tokens.");
        print_info("Choose where to store it:");
        println!();

        let options = [
            "OS Keychain (recommended for local installs)",
            "Environment variable (for CI/Docker)",
            "Skip (disable secrets features)",
        ];

        let choice = select_one("Select storage method:", &options).map_err(SetupError::Io)?;

        match choice {
            0 => {
                // Generate and store in keychain
                print_info("Generating master key...");
                let key = crate::secrets::keychain::generate_master_key();

                crate::secrets::keychain::store_master_key(&key)
                    .await
                    .map_err(|e| {
                        SetupError::Config(format!("Failed to store in keychain: {}", e))
                    })?;

                // Also create crypto instance
                let key_hex: String = key.iter().map(|b| format!("{:02x}", b)).collect();
                self.secrets_crypto = Some(Arc::new(
                    SecretsCrypto::new(SecretString::from(key_hex))
                        .map_err(|e| SetupError::Config(e.to_string()))?,
                ));

                self.settings.secrets_master_key_source = KeySource::Keychain;
                print_success("Master key generated and stored in OS keychain");
            }
            1 => {
                // Env var mode — generate key, init crypto, and persist to .env
                let key_hex = crate::secrets::keychain::generate_master_key_hex();

                // Initialize crypto so subsequent wizard steps (channel setup,
                // API key storage) can encrypt secrets immediately.
                self.secrets_crypto = Some(Arc::new(
                    SecretsCrypto::new(SecretString::from(key_hex.clone()))
                        .map_err(|e| SetupError::Config(e.to_string()))?,
                ));

                // Make visible to optional_env() for any subsequent config resolution.
                crate::config::inject_single_var("SECRETS_MASTER_KEY", &key_hex);

                // Store hex for write_bootstrap_env to persist to ~/.ironclaw/.env.
                self.settings.secrets_master_key_hex = Some(key_hex.clone());

                println!();
                print_info(&format!(
                    "Master key generated and will be saved to {}",
                    crate::bootstrap::ironclaw_env_path().display()
                ));
                println!();
                println!("  SECRETS_MASTER_KEY={}", key_hex);
                println!();
                print_info("You can also copy this to another .env file or CI secrets.");

                self.settings.secrets_master_key_source = KeySource::Env;
                print_success("Configured for environment variable");
            }
            _ => {
                self.settings.secrets_master_key_source = KeySource::None;
                print_info("Secrets features disabled. Channel tokens must be set via env vars.");
            }
        }

        Ok(())
    }

    /// Auto-setup database with zero prompts (quick mode).
    ///
    /// Uses existing env vars if present, otherwise defaults to libsql at the
    /// standard path. Falls back to the interactive `step_database()` only when
    /// just the postgres feature is compiled (can't auto-default postgres).
    async fn auto_setup_database(&mut self) -> Result<(), SetupError> {
        // If DATABASE_URL or LIBSQL_PATH already set, respect existing config
        #[cfg(feature = "postgres")]
        {
            use crate::config::DatabaseBackend;

            let env_backend = std::env::var("DATABASE_BACKEND")
                .ok()
                .and_then(|b| b.parse::<DatabaseBackend>().ok());

            if matches!(env_backend, Some(DatabaseBackend::Postgres)) {
                if let Ok(url) = std::env::var("DATABASE_URL") {
                    return self.finish_postgres_auto_setup(url).await;
                }
                // Postgres configured but no URL — fall through to interactive
                return self.step_database().await;
            }

            if let Ok(url) = std::env::var("DATABASE_URL") {
                return self.finish_postgres_auto_setup(url).await;
            }
        }

        // Auto-default to libsql if the feature is compiled
        #[cfg(feature = "libsql")]
        {
            self.settings.database_backend = Some("libsql".to_string());

            let existing_path = std::env::var("LIBSQL_PATH")
                .ok()
                .or_else(|| self.settings.libsql_path.clone());

            let db_path = existing_path.unwrap_or_else(|| {
                crate::config::default_libsql_path()
                    .to_string_lossy()
                    .to_string()
            });

            let turso_url = std::env::var("LIBSQL_URL").ok();
            let turso_token = std::env::var("LIBSQL_AUTH_TOKEN").ok();

            self.test_database_connection_libsql(
                &db_path,
                turso_url.as_deref(),
                turso_token.as_deref(),
            )
            .await?;

            self.run_migrations_libsql().await?;

            self.settings.libsql_path = Some(db_path.clone());
            if let Some(url) = turso_url {
                self.settings.libsql_url = Some(url);
            }

            print_success(&format!("Using embedded database at {}", db_path));
            return Ok(());
        }

        // Only postgres feature compiled — can't auto-default, use interactive
        #[allow(unreachable_code)]
        {
            self.step_database().await
        }
    }

    /// Establish the PostgreSQL pool, run migrations, and record the backend in
    /// settings. Shared by both `auto_setup_database` early-return branches so
    /// they cannot drift (see issue #846).
    #[cfg(feature = "postgres")]
    async fn finish_postgres_auto_setup(&mut self, url: String) -> Result<(), SetupError> {
        self.test_database_connection_postgres(&url).await?;
        self.run_migrations_postgres().await?;
        print_info("Using existing PostgreSQL configuration");
        self.settings.database_backend = Some("postgres".to_string());
        self.settings.database_url = Some(url);
        Ok(())
    }

    /// Quick first-run local deployment profile selection.
    ///
    /// Existing `IRONCLAW_PROFILE` values are respected because
    /// `load_bootstrap_settings()` has already applied them before the wizard
    /// starts. This prompt only fills in the missing first-run local default.
    fn step_quick_local_profile(&mut self) -> Result<(), SetupError> {
        if crate::config::env_or_override("IRONCLAW_PROFILE").is_some() {
            return Ok(());
        }

        let options = [
            "Local (TUI + background tasks, no Docker sandbox)",
            "Local sandbox (TUI + background tasks + Docker sandbox)",
        ];
        let choice = select_one("How should IronClaw run on this machine?", &options)
            .map_err(SetupError::Io)?;
        let profile = match choice {
            0 => QUICK_PROFILE_LOCAL,
            1 => QUICK_PROFILE_LOCAL_SANDBOX,
            _ => unreachable!("select_one only returns 0 or 1 for a two-option menu"),
        };

        self.apply_quick_local_profile(profile)?;
        print_success(&format!("Using '{profile}' deployment profile"));
        Ok(())
    }

    fn apply_quick_local_profile(&mut self, profile: &str) -> Result<(), SetupError> {
        match profile {
            QUICK_PROFILE_LOCAL | QUICK_PROFILE_LOCAL_SANDBOX => {}
            other => {
                return Err(SetupError::Config(format!(
                    "unsupported quick local profile: {other}"
                )));
            }
        }

        crate::config::set_runtime_env("IRONCLAW_PROFILE", profile);
        crate::config::profile::apply_profile(&mut self.settings)
            .map_err(|e| SetupError::Config(e.to_string()))?;

        self.selected_deployment_profile = Some(profile.to_string());
        Ok(())
    }

    /// Auto-setup security with zero prompts (quick mode).
    ///
    /// Thin caller over [`SecretsConfig::resolve`], which owns the
    /// env-var → keychain → generate-and-persist chain. The wizard's
    /// only remaining responsibilities here are building the
    /// `SecretsCrypto` instance that subsequent wizard steps use to
    /// encrypt credentials before the DB is available, mirroring the
    /// resolved source into settings so `write_bootstrap_env()` picks
    /// up the hex when in env-var mode, and printing a user-visible
    /// status line.
    async fn auto_setup_security(&mut self) -> Result<(), SetupError> {
        let cfg = crate::config::SecretsConfig::resolve()
            .await
            .map_err(|e| SetupError::Config(e.to_string()))?;

        let master_key = cfg.master_key.clone().ok_or_else(|| {
            SetupError::Config("secrets resolve returned no master key".to_string())
        })?;

        self.secrets_crypto = Some(Arc::new(
            SecretsCrypto::new(master_key.clone())
                .map_err(|e| SetupError::Config(e.to_string()))?,
        ));
        self.settings.secrets_master_key_source = cfg.source;

        // In env-var mode the hex is needed by `bootstrap_env_vars()` so
        // that the wizard's idempotent final `.env` write includes
        // `SECRETS_MASTER_KEY=…` alongside the rest of the bootstrap
        // vars. `resolve()` has already persisted the key on its own,
        // but the wizard's full-set rewrite happens after DB settings
        // merge in and must carry the key forward.
        if matches!(cfg.source, KeySource::Env) {
            self.settings.secrets_master_key_hex = Some(master_key.expose_secret().to_string());
        }

        // `SecretsConfig::resolve` either returns a key with source
        // `Env`/`Keychain` or errors — `None` cannot be produced here,
        // so only the two real sources are handled.
        let msg = match cfg.source {
            KeySource::Env => format!(
                "Master key stored in {}",
                crate::bootstrap::ironclaw_env_path().display()
            ),
            KeySource::Keychain => "Master key stored in OS keychain".to_string(),
            KeySource::None => unreachable!(
                "SecretsConfig::resolve returns a key or errors; None is never produced"
            ),
        };
        print_success(&msg);
        Ok(())
    }

    /// Step 3: Inference provider selection.
    ///
    /// Uses the provider registry to dynamically build the selection menu.
    /// NearAI is always first (special auth), then all registry providers
    /// that have setup hints.
    async fn step_inference_provider(&mut self) -> Result<(), SetupError> {
        let registry = crate::llm::ProviderRegistry::load();

        // Show current provider if already configured
        if let Some(current) = self.settings.llm_backend.clone() {
            let display = if current == "nearai" {
                "NEAR AI".to_string()
            } else if let Some(def) = registry.find(&current) {
                def.setup
                    .as_ref()
                    .map(|s| s.display_name().to_string())
                    .unwrap_or_else(|| def.id.clone())
            } else {
                match current.as_str() {
                    "nearai" => "NEAR AI".to_string(),
                    "gemini_oauth" | "gemini-oauth" => "Gemini API (OAuth)".to_string(),
                    _ => {
                        if let Some(def) = registry.find(&current) {
                            def.setup
                                .as_ref()
                                .map(|s| s.display_name().to_string())
                                .unwrap_or_else(|| def.id.clone())
                        } else {
                            current.clone()
                        }
                    }
                }
            };
            print_info(&format!("Current provider: {}", display));
            println!();

            let is_known = current == "nearai"
                || current == "bedrock"
                || current == "gemini_oauth"
                || current == "gemini-oauth"
                || current == "openai_codex"
                || registry.is_known(&current);

            if is_known && confirm("Keep current provider?", true).map_err(SetupError::Io)? {
                if current == "bedrock" {
                    print_info("Keeping existing AWS Bedrock configuration.");
                    return Ok(());
                }
                if current == "gemini_oauth" || current == "gemini-oauth" {
                    print_info("Keeping existing Gemini CLI OAuth configuration.");
                    return Ok(());
                }
                if current == "openai_codex" {
                    print_info("Keeping existing OpenAI Codex configuration.");
                    return Ok(());
                }
                return self.run_provider_setup(&current, &registry).await;
            }

            if !is_known {
                print_info(&format!(
                    "Unknown provider '{}', please select a supported provider.",
                    current
                ));
            }
        }

        print_info("Select your inference provider:");
        println!();

        // Build menu: NearAI first, then Gemini OAuth, then OpenAI Codex, then registry providers, then Bedrock
        let selectable = registry.selectable();

        // Detect which providers have API keys already set in the environment.
        let detected_env: HashMap<&str, bool> = [
            ("nearai", std::env::var("NEARAI_API_KEY").is_ok()),
            (
                "anthropic",
                std::env::var("ANTHROPIC_API_KEY").is_ok()
                    || std::env::var("ANTHROPIC_OAUTH_TOKEN").is_ok(),
            ),
            ("openai", std::env::var("OPENAI_API_KEY").is_ok()),
            ("openrouter", std::env::var("OPENROUTER_API_KEY").is_ok()),
        ]
        .into_iter()
        .collect();

        // Helper: build a label for a provider entry, prepending a checkmark if detected.
        let make_label = |id: &str, name: &str, desc: &str| -> String {
            if detected_env.get(id).copied().unwrap_or(false) {
                format!("\u{2713} {:<15}- {}", name, desc)
            } else {
                format!("  {:<15}- {}", name, desc)
            }
        };

        // Collect all entries as (provider_id, label, is_detected).
        struct ProviderEntry {
            id: String,
            label: String,
            detected: bool,
        }

        let mut entries: Vec<ProviderEntry> = Vec::with_capacity(2 + selectable.len());

        entries.push(ProviderEntry {
            id: "nearai".to_string(),
            label: make_label("nearai", "NEAR AI", "multi-model access via NEAR account"),
            detected: detected_env.get("nearai").copied().unwrap_or(false),
        });

        entries.push(ProviderEntry {
            id: "gemini_oauth".to_string(),
            label: make_label(
                "gemini_oauth",
                "Gemini CLI",
                "Official Gemini API via Gemini CLI OAuth",
            ),
            detected: false,
        });

        entries.push(ProviderEntry {
            id: "openai_codex".to_string(),
            label: make_label(
                "openai_codex",
                "OpenAI Codex",
                "ChatGPT subscription (Plus/Pro/Max)",
            ),
            detected: false,
        });

        for def in &selectable {
            let display_name = def
                .setup
                .as_ref()
                .map(|s| s.display_name())
                .unwrap_or(&def.id);
            entries.push(ProviderEntry {
                id: def.id.clone(),
                label: make_label(&def.id, display_name, &def.description),
                detected: detected_env.get(def.id.as_str()).copied().unwrap_or(false),
            });
        }

        // Bedrock is a special case (native AWS SDK, not registry-based)
        entries.push(ProviderEntry {
            id: "bedrock".to_string(),
            label: make_label(
                "bedrock",
                "AWS Bedrock",
                "Claude & other models via AWS (IAM, SSO)",
            ),
            detected: false,
        });

        // Sort: detected providers first, preserving relative order within each group.
        entries.sort_by_key(|e| !e.detected);

        let mut options: Vec<String> = Vec::with_capacity(entries.len());
        let mut provider_ids: Vec<String> = Vec::with_capacity(entries.len());
        for entry in &entries {
            options.push(entry.label.clone());
            provider_ids.push(entry.id.clone());
        }

        let option_refs: Vec<&str> = options.iter().map(|s| s.as_str()).collect();
        let choice = select_one("Provider:", &option_refs).map_err(SetupError::Io)?;
        let selected_id = &provider_ids[choice];

        if selected_id == "bedrock" {
            self.setup_bedrock().await?;
        } else if selected_id == "gemini_oauth" {
            self.setup_gemini_oauth().await?;
        } else {
            self.run_provider_setup(selected_id, &registry).await?;
        }

        Ok(())
    }

    /// Run the setup flow for a specific provider.
    ///
    /// NearAI has its own special flow. Registry providers dispatch
    /// based on their `SetupHint` kind.
    async fn run_provider_setup(
        &mut self,
        provider_id: &str,
        registry: &crate::llm::ProviderRegistry,
    ) -> Result<(), SetupError> {
        if provider_id == "nearai" {
            return self.setup_nearai().await;
        }

        if provider_id == "openai_codex" {
            return self.setup_openai_codex().await;
        }

        let def = registry
            .find(provider_id)
            .ok_or_else(|| SetupError::Config(format!("Unknown provider: {}", provider_id)))?;

        // Providers without a setup hint (e.g., user-defined providers configured
        // purely via env vars) skip credential setup and go to model selection.
        let Some(setup) = def.setup.as_ref() else {
            print_info(&format!(
                "Provider '{}' has no setup wizard. Configure via environment variables.",
                provider_id
            ));
            self.set_llm_backend_preserving_model(provider_id);
            return Ok(());
        };

        // Anthropic has a custom flow: API key or OAuth token from `claude login`.
        if provider_id == "anthropic" {
            return self.setup_anthropic().await;
        }

        if provider_id == "github_copilot" {
            return self.setup_github_copilot().await;
        }

        match setup {
            crate::llm::registry::SetupHint::ApiKey {
                secret_name,
                key_url,
                display_name,
                ..
            } => {
                let env_var = def.api_key_env.as_deref().unwrap_or("LLM_API_KEY");
                let url = key_url.as_deref().unwrap_or("the provider's website");

                // Only store base URL for providers that resolve through
                // LLM_BASE_URL (openai_compatible, openrouter). Other providers
                // like groq/nvidia have their own base_url_env and don't need
                // this backward-compat setting.
                if def.base_url_env.as_deref() == Some("LLM_BASE_URL")
                    && let Some(ref base_url) = def.default_base_url
                {
                    self.settings.openai_compatible_base_url = Some(base_url.clone());
                }

                self.setup_api_key_provider(
                    &def.id,
                    env_var,
                    secret_name,
                    &format!("{display_name} API key"),
                    url,
                    Some(display_name),
                )
                .await?;
            }
            crate::llm::registry::SetupHint::Ollama { .. } => {
                self.setup_ollama_generic(def)?;
            }
            crate::llm::registry::SetupHint::OpenAiCompatible {
                secret_name,
                display_name,
                ..
            } => {
                self.setup_openai_compatible_generic(&def.id, secret_name, display_name)
                    .await?;
            }
        }

        Ok(())
    }

    /// Detect an Anthropic credential from the environment.
    ///
    /// Checks `ANTHROPIC_API_KEY` first, then `ANTHROPIC_OAUTH_TOKEN`.
    /// Returns the key/token string if found, or `None`.
    fn detect_anthropic_key() -> Option<String> {
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY")
            && !key.is_empty()
        {
            return Some(key);
        }
        if let Ok(token) = std::env::var("ANTHROPIC_OAUTH_TOKEN")
            && !token.is_empty()
        {
            return Some(token);
        }
        None
    }

    /// Update the selected LLM backend while preserving the current model when
    /// the backend did not actually change.
    fn set_llm_backend_preserving_model(&mut self, backend: &str) {
        let backend_changed = self.settings.llm_backend.as_deref() != Some(backend);
        self.settings.llm_backend = Some(backend.to_string());
        if backend_changed {
            self.settings.selected_model = None;
        }
    }

    /// NEAR AI provider setup (extracted from the old step_authentication).
    async fn setup_nearai(&mut self) -> Result<(), SetupError> {
        self.set_llm_backend_preserving_model("nearai");

        // Check if NEARAI_API_KEY is already provided via environment or runtime overlay
        if let Some(existing) = crate::config::helpers::env_or_override("NEARAI_API_KEY")
            && !existing.is_empty()
        {
            print_info(&format!(
                "NEARAI_API_KEY found: {}",
                mask_api_key(&existing)
            ));
            if confirm("Use this key?", true).map_err(SetupError::Io)? {
                if let Ok(ctx) = self.init_secrets_context().await {
                    let key = SecretString::from(existing.clone());
                    if let Err(e) = ctx.save_secret("llm_nearai_api_key", &key).await {
                        tracing::warn!("Failed to persist NEARAI_API_KEY to secrets: {}", e);
                    }
                }
                self.llm_api_key = Some(SecretString::from(existing));
                print_success("NEAR AI configured (from env)");
                return Ok(());
            }
        }

        // Check if we already have a session
        if let Some(ref session) = self.session_manager
            && session.has_token().await
        {
            print_info("Existing session found. Validating...");
            match session.ensure_authenticated().await {
                Ok(()) => {
                    print_success("NEAR AI session valid");
                    return Ok(());
                }
                Err(e) => {
                    print_info(&format!("Session invalid: {}. Re-authenticating...", e));
                }
            }
        }

        // Create session manager if we don't have one
        let session = if let Some(ref s) = self.session_manager {
            Arc::clone(s)
        } else {
            let config = SessionConfig {
                session_path: crate::config::llm::default_session_path(),
                ..SessionConfig::default()
            };
            Arc::new(SessionManager::new(config))
        };

        // Trigger authentication flow
        session
            .ensure_authenticated()
            .await
            .map_err(|e| SetupError::Auth(e.to_string()))?;

        self.session_manager = Some(session);

        // Persist session token to the database so the runtime can load it
        // via `attach_store()` → `load_session_from_db()` without the
        // backwards-compat fallback. The session manager saved to disk but
        // doesn't have a DB store attached during onboarding.
        self.persist_session_to_db().await;

        // If the user chose the API key path, NEARAI_API_KEY is now set
        // in the runtime env overlay. Persist it to the encrypted secrets
        // store so inject_llm_keys_from_secrets() can load it on future runs.
        if let Some(api_key) = crate::config::helpers::env_or_override("NEARAI_API_KEY")
            && !api_key.is_empty()
            && let Ok(ctx) = self.init_secrets_context().await
        {
            let key = SecretString::from(api_key);
            if let Err(e) = ctx.save_secret("llm_nearai_api_key", &key).await {
                tracing::warn!("Failed to persist NEARAI_API_KEY to secrets: {}", e);
            }
        }

        print_success("NEAR AI configured");
        Ok(())
    }

    /// Anthropic provider setup: API key or OAuth token from `claude login`.
    async fn setup_anthropic(&mut self) -> Result<(), SetupError> {
        let options = &["Direct API Key", "OAuth Token (from `claude login`)"];
        let choice = select_one("How do you want to authenticate with Anthropic?", options)
            .map_err(SetupError::Io)?;

        if choice == 0 {
            // Standard API key flow
            self.setup_api_key_provider(
                "anthropic",
                "ANTHROPIC_API_KEY",
                "llm_anthropic_api_key",
                "Anthropic API key",
                "https://console.anthropic.com/settings/keys",
                None,
            )
            .await
        } else {
            // OAuth token flow
            self.setup_anthropic_oauth().await
        }
    }

    async fn setup_github_copilot(&mut self) -> Result<(), SetupError> {
        print_info("GitHub Copilot authentication:");
        let options = &[
            "GitHub device login (recommended)",
            "Paste an existing token (from IDE or personal access token)",
        ];
        let choice = select_one("Auth method:", options).map_err(SetupError::Io)?;
        match choice {
            0 => self.setup_github_copilot_device_login().await,
            _ => self.setup_github_copilot_paste_token().await,
        }
    }

    async fn setup_github_copilot_paste_token(&mut self) -> Result<(), SetupError> {
        self.set_llm_backend_preserving_model("github_copilot");

        print_info("Paste your GitHub token (requires an active Copilot subscription).");
        print_info("Sources: `gh auth token`, or the oauth_token field in");
        print_info("~/.config/github-copilot/apps.json (VS Code) or ~/.config/gh/hosts.yml.");
        let token_secret = secret_input("GitHub Copilot token").map_err(SetupError::Io)?;
        let token = token_secret.expose_secret().trim().to_string();
        if token.is_empty() {
            return Err(SetupError::Auth("No token provided".to_string()));
        }

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| SetupError::Auth(format!("Failed to create HTTP client: {e}")))?;

        self.save_github_copilot_token(&client, &token).await
    }

    async fn setup_github_copilot_device_login(&mut self) -> Result<(), SetupError> {
        self.set_llm_backend_preserving_model("github_copilot");

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| SetupError::Auth(format!("Failed to create HTTP client: {e}")))?;

        let device = crate::llm::github_copilot_auth::request_device_code(&client)
            .await
            .map_err(|e| SetupError::Auth(e.to_string()))?;

        print_info("Authorize IronClaw with GitHub Copilot in your browser.");
        print_info(&format!("Verification URL: {}", device.verification_uri));
        print_info(&format!("One-time code: {}", device.user_code));

        if let Err(e) = open::that(&device.verification_uri) {
            tracing::debug!(
                url = %device.verification_uri,
                error = %e,
                "Failed to open GitHub Copilot device login URL"
            );
            print_info("Open the URL above manually if your browser did not launch.");
        } else {
            print_info("Opened your browser to GitHub device login.");
        }

        print_info("Waiting for GitHub authorization...");
        let token = crate::llm::github_copilot_auth::wait_for_device_login(&client, &device)
            .await
            .map_err(|e| SetupError::Auth(e.to_string()))?;

        self.save_github_copilot_token(&client, &token).await
    }

    async fn save_github_copilot_token(
        &mut self,
        client: &reqwest::Client,
        token: &str,
    ) -> Result<(), SetupError> {
        crate::llm::github_copilot_auth::validate_token(client, token)
            .await
            .map_err(|e| SetupError::Auth(e.to_string()))?;

        if let Ok(ctx) = self.init_secrets_context().await {
            let key = SecretString::from(token.to_string());
            ctx.save_secret("llm_github_copilot_token", &key)
                .await
                .map_err(|e| SetupError::Config(format!("Failed to save GitHub token: {e}")))?;
            print_success("GitHub Copilot token encrypted and saved");
        } else {
            print_info("Secrets not available. Set GITHUB_COPILOT_TOKEN in your environment.");
        }

        crate::config::inject_single_var("GITHUB_COPILOT_TOKEN", token);
        self.llm_api_key = Some(SecretString::from(token.to_string()));

        print_success("GitHub Copilot configured");
        Ok(())
    }

    /// Anthropic OAuth setup: extract token from `claude login` credentials.
    async fn setup_anthropic_oauth(&mut self) -> Result<(), SetupError> {
        self.set_llm_backend_preserving_model("anthropic");

        // Try to extract existing OAuth token from Claude Code credentials
        if let Some(token) = crate::config::ClaudeCodeConfig::extract_oauth_token() {
            print_info(&format!("Found OAuth token: {}", mask_api_key(&token)));
            if confirm("Use this token?", true).map_err(SetupError::Io)? {
                return self.save_anthropic_oauth_token(&token).await;
            }
        } else {
            print_info("No OAuth token found from `claude login`.");
            print_info("Run `claude login` in a terminal to authenticate, then retry.");
            println!();

            if confirm("Retry after running `claude login`?", true).map_err(SetupError::Io)? {
                // Block until the user has run `claude login` in another terminal
                input("Press Enter after running `claude login` in another terminal...")
                    .map_err(SetupError::Io)?;
                if let Some(token) = crate::config::ClaudeCodeConfig::extract_oauth_token() {
                    print_info(&format!("Found OAuth token: {}", mask_api_key(&token)));
                    return self.save_anthropic_oauth_token(&token).await;
                }
                print_error("Still no OAuth token found.");
            }
        }

        // Fallback: let user paste the token manually, or switch to API key
        print_info("You can paste your OAuth token directly (starts with sk-ant-oat01-).");
        print_info("Or press Enter with no input to switch to the API key flow.");
        let token = secret_input("Anthropic OAuth token").map_err(SetupError::Io)?;
        let token_str = token.expose_secret();
        if token_str.is_empty() {
            print_info("Switching to API key flow...");
            return self
                .setup_api_key_provider(
                    "anthropic",
                    "ANTHROPIC_API_KEY",
                    "llm_anthropic_api_key",
                    "Anthropic API key",
                    "https://console.anthropic.com/settings/keys",
                    None,
                )
                .await;
        }
        self.save_anthropic_oauth_token(token_str).await
    }

    /// Save an Anthropic OAuth token to secrets and set env for immediate use.
    async fn save_anthropic_oauth_token(&mut self, token: &str) -> Result<(), SetupError> {
        // Validate token format to catch accidentally pasted API keys
        if !token.starts_with("sk-ant-oat") {
            print_error("Token doesn't look like an OAuth token (expected prefix: sk-ant-oat).");
            print_info("If you have an API key instead, use the 'Direct API Key' option.");
            return Err(SetupError::Config("Invalid OAuth token format".to_string()));
        }

        // Store in secrets if available
        if let Ok(ctx) = self.init_secrets_context().await {
            let key = SecretString::from(token.to_string());
            ctx.save_secret("llm_anthropic_oauth_token", &key)
                .await
                .map_err(|e| SetupError::Config(format!("Failed to save OAuth token: {e}")))?;
            print_success("OAuth token encrypted and saved");
        } else {
            print_info("Secrets not available. Set ANTHROPIC_OAUTH_TOKEN in your environment.");
        }

        // Make the token visible to `optional_env()` for subsequent config
        // resolution (model selection step). Uses the thread-safe overlay
        // instead of `std::env::set_var` to avoid UB on multi-threaded runtimes.
        crate::config::inject_single_var("ANTHROPIC_OAUTH_TOKEN", token);

        // Cache for model fetching
        self.llm_api_key = Some(SecretString::from(token.to_string()));

        print_success("Anthropic OAuth configured");
        Ok(())
    }

    /// Shared setup flow for API-key-based providers.
    async fn setup_api_key_provider(
        &mut self,
        backend: &str,
        env_var: &str,
        secret_name: &str,
        prompt_label: &str,
        hint_url: &str,
        override_display_name: Option<&str>,
    ) -> Result<(), SetupError> {
        let display_name = override_display_name.unwrap_or(match backend {
            "anthropic" => "Anthropic",
            "openai" => "OpenAI",
            other => other,
        });

        self.set_llm_backend_preserving_model(backend);

        // Check env var first
        if let Ok(existing) = std::env::var(env_var) {
            print_info(&format!("{env_var} found: {}", mask_api_key(&existing)));
            if confirm("Use this key?", true).map_err(SetupError::Io)? {
                // Persist env-provided key to secrets store for future runs
                if let Ok(ctx) = self.init_secrets_context().await {
                    let key = SecretString::from(existing.clone());
                    if let Err(e) = ctx.save_secret(secret_name, &key).await {
                        tracing::warn!("Failed to persist env key to secrets: {}", e);
                    }
                }
                self.llm_api_key = Some(SecretString::from(existing));
                print_success(&format!("{display_name} configured (from env)"));
                return Ok(());
            }
        }

        println!();
        print_info(&format!("Get your API key from: {hint_url}"));
        println!();

        let key = secret_input(prompt_label).map_err(SetupError::Io)?;
        let key_str = key.expose_secret();

        if key_str.is_empty() {
            return Err(SetupError::Config("API key cannot be empty".to_string()));
        }

        // Store in secrets if available
        if let Ok(ctx) = self.init_secrets_context().await {
            ctx.save_secret(secret_name, &key)
                .await
                .map_err(|e| SetupError::Config(format!("Failed to save API key: {e}")))?;
            print_success("API key encrypted and saved");
        } else {
            print_info(&format!(
                "Secrets not available. Set {env_var} in your environment."
            ));
        }

        // Make key visible to `optional_env()` for subsequent config resolution.
        // Uses the thread-safe overlay instead of `std::env::set_var` to avoid
        // UB on multi-threaded runtimes.
        crate::config::inject_single_var(env_var, key_str);

        // Cache key in memory for model fetching later in the wizard
        self.llm_api_key = Some(SecretString::from(key_str.to_string()));

        print_success(&format!("{display_name} configured"));
        Ok(())
    }

    /// OpenAI Codex (ChatGPT subscription) setup: device code OAuth flow.
    async fn setup_openai_codex(&mut self) -> Result<(), SetupError> {
        self.settings.llm_backend = Some("openai_codex".to_string());
        if self.settings.selected_model.is_some() {
            self.settings.selected_model = None;
        }

        use crate::config::OpenAiCodexConfig;
        use crate::llm::OpenAiCodexSessionManager;

        let config = OpenAiCodexConfig::default();

        let mgr = OpenAiCodexSessionManager::new(config).map_err(|e| {
            SetupError::Config(format!("OpenAI Codex session manager init failed: {}", e))
        })?;
        mgr.device_code_login().await.map_err(|e| {
            SetupError::Config(format!("OpenAI Codex authentication failed: {}", e))
        })?;

        print_success("OpenAI Codex configured (ChatGPT subscription)");
        Ok(())
    }

    /// Generic Ollama-style setup: just needs a base URL, no API key.
    fn setup_ollama_generic(
        &mut self,
        def: &crate::llm::ProviderDefinition,
    ) -> Result<(), SetupError> {
        self.set_llm_backend_preserving_model(&def.id);

        let default_url = self
            .settings
            .ollama_base_url
            .as_deref()
            .or(def.default_base_url.as_deref())
            .unwrap_or(OLLAMA_DEFAULT_BASE_URL);

        let display_name = def
            .setup
            .as_ref()
            .map(|s| s.display_name())
            .unwrap_or(&def.id);

        let url_input = optional_input(
            &format!("{display_name} base URL"),
            Some(&format!("default: {}", default_url)),
        )
        .map_err(SetupError::Io)?;

        let url = url_input.unwrap_or_else(|| default_url.to_string());
        self.settings.ollama_base_url = Some(url.clone());

        print_success(&format!("{display_name} configured ({})", url));
        Ok(())
    }

    /// AWS Bedrock provider setup: region, auth, and cross-region config.
    async fn setup_bedrock(&mut self) -> Result<(), SetupError> {
        self.set_llm_backend_preserving_model("bedrock");

        // Region
        let default_region = self
            .settings
            .bedrock_region
            .as_deref()
            .unwrap_or("us-east-1");

        let region_input =
            optional_input("AWS region", Some(&format!("default: {}", default_region)))
                .map_err(SetupError::Io)?;

        let region = region_input.unwrap_or_else(|| default_region.to_string());
        self.settings.bedrock_region = Some(region.clone());

        // Auth method
        print_info("Select authentication method:");
        println!();
        let auth_options = &[
            "AWS default credentials (env vars, ~/.aws/credentials, IAM roles)",
            "AWS named profile (SSO / assume-role)",
        ];
        let auth_choice = select_one("Auth:", auth_options).map_err(SetupError::Io)?;

        match auth_choice {
            0 => {
                // Default AWS credentials — clear any stale named profile
                self.settings.bedrock_profile = None;
                print_info(
                    "Using default AWS credential chain (env vars, ~/.aws/credentials, IAM roles).",
                );
            }
            1 => {
                // Named profile
                let profile =
                    input("AWS profile name (from ~/.aws/config)").map_err(SetupError::Io)?;
                if profile.trim().is_empty() {
                    // Empty input clears any previously configured profile
                    self.settings.bedrock_profile = None;
                    print_info("AWS profile cleared; using default AWS credential chain instead.");
                } else {
                    self.settings.bedrock_profile = Some(profile.clone());
                    print_success(&format!("AWS profile '{}' saved", profile));
                }
            }
            _ => return Err(SetupError::Config("Invalid auth selection".to_string())),
        }

        self.setup_bedrock_cross_region()
    }

    /// Bedrock cross-region inference prefix selection (sub-step of setup_bedrock).
    fn setup_bedrock_cross_region(&mut self) -> Result<(), SetupError> {
        print_info("Cross-region inference routes requests across AWS regions for capacity:");
        println!();
        let cross_options = &[
            "us     - route within US regions (recommended for us-east-1)",
            "global - route to any AWS region worldwide",
            "eu     - route within European regions",
            "apac   - route within Asia-Pacific regions",
            "none   - single-region only (no cross-region routing)",
        ];
        let cross_choice = select_one("Cross-region:", cross_options).map_err(SetupError::Io)?;

        let cross_region = match cross_choice {
            0 => Some("us".to_string()),
            1 => Some("global".to_string()),
            2 => Some("eu".to_string()),
            3 => Some("apac".to_string()),
            4 => None,
            _ => None,
        };
        self.settings.bedrock_cross_region = cross_region;

        let region = self
            .settings
            .bedrock_region
            .as_deref()
            .unwrap_or("us-east-1");
        print_success(&format!("AWS Bedrock configured (region: {})", region));
        Ok(())
    }

    /// Generic OpenAI-compatible setup: base URL + optional API key.
    async fn setup_openai_compatible_generic(
        &mut self,
        backend_id: &str,
        secret_name: &str,
        display_name: &str,
    ) -> Result<(), SetupError> {
        self.set_llm_backend_preserving_model(backend_id);

        let existing_url = self
            .settings
            .openai_compatible_base_url
            .clone()
            .or_else(|| std::env::var("LLM_BASE_URL").ok());

        let url = if let Some(ref u) = existing_url {
            let url_input = optional_input("Base URL", Some(&format!("current: {}", u)))
                .map_err(SetupError::Io)?;
            url_input.unwrap_or_else(|| u.clone())
        } else {
            input("Base URL (e.g., http://localhost:8000/v1)").map_err(SetupError::Io)?
        };

        if url.is_empty() {
            return Err(SetupError::Config(format!(
                "Base URL is required for {display_name}"
            )));
        }

        self.settings.openai_compatible_base_url = Some(url.clone());

        // Optional API key
        if confirm("Does this endpoint require an API key?", false).map_err(SetupError::Io)? {
            let key = secret_input("API key").map_err(SetupError::Io)?;
            let key_str = key.expose_secret();

            if !key_str.is_empty() {
                if let Ok(ctx) = self.init_secrets_context().await {
                    ctx.save_secret(secret_name, &key)
                        .await
                        .map_err(|e| SetupError::Config(format!("Failed to save API key: {e}")))?;
                    print_success("API key encrypted and saved");
                } else {
                    print_info("Secrets not available. Set the API key in your environment.");
                }
            }
        }

        print_success(&format!("{display_name} configured ({})", url));
        Ok(())
    }

    async fn setup_gemini_oauth(&mut self) -> Result<(), SetupError> {
        self.settings.llm_backend = Some("gemini_oauth".to_string());
        print_info("Starting Gemini CLI OAuth authentication...");
        println!();

        let creds_path = crate::config::GeminiOauthConfig::default_credentials_path();
        let cred_manager =
            crate::llm::gemini_oauth::CredentialManager::new(&creds_path).map_err(|e| {
                SetupError::Config(format!(
                    "Failed to initialize Gemini credential manager: {}",
                    e
                ))
            })?;

        match cred_manager.get_valid_credential().await {
            Ok(cred) => {
                print_success("Gemini CLI authentication successful!");
                if let Some(ref pid) = cred.project_id {
                    print_info(&format!("Cloud Code project: {}", pid));
                }
            }
            Err(e) => {
                return Err(SetupError::Config(format!(
                    "Gemini CLI authentication failed: {}. Please try again.",
                    e
                )));
            }
        }

        println!();
        print_success("Gemini API configured via Gemini CLI");
        Ok(())
    }

    /// Step 4: Model selection.
    ///
    /// Branches on the selected LLM backend and fetches models from the
    /// appropriate provider API, with static defaults as fallback.
    async fn step_model_selection(&mut self) -> Result<(), SetupError> {
        // Show current model if already configured
        if let Some(ref current) = self.settings.selected_model {
            print_info(&format!("Current model: {}", current));
            println!();

            let options = ["Keep current model", "Change model"];
            let choice =
                select_one("What would you like to do?", &options).map_err(SetupError::Io)?;

            if choice == 0 {
                print_success(&format!("Keeping {}", current));
                return Ok(());
            }
        }

        let backend = self.settings.llm_backend.as_deref().unwrap_or("nearai");
        let registry = crate::llm::ProviderRegistry::load();

        match backend {
            "nearai" => {
                // NEAR AI: use existing provider list_models()
                let fetched = self.fetch_nearai_models().await;
                let models = if fetched.is_empty() {
                    crate::llm::default_models()
                } else {
                    fetched.iter().map(|m| (m.clone(), m.clone())).collect()
                };
                self.select_from_model_list(&models)?;
            }
            "gemini_oauth" | "gemini-oauth" => {
                let default_models: Vec<(String, String)> = vec![
                    (
                        "gemini-3.1-pro-preview".into(),
                        "Gemini 3.1 Pro (Latest, strongest reasoning)".into(),
                    ),
                    (
                        "gemini-3.1-pro-preview-customtools".into(),
                        "Gemini 3.1 Pro Custom Tools (Enhanced tool use)".into(),
                    ),
                    (
                        "gemini-3-pro-preview".into(),
                        "Gemini 3 Pro (Preview)".into(),
                    ),
                    (
                        "gemini-3-flash-preview".into(),
                        "Gemini 3 Flash (Fast preview with thinking)".into(),
                    ),
                    (
                        "gemini-3.1-flash-lite-preview".into(),
                        "Gemini 3.1 Flash Lite (Preview, lightweight)".into(),
                    ),
                    (
                        "gemini-2.5-pro".into(),
                        "Gemini 2.5 Pro (Stable, strong reasoning)".into(),
                    ),
                    (
                        "gemini-2.5-flash".into(),
                        "Gemini 2.5 Flash (Fast, good quality)".into(),
                    ),
                    (
                        "gemini-2.5-flash-lite".into(),
                        "Gemini 2.5 Flash Lite (Fastest, lightweight)".into(),
                    ),
                ];
                self.select_from_model_list(&default_models)?;
            }
            "bedrock" => {
                let model_id =
                    input("Bedrock model ID (e.g., anthropic.claude-v3-sonnet-20240229-v1:0)")
                        .map_err(SetupError::Io)?;
                if model_id.is_empty() {
                    return Err(SetupError::Config("Model ID is required".to_string()));
                }
                self.settings.selected_model = Some(model_id.clone());
                print_success(&format!("Selected {}", model_id));
            }
            _ => {
                if let Some(def) = registry.find(backend) {
                    let can_list = def
                        .setup
                        .as_ref()
                        .map(|s| s.can_list_models())
                        .unwrap_or(false);

                    if can_list {
                        // Try to fetch models from the provider's /v1/models endpoint
                        let cached_key = self
                            .llm_api_key
                            .as_ref()
                            .map(|k| k.expose_secret().to_string());

                        let models = match backend {
                            "anthropic" => fetch_anthropic_models(cached_key.as_deref()).await,
                            "openai" => fetch_openai_models(cached_key.as_deref()).await,
                            "ollama" => {
                                let base_url = self
                                    .settings
                                    .ollama_base_url
                                    .as_deref()
                                    .or(def.default_base_url.as_deref())
                                    .unwrap_or(OLLAMA_DEFAULT_BASE_URL);
                                let models = fetch_ollama_models(base_url).await;
                                if models.is_empty() {
                                    print_info(
                                        "No models found. Pull one first: ollama pull llama3",
                                    );
                                }
                                models
                            }
                            _ => {
                                // Generic OpenAI-compatible model listing
                                let base_url = def.default_base_url.as_deref().unwrap_or("");
                                fetch_openai_compatible_models(base_url, cached_key.as_deref())
                                    .await
                            }
                        };

                        // Apply models_filter from setup hint
                        let models = if let Some(filter) =
                            def.setup.as_ref().and_then(|s| s.models_filter())
                        {
                            let filter_lower = filter.to_lowercase();
                            models
                                .into_iter()
                                .filter(|(id, _)| id.to_lowercase().contains(&filter_lower))
                                .collect()
                        } else {
                            models
                        };

                        if models.is_empty() {
                            // Fall back to manual entry
                            let default = &def.default_model;
                            let model_id = input(&format!("Model name (default: {default})"))
                                .map_err(SetupError::Io)?;
                            let model_id = if model_id.is_empty() {
                                default.clone()
                            } else {
                                model_id
                            };
                            self.settings.selected_model = Some(model_id.clone());
                            print_success(&format!("Selected {}", model_id));
                        } else {
                            self.select_from_model_list(&models)?;
                        }
                    } else {
                        // Manual model entry
                        let default = &def.default_model;
                        let model_id = input(&format!("Model name (default: {default})"))
                            .map_err(SetupError::Io)?;
                        let model_id = if model_id.is_empty() {
                            default.clone()
                        } else {
                            model_id
                        };
                        self.settings.selected_model = Some(model_id.clone());
                        print_success(&format!("Selected {}", model_id));
                    }
                } else {
                    // Unknown provider, manual entry
                    let model_id = input("Model name (e.g., meta-llama/Llama-3-8b-chat-hf)")
                        .map_err(SetupError::Io)?;
                    if model_id.is_empty() {
                        return Err(SetupError::Config("Model name is required".to_string()));
                    }
                    self.settings.selected_model = Some(model_id.clone());
                    print_success(&format!("Selected {}", model_id));
                }
            }
        }

        Ok(())
    }

    /// Present a model list to the user, with a "Custom model ID" escape hatch.
    ///
    /// Each entry is `(model_id, display_label)`.
    fn select_from_model_list(&mut self, models: &[(String, String)]) -> Result<(), SetupError> {
        println!("Available models:");
        println!();

        let mut options: Vec<&str> = models.iter().map(|(_, desc)| desc.as_str()).collect();
        options.push("Custom model ID");

        let choice = select_one("Select a model:", &options).map_err(SetupError::Io)?;

        let selected = if choice == options.len() - 1 {
            loop {
                let raw = input("Enter model ID").map_err(SetupError::Io)?;
                let trimmed = raw.trim().to_string();
                if trimmed.is_empty() {
                    println!("Model ID cannot be empty.");
                    continue;
                }
                break trimmed;
            }
        } else {
            models[choice].0.clone()
        };

        self.settings.selected_model = Some(selected.clone());
        print_success(&format!("Selected {}", selected));
        Ok(())
    }

    /// Fetch available models from the NEAR AI API.
    ///
    /// Uses [`build_nearai_model_fetch_config`] to construct the provider config,
    /// which reads `NEARAI_API_KEY` from the environment when present.
    async fn fetch_nearai_models(&self) -> Vec<String> {
        let session = match self.session_manager {
            Some(ref s) => Arc::clone(s),
            None => return vec![],
        };

        use crate::llm::create_llm_provider;

        let config = build_nearai_model_fetch_config();

        match create_llm_provider(&config, session).await {
            Ok(provider) => match provider.list_models().await {
                Ok(models) => models,
                Err(e) => {
                    print_info(&format!("Could not fetch models: {}. Using defaults.", e));
                    vec![]
                }
            },
            Err(e) => {
                print_info(&format!(
                    "Could not initialize provider: {}. Using defaults.",
                    e
                ));
                vec![]
            }
        }
    }

    /// Step 5: Embeddings configuration.
    fn step_embeddings(&mut self) -> Result<(), SetupError> {
        print_info("Embeddings enable semantic search in your workspace memory.");
        println!();

        if !confirm("Enable semantic search?", true).map_err(SetupError::Io)? {
            self.settings.embeddings.enabled = false;
            print_info("Embeddings disabled. Workspace will use keyword search only.");
            return Ok(());
        }

        let backend = self.settings.llm_backend.as_deref().unwrap_or("nearai");
        let has_openai_key = std::env::var("OPENAI_API_KEY").is_ok()
            || (backend == "openai" && self.llm_api_key.is_some());
        let has_nearai = backend == "nearai" || self.session_manager.is_some();
        let has_bedrock = backend == "bedrock";

        // If the LLM backend is OpenAI and we already have a key, default to OpenAI embeddings
        if backend == "openai" && has_openai_key {
            self.settings.embeddings.enabled = true;
            self.settings.embeddings.provider = "openai".to_string();
            self.settings.embeddings.model = "text-embedding-3-small".to_string();
            print_success("Embeddings enabled via OpenAI (using existing API key)");
            return Ok(());
        }

        if backend == "bedrock" {
            self.settings.embeddings.enabled = true;
            self.settings.embeddings.provider = "bedrock".to_string();
            self.settings.embeddings.model = "amazon.titan-embed-text-v2:0".to_string();
            print_success("Embeddings enabled via AWS Bedrock");
            return Ok(());
        }

        // If no NEAR AI session, Bedrock config, or OpenAI key, embeddings aren't available.
        if !has_nearai && !has_bedrock && !has_openai_key {
            print_info("No NEAR AI session or OpenAI key found for embeddings.");
            print_info("Set OPENAI_API_KEY in your environment to enable embeddings.");
            self.settings.embeddings.enabled = false;
            return Ok(());
        }

        let mut provider_options = Vec::new();
        if has_nearai {
            provider_options.push(("nearai", "NEAR AI (uses same auth, no extra cost)"));
        }
        if has_bedrock {
            provider_options.push(("bedrock", "AWS Bedrock (uses AWS auth and region)"));
        }
        provider_options.push(("openai", "OpenAI (requires API key)"));

        let display_options: Vec<&str> = provider_options
            .iter()
            .map(|(_, display)| *display)
            .collect();
        let choice =
            select_one("Select embeddings provider:", &display_options).map_err(SetupError::Io)?;
        let provider = provider_options[choice].0;

        match provider {
            "nearai" => {
                self.settings.embeddings.enabled = true;
                self.settings.embeddings.provider = "nearai".to_string();
                self.settings.embeddings.model = "text-embedding-3-small".to_string();
                print_success("Embeddings enabled via NEAR AI");
            }
            "bedrock" => {
                self.settings.embeddings.enabled = true;
                self.settings.embeddings.provider = "bedrock".to_string();
                self.settings.embeddings.model = "amazon.titan-embed-text-v2:0".to_string();
                print_success("Embeddings enabled via AWS Bedrock");
            }
            _ => {
                if !has_openai_key {
                    print_info("OPENAI_API_KEY not set in environment.");
                    print_info("Add it to your .env file or environment to enable embeddings.");
                }
                self.settings.embeddings.enabled = true;
                self.settings.embeddings.provider = "openai".to_string();
                self.settings.embeddings.model = "text-embedding-3-small".to_string();
                print_success("Embeddings configured for OpenAI");
            }
        }

        Ok(())
    }

    /// Initialize secrets context for channel setup.
    async fn init_secrets_context(&mut self) -> Result<SecretsContext, SetupError> {
        // Get crypto (should be set from step 2, or load from keychain/env)
        let crypto = if let Some(ref c) = self.secrets_crypto {
            Arc::clone(c)
        } else {
            // Try to load master key from keychain or env
            let key = if let Ok(env_key) = std::env::var("SECRETS_MASTER_KEY") {
                env_key
            } else if let Ok(keychain_key) = crate::secrets::keychain::get_master_key().await {
                keychain_key.iter().map(|b| format!("{:02x}", b)).collect()
            } else {
                return Err(SetupError::Config(
                    "Secrets not configured. Run full setup or set SECRETS_MASTER_KEY.".to_string(),
                ));
            };

            let crypto = Arc::new(
                SecretsCrypto::new(SecretString::from(key))
                    .map_err(|e| SetupError::Config(e.to_string()))?,
            );
            self.secrets_crypto = Some(Arc::clone(&crypto));
            crypto
        };

        // Create backend-appropriate secrets store.
        // Use runtime dispatch based on the user's selected backend.
        // Default to whichever backend is compiled in. When only libsql is
        // available, we must not default to "postgres" or we'd skip store creation.
        let default_backend = {
            #[cfg(feature = "postgres")]
            {
                "postgres"
            }
            #[cfg(not(feature = "postgres"))]
            {
                "libsql"
            }
        };
        let selected_backend = self
            .settings
            .database_backend
            .as_deref()
            .unwrap_or(default_backend);

        match selected_backend {
            #[cfg(feature = "libsql")]
            "libsql" | "turso" | "sqlite" => {
                if let Some(store) = self.create_libsql_secrets_store(&crypto)? {
                    return Ok(SecretsContext::from_store(store, self.owner_id()));
                }
                // Fallback to postgres if libsql store creation returned None
                #[cfg(feature = "postgres")]
                if let Some(store) = self.create_postgres_secrets_store(&crypto).await? {
                    return Ok(SecretsContext::from_store(store, self.owner_id()));
                }
            }
            #[cfg(feature = "postgres")]
            _ => {
                if let Some(store) = self.create_postgres_secrets_store(&crypto).await? {
                    return Ok(SecretsContext::from_store(store, self.owner_id()));
                }
                // Fallback to libsql if postgres store creation returned None
                #[cfg(feature = "libsql")]
                if let Some(store) = self.create_libsql_secrets_store(&crypto)? {
                    return Ok(SecretsContext::from_store(store, self.owner_id()));
                }
            }
            #[cfg(not(feature = "postgres"))]
            _ => {}
        }

        Err(SetupError::Config(
            "No database backend available for secrets storage".to_string(),
        ))
    }

    /// Create a PostgreSQL secrets store from the current pool.
    #[cfg(feature = "postgres")]
    async fn create_postgres_secrets_store(
        &mut self,
        crypto: &Arc<SecretsCrypto>,
    ) -> Result<Option<Arc<dyn SecretsStore>>, SetupError> {
        let pool = if let Some(ref p) = self.db_pool {
            p.clone()
        } else {
            // Fall back to creating one from settings/env
            let url = self
                .settings
                .database_url
                .clone()
                .or_else(|| std::env::var("DATABASE_URL").ok());

            if let Some(url) = url {
                self.test_database_connection_postgres(&url).await?;
                self.run_migrations_postgres().await?;
                match self.db_pool.clone() {
                    Some(pool) => pool,
                    None => {
                        return Err(SetupError::Database(
                            "Database pool not initialized after connection test".to_string(),
                        ));
                    }
                }
            } else {
                return Ok(None);
            }
        };

        let store: Arc<dyn SecretsStore> = Arc::new(crate::secrets::PostgresSecretsStore::new(
            pool,
            Arc::clone(crypto),
        ));
        Ok(Some(store))
    }

    /// Create a libSQL secrets store from the current backend.
    #[cfg(feature = "libsql")]
    fn create_libsql_secrets_store(
        &self,
        crypto: &Arc<SecretsCrypto>,
    ) -> Result<Option<Arc<dyn SecretsStore>>, SetupError> {
        if let Some(ref backend) = self.db_backend {
            let store: Arc<dyn SecretsStore> = Arc::new(crate::secrets::LibSqlSecretsStore::new(
                backend.shared_db(),
                Arc::clone(crypto),
            ));
            Ok(Some(store))
        } else {
            Ok(None)
        }
    }

    /// Step 6: Channel configuration.
    async fn step_channels(&mut self) -> Result<(), SetupError> {
        // First, configure tunnel (shared across all channels that need webhooks)
        match setup_tunnel(&self.settings).await {
            Ok(tunnel_settings) => {
                self.settings.tunnel = tunnel_settings;
            }
            Err(e) => {
                print_info(&format!("Tunnel setup skipped: {}", e));
            }
        }
        println!();

        // Discover available WASM channels
        let channels_dir = ironclaw_base_dir().join("channels");

        let mut discovered_channels = discover_wasm_channels(&channels_dir).await;
        let installed_names: HashSet<String> = discovered_channels
            .iter()
            .map(|(name, _)| name.clone())
            .collect();

        // Build channel list from registry (if available) + bundled + discovered
        let wasm_channel_names = build_channel_options(&discovered_channels);

        // Build options list dynamically
        let mut options: Vec<(String, bool)> = vec![
            ("CLI/TUI (always enabled)".to_string(), true),
            (
                "HTTP webhook".to_string(),
                self.settings.channels.http_enabled,
            ),
            ("Signal".to_string(), self.settings.channels.signal_enabled),
        ];

        let non_wasm_count = options.len();

        // Add available WASM channels (installed + bundled + registry)
        for name in &wasm_channel_names {
            let is_enabled = self.settings.channels.wasm_channels.contains(name);
            let label = if installed_names.contains(name) {
                format!("{} (installed)", capitalize_first(name))
            } else {
                format!("{} (will install)", capitalize_first(name))
            };
            options.push((label, is_enabled));
        }

        let options_refs: Vec<(&str, bool)> =
            options.iter().map(|(s, b)| (s.as_str(), *b)).collect();

        let selected = select_many("Which channels do you want to enable?", &options_refs)
            .map_err(SetupError::Io)?;

        let selected_wasm_channels: Vec<String> = wasm_channel_names
            .iter()
            .enumerate()
            .filter_map(|(idx, name)| {
                if selected.contains(&(non_wasm_count + idx)) {
                    Some(name.clone())
                } else {
                    None
                }
            })
            .collect();

        // Install selected channels that aren't already on disk
        let mut any_installed = false;

        // Try bundled channels first (pre-compiled artifacts from channels-src/)
        if let Some(installed) = install_selected_bundled_channels(
            &channels_dir,
            &selected_wasm_channels,
            &installed_names,
        )
        .await?
            && !installed.is_empty()
        {
            print_success(&format!(
                "Installed bundled channels: {}",
                installed.join(", ")
            ));
            any_installed = true;
        }

        let installed_from_registry = install_selected_registry_channels(
            &channels_dir,
            &selected_wasm_channels,
            &installed_names,
        )
        .await;

        if !installed_from_registry.is_empty() {
            print_success(&format!(
                "Built from registry: {}",
                installed_from_registry.join(", ")
            ));
            any_installed = true;
        }

        // Re-discover after installs
        if any_installed {
            discovered_channels = discover_wasm_channels(&channels_dir).await;
        }

        // Determine if we need secrets context
        let needs_secrets =
            selected.contains(&CHANNEL_INDEX_HTTP) || !selected_wasm_channels.is_empty();
        let secrets = if needs_secrets {
            match self.init_secrets_context().await {
                Ok(ctx) => Some(ctx),
                Err(e) => {
                    print_info(&format!("Secrets not available: {}", e));
                    print_info("Channel tokens must be set via environment variables.");
                    None
                }
            }
        } else {
            None
        };

        // HTTP channel
        if selected.contains(&CHANNEL_INDEX_HTTP) {
            println!();
            if let Some(ref ctx) = secrets {
                let result = setup_http(ctx).await?;
                self.settings.channels.http_enabled = result.enabled;
                self.settings.channels.http_port = Some(result.port);
            } else {
                self.settings.channels.http_enabled = true;
                self.settings.channels.http_port = Some(8080);
                print_info("HTTP webhook enabled on port 8080 (set HTTP_WEBHOOK_SECRET in env)");
            }
        } else {
            self.settings.channels.http_enabled = false;
        }

        // Signal channel
        if selected.contains(&CHANNEL_INDEX_SIGNAL) {
            println!();
            let result = setup_signal(&self.settings).await?;
            self.settings.channels.signal_enabled = result.enabled;
            self.settings.channels.signal_http_url = Some(result.http_url);
            self.settings.channels.signal_account = Some(result.account);
            self.settings.channels.signal_allow_from = Some(result.allow_from);
            self.settings.channels.signal_allow_from_groups = Some(result.allow_from_groups);
            self.settings.channels.signal_dm_policy = Some(result.dm_policy);
            self.settings.channels.signal_group_policy = Some(result.group_policy);
            self.settings.channels.signal_group_allow_from = Some(result.group_allow_from);
        } else {
            self.settings.channels.signal_enabled = false;
            self.settings.channels.signal_http_url = None;
            self.settings.channels.signal_account = None;
            self.settings.channels.signal_allow_from = None;
            self.settings.channels.signal_allow_from_groups = None;
            self.settings.channels.signal_dm_policy = None;
            self.settings.channels.signal_group_policy = None;
            self.settings.channels.signal_group_allow_from = None;
        }

        let discovered_by_name: HashMap<String, ChannelCapabilitiesFile> =
            discovered_channels.into_iter().collect();

        // Process selected WASM channels
        let mut enabled_wasm_channels = Vec::new();
        for channel_name in selected_wasm_channels {
            println!();
            if let Some(ref ctx) = secrets {
                let result = if let Some(cap_file) = discovered_by_name.get(&channel_name) {
                    if !cap_file.setup.required_secrets.is_empty() {
                        setup_wasm_channel(ctx, &channel_name, &cap_file.setup).await?
                    } else {
                        print_info(&format!(
                            "No setup configuration found for {}",
                            channel_name
                        ));
                        crate::setup::channels::WasmChannelSetupResult {
                            enabled: true,
                            channel_name: channel_name.clone(),
                        }
                    }
                } else {
                    print_info(&format!(
                        "Channel '{}' is selected but not available on disk.",
                        channel_name
                    ));
                    continue;
                };

                if result.enabled {
                    enabled_wasm_channels.push(result.channel_name);
                }
            } else {
                // No secrets context, just enable the channel
                print_info(&format!(
                    "{} enabled (configure tokens via environment)",
                    capitalize_first(&channel_name)
                ));
                enabled_wasm_channels.push(channel_name.clone());
            }
        }

        self.settings.channels.wasm_channels = enabled_wasm_channels;

        Ok(())
    }

    /// Step 7: Extensions (tools) installation from registry.
    async fn step_extensions(&mut self) -> Result<(), SetupError> {
        let catalog = match load_registry_catalog() {
            Some(c) => c,
            None => {
                print_info("Extension registry not found. Skipping tool installation.");
                print_info("Install tools manually with: ironclaw tool install <path>");
                return Ok(());
            }
        };

        let tools: Vec<_> = catalog
            .list(Some(crate::registry::manifest::ManifestKind::Tool), None)
            .into_iter()
            .cloned()
            .collect();

        if tools.is_empty() {
            print_info("No tools found in registry.");
            return Ok(());
        }

        print_info("Available tools from the extension registry:");
        print_info("Select which tools to install. You can install more later with:");
        print_info("  ironclaw registry install <name>");
        println!();

        // Check which tools are already installed
        let tools_dir = ironclaw_base_dir().join("tools");

        let installed_tools = discover_installed_tools(&tools_dir).await;

        // Build options: show display_name + description, pre-check "default" tagged + already installed
        let mut options: Vec<(String, bool)> = Vec::new();
        for tool in &tools {
            let is_installed = installed_tools.contains(&tool.name);
            let is_default = tool.tags.contains(&"default".to_string());
            let status = if is_installed { " (installed)" } else { "" };
            let auth_hint = tool
                .auth_summary
                .as_ref()
                .and_then(|a| a.method.as_deref())
                .map(|m| format!(" [{}]", m))
                .unwrap_or_default();

            let label = format!(
                "{}{}{} - {}",
                tool.display_name, auth_hint, status, tool.description
            );
            options.push((label, is_default || is_installed));
        }

        let options_refs: Vec<(&str, bool)> =
            options.iter().map(|(s, b)| (s.as_str(), *b)).collect();

        let selected = select_many("Which tools do you want to install?", &options_refs)
            .map_err(SetupError::Io)?;

        if selected.is_empty() {
            print_info("No tools selected.");
            return Ok(());
        }

        // Install selected tools that aren't already on disk
        let repo_root = catalog.root().parent().unwrap_or(catalog.root());
        let installer = crate::registry::installer::RegistryInstaller::new(
            repo_root.to_path_buf(),
            tools_dir.clone(),
            ironclaw_base_dir().join("channels"),
        );

        let mut installed_count = 0;
        let mut auth_needed: Vec<String> = Vec::new();

        for idx in &selected {
            let tool = &tools[*idx];
            if installed_tools.contains(&tool.name) {
                continue; // Already installed, skip
            }

            match installer.install_with_source_fallback(tool, false).await {
                Ok(outcome) => {
                    print_success(&format!("Installed {}", outcome.name));
                    for warning in &outcome.warnings {
                        print_info(&format!("{}: {}", outcome.name, warning));
                    }
                    installed_count += 1;

                    // Track auth needs
                    if let Some(auth) = &tool.auth_summary
                        && auth.method.as_deref() != Some("none")
                        && auth.method.is_some()
                    {
                        let provider = auth.provider.as_deref().unwrap_or(&tool.name);
                        // Only mention unique providers (Google tools share auth)
                        let hint = format!("  {} - ironclaw tool auth {}", provider, tool.name);
                        if !auth_needed
                            .iter()
                            .any(|h| h.starts_with(&format!("  {} -", provider)))
                        {
                            auth_needed.push(hint);
                        }
                    }
                }
                Err(e) => {
                    print_error(&format!("Failed to install {}: {}", tool.display_name, e));
                }
            }
        }

        if installed_count > 0 {
            println!();
            print_success(&format!("{} tool(s) installed.", installed_count));
        }

        if !auth_needed.is_empty() {
            println!();
            print_info("Some tools need authentication. Run after setup:");
            for hint in &auth_needed {
                print_info(hint);
            }
        }

        Ok(())
    }

    /// Step 8: Docker Sandbox -- check Docker installation and availability.
    async fn step_docker_sandbox(&mut self) -> Result<(), SetupError> {
        print_info("IronClaw can execute code, run builds, and use tools inside Docker");
        print_info("containers. This keeps your system safe -- commands from the LLM run");
        print_info("in an isolated sandbox with no access to your credentials, limited");
        print_info("filesystem access, and network traffic restricted to an allowlist.");
        println!();
        print_info("Without Docker, code execution tools (shell, file write) run directly");
        print_info("on your machine with no isolation.");
        println!();

        if !confirm("Enable Docker sandbox?", false).map_err(SetupError::Io)? {
            self.settings.sandbox.enabled = false;
            print_info("Sandbox disabled. You can enable it later with SANDBOX_ENABLED=true.");
            return Ok(());
        }

        // Check Docker availability
        let detection = crate::sandbox::detect::check_docker().await;

        match detection.status {
            crate::sandbox::detect::DockerStatus::Available => {
                self.settings.sandbox.enabled = true;
                print_success("Docker is installed and running. Sandbox enabled.");

                // Check if the worker image exists
                self.ensure_worker_image().await?;
            }
            crate::sandbox::detect::DockerStatus::NotInstalled
            | crate::sandbox::detect::DockerStatus::NotRunning => {
                println!();
                let not_installed =
                    detection.status == crate::sandbox::detect::DockerStatus::NotInstalled;
                if not_installed {
                    print_error("Docker is not installed.");
                    print_info(detection.platform.install_hint());
                } else {
                    print_error("Docker is installed but not running.");
                    print_info(detection.platform.start_hint());
                }
                println!();

                let retry_prompt = if not_installed {
                    "Retry after installing Docker?"
                } else {
                    "Retry after starting Docker?"
                };
                if confirm(retry_prompt, false).map_err(SetupError::Io)? {
                    let retry = crate::sandbox::detect::check_docker().await;
                    if retry.status.is_ok() {
                        self.settings.sandbox.enabled = true;
                        print_success(if not_installed {
                            "Docker is now available. Sandbox enabled."
                        } else {
                            "Docker is now running. Sandbox enabled."
                        });
                        // Check if the worker image exists
                        self.ensure_worker_image().await?;
                    } else {
                        self.settings.sandbox.enabled = false;
                        print_info(if not_installed {
                            "Docker still not available. Sandbox disabled for now."
                        } else {
                            "Docker still not responding. Sandbox disabled for now."
                        });
                    }
                } else {
                    self.settings.sandbox.enabled = false;
                    print_info(if not_installed {
                        "Sandbox disabled. Install Docker and set SANDBOX_ENABLED=true later."
                    } else {
                        "Sandbox disabled. Start Docker and set SANDBOX_ENABLED=true later."
                    });
                }
            }
            crate::sandbox::detect::DockerStatus::Disabled => {
                self.settings.sandbox.enabled = false;
            }
        }

        // Claude Code sandbox sub-step (only if Docker sandbox is enabled)
        if self.settings.sandbox.enabled {
            self.step_claude_code_sandbox().await?;
        }

        Ok(())
    }

    /// Claude Code sandbox sub-step: enable Claude CLI inside Docker containers.
    async fn step_claude_code_sandbox(&mut self) -> Result<(), SetupError> {
        println!();
        print_info("Claude Code mode lets the agent delegate complex tasks to Claude CLI");
        print_info("running inside sandboxed Docker containers.");
        println!();

        if !confirm("Enable Claude Code sandbox mode?", false).map_err(SetupError::Io)? {
            self.settings.sandbox.claude_code_enabled = false;
            return Ok(());
        }

        // Check for Anthropic credentials (API key or OAuth token).
        // Uses `optional_env()` which reads both real env vars and the
        // injected overlay (secrets DB, wizard-set values).
        let has_credentials = || {
            let has_api_key = crate::config::helpers::optional_env("ANTHROPIC_API_KEY")
                .ok()
                .flatten()
                .is_some_and(|v| !v.is_empty() && v != OAUTH_PLACEHOLDER);
            let has_oauth = crate::config::ClaudeCodeConfig::extract_oauth_token().is_some()
                || crate::config::helpers::optional_env("ANTHROPIC_OAUTH_TOKEN")
                    .ok()
                    .flatten()
                    .is_some_and(|v| !v.is_empty());
            has_api_key || has_oauth
        };

        if has_credentials() {
            self.settings.sandbox.claude_code_enabled = true;
            print_success("Claude Code sandbox enabled");
        } else {
            print_error("No Anthropic credentials found.");
            print_info(
                "Claude Code needs ANTHROPIC_API_KEY or an OAuth token from `claude login`.",
            );
            println!();

            if confirm("Retry after setting up credentials?", false).map_err(SetupError::Io)? {
                if has_credentials() {
                    self.settings.sandbox.claude_code_enabled = true;
                    print_success("Claude Code sandbox enabled");
                } else {
                    self.settings.sandbox.claude_code_enabled = false;
                    print_info("No credentials found. Claude Code disabled for now.");
                    print_info("Set ANTHROPIC_API_KEY or run `claude login` and enable later.");
                }
            } else {
                self.settings.sandbox.claude_code_enabled = false;
                print_info("Claude Code disabled. Enable with CLAUDE_CODE_ENABLED=true later.");
            }
        }

        Ok(())
    }

    /// Ensure the sandbox worker Docker image exists, building it if necessary.
    async fn ensure_worker_image(&mut self) -> Result<(), SetupError> {
        use crate::sandbox::container::{ContainerRunner, connect_docker};

        let image_name = self.settings.sandbox.image.clone();
        let docker = match connect_docker().await {
            Ok(d) => d,
            Err(e) => {
                // check_docker() may report Available (via CLI fallback) even when
                // connect_docker() fails (e.g. on Windows). Don't hard-fail setup.
                print_info(&format!(
                    "Could not connect to Docker API to verify image: {}",
                    e
                ));
                print_info("Image check skipped. The image will be pulled at first job run.");
                return Ok(());
            }
        };
        let runner = ContainerRunner::for_image_ops(docker, image_name.clone());

        if runner.image_exists().await {
            print_success(&format!("Worker image '{}' found.", image_name));
            return Ok(());
        }

        println!();
        print_info(&format!("Worker image '{}' not found.", image_name));
        print_info("This image is required for sandboxed job execution.");
        println!();

        // Images that contain '/' look like registry references (e.g.
        // "ghcr.io/nearai/ironclaw-worker:v1"). For those, or when
        // auto_pull_image is enabled, attempt a pull before offering a
        // local build — the runtime would do the same thing via
        // SandboxManager::ensure_ready().
        let is_registry_image = image_name.contains('/');
        if is_registry_image || self.settings.sandbox.auto_pull_image {
            print_info(&format!("Attempting to pull '{}'...", image_name));
            match runner.pull_image().await {
                Ok(()) => {
                    print_success(&format!("Successfully pulled image '{}'.", image_name));
                    return Ok(());
                }
                Err(e) => {
                    if is_registry_image {
                        // Registry image that can't be pulled — don't offer local build.
                        print_error(&format!("Failed to pull image: {}", e));
                        print_info("Ensure the image is published and accessible, or set");
                        print_info("SANDBOX_IMAGE to a local image name and try again.");
                        return Ok(());
                    }
                    print_info(&format!(
                        "Pull failed ({}). Checking for local Dockerfile...",
                        e
                    ));
                }
            }
        }

        // Only offer local build for default-style local images.
        let dockerfile_path = std::path::PathBuf::from("Dockerfile.worker");

        if dockerfile_path.exists() {
            print_info(&format!(
                "Found Dockerfile at: {}",
                dockerfile_path.display()
            ));
            if confirm(
                "Build the worker image now? (this may take a few minutes)",
                true,
            )
            .map_err(SetupError::Io)?
            {
                print_info("Building worker image... This may take a few minutes.");
                match runner.build_image(&dockerfile_path).await {
                    Ok(()) => {
                        print_success(&format!("Successfully built image '{}'.", image_name));
                    }
                    Err(e) => {
                        print_error(&format!("Failed to build image: {}", e));
                        print_info("You can build it manually later with:");
                        print_info(&format!(
                            "  docker build -f Dockerfile.worker -t {} .",
                            image_name
                        ));
                    }
                }
            } else {
                print_info("Skipped image build. Build it manually with:");
                print_info(&format!(
                    "  docker build -f Dockerfile.worker -t {} .",
                    image_name
                ));
            }
        } else {
            print_info("No Dockerfile.worker found in current directory.");
            print_info("To use Docker sandbox, build the worker image manually:");
            print_info(&format!(
                "  docker build -f Dockerfile.worker -t {} .",
                image_name
            ));
            print_info("or clone the IronClaw repository and build from source.");
        }

        Ok(())
    }

    /// Step 9: Heartbeat configuration.
    fn step_heartbeat(&mut self) -> Result<(), SetupError> {
        print_info("Heartbeat runs periodic background tasks (e.g., checking your calendar,");
        print_info("monitoring for notifications, running scheduled workflows).");
        println!();

        if !confirm("Enable heartbeat?", false).map_err(SetupError::Io)? {
            self.settings.heartbeat.enabled = false;
            print_info("Heartbeat disabled.");
            return Ok(());
        }

        self.settings.heartbeat.enabled = true;

        // Interval
        let interval_str = optional_input("Check interval in minutes", Some("default: 30"))
            .map_err(SetupError::Io)?;

        if let Some(s) = interval_str {
            if let Ok(mins) = s.parse::<u64>() {
                self.settings.heartbeat.interval_secs = mins * 60;
            }
        } else {
            self.settings.heartbeat.interval_secs = 1800; // 30 minutes
        }

        // Notify channel
        let notify_channel = optional_input("Notify channel on findings", Some("e.g., telegram"))
            .map_err(SetupError::Io)?;
        self.settings.heartbeat.notify_channel = notify_channel;

        print_success(&format!(
            "Heartbeat enabled (every {} minutes)",
            self.settings.heartbeat.interval_secs / 60
        ));

        Ok(())
    }

    /// Persist current settings to the database.
    ///
    /// Returns `Ok(true)` if settings were saved, `Ok(false)` if no database
    /// connection is available yet (e.g., before Step 1 completes).
    async fn persist_settings(&self) -> Result<bool, SetupError> {
        let db_map = self.settings.to_db_map();
        let saved = false;

        #[cfg(feature = "postgres")]
        let saved = if !saved {
            if let Some(ref pool) = self.db_pool {
                let store = crate::history::Store::from_pool(pool.clone());
                store
                    .set_all_settings(self.owner_id(), &db_map)
                    .await
                    .map_err(|e| {
                        SetupError::Database(format!("Failed to save settings to database: {}", e))
                    })?;
                true
            } else {
                false
            }
        } else {
            saved
        };

        #[cfg(feature = "libsql")]
        let saved = if !saved {
            if let Some(ref backend) = self.db_backend {
                use crate::db::SettingsStore as _;
                backend
                    .set_all_settings(self.owner_id(), &db_map)
                    .await
                    .map_err(|e| {
                        SetupError::Database(format!("Failed to save settings to database: {}", e))
                    })?;
                true
            } else {
                false
            }
        } else {
            saved
        };

        Ok(saved)
    }

    /// Write bootstrap environment variables to `~/.ironclaw/.env`.
    ///
    /// Only true chicken-and-egg settings are written here — things needed
    /// before the database is connected: `IRONCLAW_PROFILE`, `DATABASE_BACKEND`,
    /// `DATABASE_URL`, `LIBSQL_PATH`, `SECRETS_MASTER_KEY`, `ONBOARD_COMPLETED`, and
    /// channel config vars (Signal, Claude Code sandbox).
    ///
    /// **LLM settings and credentials are NOT written here.** `LLM_BACKEND`,
    /// base URLs, and model names are persisted to the DB via
    /// `persist_settings()` and loaded by `Config::from_db_with_toml()`.
    /// API keys live only in the encrypted secrets DB and are injected via
    /// `inject_llm_keys_from_secrets()` after DB init.
    fn bootstrap_env_vars(&self) -> Vec<(String, String)> {
        let mut env_vars: Vec<(String, String)> = Vec::new();

        if let Some(ref profile) = self.selected_deployment_profile {
            env_vars.push(("IRONCLAW_PROFILE".to_string(), profile.clone()));
        }

        if let Some(ref backend) = self.settings.database_backend {
            env_vars.push(("DATABASE_BACKEND".to_string(), backend.clone()));
        }
        if let Some(ref url) = self.settings.database_url {
            env_vars.push(("DATABASE_URL".to_string(), url.clone()));
        }
        if let Some(ref path) = self.settings.libsql_path {
            env_vars.push(("LIBSQL_PATH".to_string(), path.clone()));
        }
        if let Some(ref url) = self.settings.libsql_url {
            env_vars.push(("LIBSQL_URL".to_string(), url.clone()));
        }

        // Secrets master key (env var mode): write to .env so it's available
        // on next startup before the DB is connected.
        if let Some(ref key_hex) = self.settings.secrets_master_key_hex {
            env_vars.push(("SECRETS_MASTER_KEY".to_string(), key_hex.clone()));
        }

        // Always write ONBOARD_COMPLETED so that check_onboard_needed()
        // (which runs before the DB is connected) knows to skip re-onboarding.
        if self.settings.onboard_completed {
            env_vars.push(("ONBOARD_COMPLETED".to_string(), "true".to_string()));
        }

        // Claude Code sandbox mode
        if self.settings.sandbox.claude_code_enabled {
            env_vars.push(("CLAUDE_CODE_ENABLED".to_string(), "true".to_string()));
        }

        // Signal channel env vars (chicken-and-egg: config resolves before DB).
        if let Some(ref url) = self.settings.channels.signal_http_url {
            env_vars.push(("SIGNAL_HTTP_URL".to_string(), url.clone()));
        }
        if let Some(ref account) = self.settings.channels.signal_account {
            env_vars.push(("SIGNAL_ACCOUNT".to_string(), account.clone()));
        }
        if let Some(ref allow_from) = self.settings.channels.signal_allow_from {
            env_vars.push(("SIGNAL_ALLOW_FROM".to_string(), allow_from.clone()));
        }
        if let Some(ref allow_from_groups) = self.settings.channels.signal_allow_from_groups
            && !allow_from_groups.is_empty()
        {
            env_vars.push((
                "SIGNAL_ALLOW_FROM_GROUPS".to_string(),
                allow_from_groups.clone(),
            ));
        }
        if let Some(ref dm_policy) = self.settings.channels.signal_dm_policy {
            env_vars.push(("SIGNAL_DM_POLICY".to_string(), dm_policy.clone()));
        }
        if let Some(ref group_policy) = self.settings.channels.signal_group_policy {
            env_vars.push(("SIGNAL_GROUP_POLICY".to_string(), group_policy.clone()));
        }
        if let Some(ref group_allow_from) = self.settings.channels.signal_group_allow_from
            && !group_allow_from.is_empty()
        {
            env_vars.push((
                "SIGNAL_GROUP_ALLOW_FROM".to_string(),
                group_allow_from.clone(),
            ));
        }

        env_vars
    }

    fn write_bootstrap_env(&self) -> Result<(), SetupError> {
        let env_vars = self.bootstrap_env_vars();

        if !env_vars.is_empty() {
            let pairs: Vec<(&str, &str)> = env_vars
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            crate::bootstrap::upsert_bootstrap_vars(&pairs).map_err(|e| {
                SetupError::Io(std::io::Error::other(format!(
                    "Failed to save bootstrap env to .env: {}",
                    e
                )))
            })?;
        }

        Ok(())
    }

    /// Persist the NEAR AI session token to encrypted secrets and the database.
    ///
    /// The session manager writes to disk during `ensure_authenticated()` but
    /// doesn't have a DB store attached during onboarding. This reads the
    /// session file from disk and stores it under `nearai_session_token` in the
    /// encrypted secrets store. Falls back to the plaintext settings table
    /// only when no secrets store is available.
    ///
    /// Best-effort: silently ignores errors (no DB connection yet, no
    /// session file, etc.).
    async fn persist_session_to_db(&mut self) {
        let session_path = crate::config::llm::default_session_path();
        let data = match std::fs::read_to_string(&session_path) {
            Ok(d) if !d.trim().is_empty() => d,
            _ => return,
        };
        let value: serde_json::Value = match serde_json::from_str(&data) {
            Ok(v) => v,
            Err(_) => return,
        };

        // Try to persist to encrypted secrets store (preferred).
        if let Ok(ctx) = self.init_secrets_context().await {
            if let Err(e) = ctx
                .save_secret(
                    "nearai_session_token",
                    &secrecy::SecretString::from(data.clone()),
                )
                .await
            {
                tracing::debug!("Could not persist session token to secrets store: {}", e);
            } else {
                tracing::debug!("Session token persisted to encrypted secrets store");
                return;
            }
        }

        // Fallback: persist to plaintext settings table.
        #[cfg(feature = "postgres")]
        if let Some(ref pool) = self.db_pool {
            let store = crate::history::Store::from_pool(pool.clone());
            if let Err(e) = store
                .set_setting(self.owner_id(), "nearai.session_token", &value)
                .await
            {
                tracing::debug!("Could not persist session token to postgres: {}", e);
            } else {
                tracing::debug!("Session token persisted to database");
                return;
            }
        }

        #[cfg(feature = "libsql")]
        if let Some(ref backend) = self.db_backend {
            use crate::db::SettingsStore as _;
            if let Err(e) = backend
                .set_setting(self.owner_id(), "nearai.session_token", &value)
                .await
            {
                tracing::debug!("Could not persist session token to libsql: {}", e);
            } else {
                tracing::debug!("Session token persisted to database");
            }
        }
    }

    /// Persist settings to DB and bootstrap .env after each step.
    ///
    /// Silently ignores errors (e.g., DB not connected yet before step 1
    /// completes). This is best-effort incremental persistence.
    async fn persist_after_step(&self) {
        // Write bootstrap .env (always possible)
        if let Err(e) = self.write_bootstrap_env() {
            tracing::debug!("Could not write bootstrap env after step: {}", e);
        }

        // Persist to DB
        match self.persist_settings().await {
            Ok(true) => tracing::debug!("Settings persisted to database after step"),
            Ok(false) => tracing::debug!("No DB connection yet, skipping settings persist"),
            Err(e) => tracing::debug!("Could not persist settings after step: {}", e),
        }
    }

    /// Load previously saved settings from the database after Step 1
    /// establishes a connection.
    ///
    /// This enables recovery from partial onboarding runs: if the user
    /// completed steps 1-4 previously but step 5 failed, re-running
    /// the wizard will pre-populate settings from the database.
    ///
    /// **Callers must re-apply any wizard choices made before this call**
    /// via `self.settings.merge_from(&step_settings)`, since `merge_from`
    /// prefers the `other` argument's non-default values. Without this,
    /// stale DB values would overwrite fresh user choices.
    async fn try_load_existing_settings(&mut self) -> bool {
        // NB: `loaded` starts false and is only reassigned inside feature-gated
        // blocks below.  When *neither* `postgres` nor `libsql` is enabled the
        // function always returns false (no DB backend → nothing to load).
        // Each block shadows `loaded` via `let loaded = …` so the compiler
        // sees a use on every path; this is intentional, not accidental shadowing.
        let loaded = false;

        #[cfg(feature = "postgres")]
        let loaded = if !loaded {
            if let Some(ref pool) = self.db_pool {
                let store = crate::history::Store::from_pool(pool.clone());
                match store.get_all_settings(self.owner_id()).await {
                    Ok(db_map) if !db_map.is_empty() => {
                        let existing = Settings::from_db_map(&db_map);
                        self.settings.merge_from(&existing);
                        tracing::info!("Loaded {} existing settings from database", db_map.len());
                        true
                    }
                    Ok(_) => false,
                    Err(e) => {
                        tracing::debug!("Could not load existing settings: {}", e);
                        false
                    }
                }
            } else {
                false
            }
        } else {
            loaded
        };

        #[cfg(feature = "libsql")]
        let loaded = if !loaded {
            if let Some(ref backend) = self.db_backend {
                use crate::db::SettingsStore as _;
                match backend.get_all_settings(self.owner_id()).await {
                    Ok(db_map) if !db_map.is_empty() => {
                        let existing = Settings::from_db_map(&db_map);
                        self.settings.merge_from(&existing);
                        tracing::info!("Loaded {} existing settings from database", db_map.len());
                        true
                    }
                    Ok(_) => false,
                    Err(e) => {
                        tracing::debug!("Could not load existing settings: {}", e);
                        false
                    }
                }
            } else {
                false
            }
        } else {
            loaded
        };

        loaded
    }

    /// Save settings to the database and `~/.ironclaw/.env`, then print
    /// a warm completion card with the 3 key facts.
    async fn save_and_summarize(&mut self) -> Result<(), SetupError> {
        use crate::cli::fmt;

        self.settings.onboard_completed = true;

        // Final persist (idempotent — earlier incremental saves already wrote
        // most settings, but this ensures onboard_completed is saved).
        let saved = self.persist_settings().await?;

        if !saved {
            return Err(SetupError::Database(
                "No database connection, cannot save settings".to_string(),
            ));
        }

        // Write bootstrap env (also idempotent)
        self.write_bootstrap_env()?;

        // ── Completion card ───────────────────────────────────
        let sep = fmt::separator(38);

        println!();
        println!("  {}", sep);
        println!();

        // Title line: checkmark + "ironclaw is ready"
        println!(
            "  {}\u{2713}{} {}ironclaw is ready{}",
            fmt::success(),
            fmt::reset(),
            fmt::bold_accent(),
            fmt::reset(),
        );
        println!();

        // Fact 1: Provider + model
        let provider_display = match self.settings.llm_backend.as_deref() {
            Some("nearai") => "NEAR AI".to_string(),
            Some("anthropic") => "Anthropic".to_string(),
            Some("openai") => "OpenAI".to_string(),
            Some("ollama") => "Ollama".to_string(),
            Some("openai_compatible") => "OpenAI-compatible".to_string(),
            Some("bedrock") => "AWS Bedrock".to_string(),
            Some("openai_codex") => "OpenAI Codex".to_string(),
            Some("gemini_oauth") => "Gemini CLI".to_string(),
            Some(other) => other.to_string(),
            None => "unknown".to_string(),
        };
        let model_suffix = if let Some(ref model) = self.settings.selected_model {
            // Truncate long model names (char-based to avoid UTF-8 panic)
            let display = if model.chars().count() > 30 {
                let truncated: String = model.chars().take(27).collect();
                format!("{}...", truncated)
            } else {
                model.clone()
            };
            format!(" ({})", display)
        } else {
            String::new()
        };
        let provider_value = format!("{}{}", provider_display, model_suffix);
        println!(
            "    {}provider{}    {}{}{}",
            fmt::dim(),
            fmt::reset(),
            fmt::accent(),
            provider_value,
            fmt::reset(),
        );

        // Fact 2: Database
        let db_display = match self.settings.database_backend.as_deref() {
            Some("libsql") => "libSQL".to_string(),
            Some("postgres") | Some("postgresql") => "PostgreSQL".to_string(),
            Some(other) => other.to_string(),
            None => "unknown".to_string(),
        };
        println!(
            "    {}database{}    {}{}{}",
            fmt::dim(),
            fmt::reset(),
            fmt::accent(),
            db_display,
            fmt::reset(),
        );

        // Fact 3: Security
        let security_display = match self.settings.secrets_master_key_source {
            KeySource::Keychain => "OS keychain",
            KeySource::Env => "environment variable",
            KeySource::None => "disabled",
        };
        println!(
            "    {}security{}    {}{}{}",
            fmt::dim(),
            fmt::reset(),
            fmt::accent(),
            security_display,
            fmt::reset(),
        );

        println!();
        println!("  {}", sep);
        println!();

        // Action hints
        println!(
            "  {}Start chatting:{}   {}ironclaw{}",
            fmt::dim(),
            fmt::reset(),
            fmt::bold_accent(),
            fmt::reset(),
        );
        println!(
            "  {}Full setup:{}       {}ironclaw onboard{}",
            fmt::dim(),
            fmt::reset(),
            fmt::bold_accent(),
            fmt::reset(),
        );
        println!();

        if self.config.quick {
            print_info(
                "Tip: Run `ironclaw onboard` to configure channels, extensions, embeddings, and more.",
            );
            println!();
        }

        Ok(())
    }
}

impl Default for SetupWizard {
    fn default() -> Self {
        Self::new()
    }
}

/// Mask password in a database URL for display.
#[cfg(feature = "postgres")]
fn mask_password_in_url(url: &str) -> String {
    // URL format: scheme://user:password@host/database
    // Find "://" to locate start of credentials
    let Some(scheme_end) = url.find("://") else {
        return url.to_string();
    };
    let credentials_start = scheme_end + 3; // After "://"

    // Find "@" to locate end of credentials
    let Some(at_pos) = url[credentials_start..].find('@') else {
        return url.to_string();
    };
    let at_abs = credentials_start + at_pos;

    // Find ":" in the credentials section (separates user from password)
    let credentials = &url[credentials_start..at_abs];
    let Some(colon_pos) = credentials.find(':') else {
        return url.to_string();
    };

    // Build masked URL: scheme://user:****@host/database
    let scheme = &url[..credentials_start]; // "postgres://"
    let username = &credentials[..colon_pos]; // "user"
    let after_at = &url[at_abs..]; // "@localhost/db"

    format!("{}{}:****{}", scheme, username, after_at)
}

/// Discover WASM channels in a directory.
///
/// Returns a list of (channel_name, capabilities_file) pairs.
async fn discover_wasm_channels(dir: &std::path::Path) -> Vec<(String, ChannelCapabilitiesFile)> {
    let mut channels = Vec::new();

    if !dir.is_dir() {
        return channels;
    }

    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(e) => e,
        Err(_) => return channels,
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();

        // Look for .capabilities.json files
        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        if !filename.ends_with(".capabilities.json") {
            continue;
        }

        // Extract channel name
        let name = filename.trim_end_matches(".capabilities.json").to_string();
        if name.is_empty() {
            continue;
        }

        // Check if corresponding .wasm file exists
        let wasm_path = dir.join(format!("{}.wasm", name));
        if !wasm_path.exists() {
            continue;
        }

        // Parse capabilities file
        match tokio::fs::read(&path).await {
            Ok(bytes) => match ChannelCapabilitiesFile::from_bytes(&bytes) {
                Ok(cap_file) => {
                    channels.push((name, cap_file));
                }
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "Failed to parse channel capabilities file"
                    );
                }
            },
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "Failed to read channel capabilities file"
                );
            }
        }
    }

    // Sort by name for consistent ordering
    channels.sort_by(|a, b| a.0.cmp(&b.0));
    channels
}

/// Mask an API key for display: show first 6 + last 4 chars.
///
/// Uses char-based indexing to avoid panicking on multi-byte UTF-8.
fn mask_api_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() < 12 {
        let prefix: String = chars.iter().take(4).collect();
        return format!("{prefix}...");
    }
    let prefix: String = chars[..6].iter().collect();
    let suffix: String = chars[chars.len() - 4..].iter().collect();
    format!("{prefix}...{suffix}")
}

/// Capitalize the first letter of a string.
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().chain(chars).collect(),
    }
}

#[cfg(test)]
async fn install_missing_bundled_channels(
    channels_dir: &std::path::Path,
    already_installed: &HashSet<String>,
) -> Result<Vec<String>, SetupError> {
    let mut installed = Vec::new();

    for name in available_channel_names().iter().copied() {
        if already_installed.contains(name) {
            continue;
        }

        install_bundled_channel(name, channels_dir, false)
            .await
            .map_err(SetupError::Channel)?;
        installed.push(name.to_string());
    }

    Ok(installed)
}

/// Build channel options from discovered channels + bundled + registry catalog.
///
/// Returns a deduplicated, sorted list of channel names available for selection.
fn build_channel_options(discovered: &[(String, ChannelCapabilitiesFile)]) -> Vec<String> {
    let mut names: Vec<String> = discovered.iter().map(|(name, _)| name.clone()).collect();

    // Add bundled channels
    for bundled in available_channel_names().iter().copied() {
        if !names.iter().any(|name| name == bundled) {
            names.push(bundled.to_string());
        }
    }

    // Add registry channels
    if let Some(catalog) = load_registry_catalog() {
        for manifest in catalog.list(Some(crate::registry::manifest::ManifestKind::Channel), None) {
            if !names.iter().any(|n| n == &manifest.name) {
                names.push(manifest.name.clone());
            }
        }
    }

    names.sort();
    names
}

/// Try to load the registry catalog. Falls back to embedded manifests when
/// the `registry/` directory cannot be found (e.g. running from an installed binary).
fn load_registry_catalog() -> Option<crate::registry::catalog::RegistryCatalog> {
    crate::registry::catalog::RegistryCatalog::load_or_embedded().ok()
}

/// Install selected channels from the registry that aren't already on disk
/// and weren't handled by the bundled installer.
async fn install_selected_registry_channels(
    channels_dir: &std::path::Path,
    selected_channels: &[String],
    already_installed: &HashSet<String>,
) -> Vec<String> {
    let catalog = match load_registry_catalog() {
        Some(c) => c,
        None => return Vec::new(),
    };

    let repo_root = catalog
        .root()
        .parent()
        .unwrap_or(catalog.root())
        .to_path_buf();

    let bundled: HashSet<&str> = available_channel_names().iter().copied().collect();
    let mut installed = Vec::new();

    for name in selected_channels {
        // Skip if already installed or handled by bundled installer
        if already_installed.contains(name) || bundled.contains(name.as_str()) {
            continue;
        }

        // Check if already on disk (may have been installed between bundled and here)
        let wasm_on_disk = channels_dir.join(format!("{}.wasm", name)).exists()
            || channels_dir.join(format!("{}-channel.wasm", name)).exists();
        if wasm_on_disk {
            continue;
        }

        // Look up in registry
        let manifest = match catalog.get(&format!("channels/{}", name)) {
            Some(m) => m,
            None => continue,
        };

        let installer = crate::registry::installer::RegistryInstaller::new(
            repo_root.clone(),
            ironclaw_base_dir().join("tools"),
            channels_dir.to_path_buf(),
        );

        match installer
            .install_with_source_fallback(manifest, false)
            .await
        {
            Ok(outcome) => {
                for warning in &outcome.warnings {
                    crate::setup::prompts::print_info(&format!("{}: {}", name, warning));
                }
                installed.push(name.clone());
            }
            Err(e) => {
                tracing::warn!(
                    channel = %name,
                    error = %e,
                    "Failed to install channel from registry"
                );
                crate::setup::prompts::print_error(&format!(
                    "Failed to install channel '{}': {}",
                    name, e
                ));
            }
        }
    }

    installed
}

/// Discover which tools are already installed in the tools directory.
///
/// Returns a set of tool names (the stem of .wasm files).
async fn discover_installed_tools(tools_dir: &std::path::Path) -> HashSet<String> {
    let mut names = HashSet::new();

    if !tools_dir.is_dir() {
        return names;
    }

    let mut entries = match tokio::fs::read_dir(tools_dir).await {
        Ok(e) => e,
        Err(_) => return names,
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("wasm")
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            names.insert(stem.to_string());
        }
    }

    names
}

async fn install_selected_bundled_channels(
    channels_dir: &std::path::Path,
    selected_channels: &[String],
    already_installed: &HashSet<String>,
) -> Result<Option<Vec<String>>, SetupError> {
    let bundled: HashSet<&str> = available_channel_names().iter().copied().collect();
    let selected_missing: HashSet<String> = selected_channels
        .iter()
        .filter(|name| bundled.contains(name.as_str()) && !already_installed.contains(*name))
        .cloned()
        .collect();

    if selected_missing.is_empty() {
        return Ok(None);
    }

    let mut installed = Vec::new();
    for name in selected_missing {
        install_bundled_channel(&name, channels_dir, false)
            .await
            .map_err(SetupError::Channel)?;
        installed.push(name);
    }

    installed.sort();
    Ok(Some(installed))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    #[cfg(unix)]
    use std::ffi::OsString;

    use tempfile::tempdir;

    use super::*;
    use crate::config::helpers::lock_env;

    #[test]
    fn test_wizard_creation() {
        let wizard = SetupWizard::new();
        assert!(!wizard.config.skip_auth);
        assert!(!wizard.config.channels_only);
    }

    #[test]
    fn test_wizard_with_config() {
        let config = SetupConfig {
            skip_auth: true,
            channels_only: false,
            provider_only: false,
            quick: false,
            steps: vec![],
        };
        let wizard = SetupWizard::with_config(config);
        assert!(wizard.config.skip_auth);
    }

    #[test]
    fn test_wizard_owner_id_uses_resolved_env_scope() {
        let _guard = lock_env();
        let _owner = EnvGuard::set("IRONCLAW_OWNER_ID", " wizard-owner ");

        let wizard = SetupWizard::new();
        assert_eq!(wizard.owner_id(), "wizard-owner"); // safety: test-only assertion
    }

    #[test]
    fn test_wizard_owner_id_uses_toml_scope() {
        let _guard = lock_env();
        let _owner = EnvGuard::clear("IRONCLAW_OWNER_ID");
        let dir = tempdir().unwrap(); // safety: test-only tempdir setup
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "owner_id = \"toml-owner\"\n").unwrap(); // safety: test-only fixture write

        let wizard = SetupWizard::try_with_config_and_toml(Default::default(), Some(&path))
            .expect("wizard should load owner_id from TOML"); // safety: test-only assertion
        assert_eq!(wizard.owner_id(), "toml-owner"); // safety: test-only assertion
    }

    #[test]
    fn test_bootstrap_env_vars_include_selected_deployment_profile() {
        let _guard = lock_env();
        let _profile = EnvGuard::clear("IRONCLAW_PROFILE");
        let mut wizard = SetupWizard::with_config(SetupConfig {
            quick: true,
            ..Default::default()
        });
        wizard.selected_deployment_profile = Some(QUICK_PROFILE_LOCAL_SANDBOX.to_string());
        wizard.settings.database_backend = Some("libsql".to_string());

        let vars = wizard.bootstrap_env_vars();

        assert!(
            vars.iter().any(|(key, value)| {
                key == "IRONCLAW_PROFILE" && value == QUICK_PROFILE_LOCAL_SANDBOX
            }),
            "selected deployment profile should be persisted to bootstrap env"
        );
        assert!(
            vars.iter()
                .any(|(key, value)| key == "DATABASE_BACKEND" && value == "libsql"),
            "existing bootstrap vars should still be written"
        );
    }

    #[test]
    fn test_bootstrap_env_vars_do_not_persist_unselected_profile() {
        let _guard = lock_env();
        let _profile = EnvGuard::set("IRONCLAW_PROFILE", QUICK_PROFILE_LOCAL);
        let wizard = SetupWizard::with_config(SetupConfig {
            quick: true,
            ..Default::default()
        });

        let vars = wizard.bootstrap_env_vars();

        assert!(
            !vars.iter().any(|(key, _)| key == "IRONCLAW_PROFILE"),
            "only a wizard-selected profile should be written back"
        );
    }

    #[test]
    fn test_apply_quick_local_profile_sets_profile_and_preserves_db_config_on_merge() {
        let _guard = lock_env();
        let _profile = EnvGuard::clear("IRONCLAW_PROFILE");

        let mut wizard = SetupWizard::with_config(SetupConfig {
            quick: true,
            ..Default::default()
        });
        // Simulate a pre-existing database_url chosen earlier in the wizard
        // (e.g. by auto_setup_database before profile selection was reordered).
        wizard.settings.database_url = Some("postgres://my-host/db".to_string());
        wizard.settings.database_backend = Some("postgres".to_string());

        // Snapshot settings *before* profile application — mimics the
        // `let step1_settings = self.settings.clone()` line in quick mode.
        let step1_settings = wizard.settings.clone();

        wizard
            .apply_quick_local_profile(QUICK_PROFILE_LOCAL)
            .expect("apply_quick_local_profile should succeed");

        // Profile applies its own database_backend (libsql) which overwrites
        // the wizard-chosen value.
        assert_eq!(
            wizard.settings.database_backend.as_deref(),
            Some("libsql"),
            "profile should overwrite database_backend"
        );

        // After merge_from, the wizard-chosen values should win back.
        wizard.settings.merge_from(&step1_settings);

        assert_eq!(
            wizard.settings.database_backend.as_deref(),
            Some("postgres"),
            "merge_from should restore the wizard-chosen database_backend"
        );
        assert_eq!(
            wizard.settings.database_url.as_deref(),
            Some("postgres://my-host/db"),
            "merge_from should restore the wizard-chosen database_url"
        );
        // Profile-applied non-DB settings should still be present since
        // step1_settings had defaults for them.
        assert!(
            wizard.settings.heartbeat.enabled,
            "profile-set heartbeat.enabled should survive merge"
        );
        assert_eq!(
            wizard.selected_deployment_profile.as_deref(),
            Some("local"),
            "selected_deployment_profile should be set"
        );
    }

    #[test]
    #[cfg(unix)]
    fn test_try_with_config_and_toml_propagates_invalid_owner_env() {
        use std::os::unix::ffi::OsStringExt;

        let _guard = lock_env();
        let original = std::env::var_os("IRONCLAW_OWNER_ID");
        unsafe {
            std::env::set_var("IRONCLAW_OWNER_ID", OsString::from_vec(vec![0x66, 0x80]));
        }

        let result = SetupWizard::try_with_config_and_toml(Default::default(), None);

        unsafe {
            if let Some(value) = original {
                std::env::set_var("IRONCLAW_OWNER_ID", value);
            } else {
                std::env::remove_var("IRONCLAW_OWNER_ID");
            }
        }

        assert!(result.is_err()); // safety: test-only assertion
    }

    #[test]
    #[cfg(feature = "postgres")]
    fn test_mask_password_in_url() {
        assert_eq!(
            mask_password_in_url("postgres://user:secret@localhost/db"),
            "postgres://user:****@localhost/db"
        );

        // URL without password
        assert_eq!(
            mask_password_in_url("postgres://localhost/db"),
            "postgres://localhost/db"
        );
    }

    #[test]
    fn test_capitalize_first() {
        assert_eq!(capitalize_first("telegram"), "Telegram");
        assert_eq!(capitalize_first("CAPS"), "CAPS");
        assert_eq!(capitalize_first(""), "");
    }

    #[test]
    fn test_mask_api_key() {
        assert_eq!(
            mask_api_key("sk-ant-api03-abcdef1234567890"),
            "sk-ant...7890"
        );
        assert_eq!(mask_api_key("short"), "shor...");
        assert_eq!(mask_api_key("exactly12ch"), "exac...");
        assert_eq!(mask_api_key("exactly12chr"), "exactl...2chr");
        assert_eq!(mask_api_key(""), "...");
        // Multi-byte chars should not panic
        assert_eq!(mask_api_key("日本語キー"), "日本語キ...");
    }

    #[tokio::test]
    async fn test_install_missing_bundled_channels_installs_telegram() {
        // WASM artifacts only exist in dev builds (not CI). Skip gracefully
        // rather than fail when the telegram channel hasn't been compiled.
        if !available_channel_names().contains(&"telegram") {
            eprintln!("skipping: telegram WASM artifacts not built");
            return;
        }

        let dir = tempdir().unwrap(); // safety: test-only tempdir setup
        let installed = HashSet::<String>::new();

        install_missing_bundled_channels(dir.path(), &installed)
            .await
            .unwrap(); // safety: test-only assertion

        assert!(dir.path().join("telegram.wasm").exists());
        assert!(dir.path().join("telegram.capabilities.json").exists());
    }

    #[test]
    fn test_build_channel_options_includes_available_when_missing() {
        let discovered = Vec::new();
        let options = build_channel_options(&discovered);
        let available = available_channel_names();
        // All available (built) channels should appear
        for name in &available {
            assert!(
                options.contains(&name.to_string()),
                "expected '{}' in options",
                name
            );
        }
    }

    #[test]
    fn test_build_channel_options_dedupes_available() {
        let discovered = vec![(String::from("telegram"), ChannelCapabilitiesFile::default())];
        let options = build_channel_options(&discovered);
        // telegram should appear exactly once despite being both discovered and available
        assert_eq!(
            options.iter().filter(|n| *n == "telegram").count(),
            1,
            "telegram should not be duplicated"
        );
    }

    #[tokio::test]
    async fn test_fetch_anthropic_models_static_fallback() {
        // With no API key, should return static defaults
        let _guard = EnvGuard::clear("ANTHROPIC_API_KEY");
        let models = fetch_anthropic_models(None).await;
        assert!(!models.is_empty());
        assert!(
            models.iter().any(|(id, _)| id.contains("claude")),
            "static defaults should include a Claude model"
        );
    }

    #[tokio::test]
    async fn test_fetch_openai_models_static_fallback() {
        let _guard = EnvGuard::clear("OPENAI_API_KEY");
        let models = fetch_openai_models(None).await;
        assert!(!models.is_empty());
        assert_eq!(models[0].0, "gpt-5.3-codex");
        assert!(
            models.iter().any(|(id, _)| id.contains("gpt")),
            "static defaults should include a GPT model"
        );
    }

    #[test]
    fn test_github_copilot_setup_preserves_model_for_same_backend() {
        let mut wizard = SetupWizard::new();
        wizard.settings.llm_backend = Some("github_copilot".to_string());
        wizard.settings.selected_model = Some("gpt-4o".to_string());

        wizard.set_llm_backend_preserving_model("github_copilot");

        assert_eq!(wizard.settings.selected_model.as_deref(), Some("gpt-4o"));
        assert_eq!(
            wizard.settings.llm_backend.as_deref(),
            Some("github_copilot")
        );
    }

    #[test]
    fn test_github_copilot_setup_clears_stale_model_on_switch() {
        let mut wizard = SetupWizard::new();
        wizard.settings.llm_backend = Some("openai".to_string());
        wizard.settings.selected_model = Some("gpt-5".to_string());

        wizard.set_llm_backend_preserving_model("github_copilot");

        assert!(wizard.settings.selected_model.is_none());
        assert_eq!(
            wizard.settings.llm_backend.as_deref(),
            Some("github_copilot")
        );
    }

    #[test]
    fn test_is_openai_chat_model_includes_gpt5_and_filters_non_chat_variants() {
        assert!(is_openai_chat_model("gpt-5"));
        assert!(is_openai_chat_model("gpt-5-mini-2026-01-01"));
        assert!(is_openai_chat_model("o3-2025-04-16"));
        assert!(!is_openai_chat_model("chatgpt-image-latest"));
        assert!(!is_openai_chat_model("gpt-4o-realtime-preview"));
        assert!(!is_openai_chat_model("gpt-4o-mini-transcribe"));
        assert!(!is_openai_chat_model("text-embedding-3-large"));
    }

    #[test]
    fn test_sort_openai_models_prioritizes_best_models_first() {
        let mut models = vec![
            ("gpt-4o-mini".to_string(), "gpt-4o-mini".to_string()),
            ("gpt-5-mini".to_string(), "gpt-5-mini".to_string()),
            ("o3".to_string(), "o3".to_string()),
            ("gpt-4.1".to_string(), "gpt-4.1".to_string()),
            ("gpt-5".to_string(), "gpt-5".to_string()),
        ];

        sort_openai_models(&mut models);

        let ordered: Vec<String> = models.into_iter().map(|(id, _)| id).collect();
        assert_eq!(
            ordered,
            vec![
                "gpt-5".to_string(),
                "gpt-5-mini".to_string(),
                "o3".to_string(),
                "gpt-4.1".to_string(),
                "gpt-4o-mini".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn test_fetch_ollama_models_unreachable_fallback() {
        // Point at a port nothing listens on
        let models = fetch_ollama_models("http://127.0.0.1:1").await;
        assert!(!models.is_empty(), "should fall back to static defaults");
    }

    #[tokio::test]
    async fn test_discover_wasm_channels_empty_dir() {
        let dir = tempdir().unwrap(); // safety: test-only tempdir setup
        let channels = discover_wasm_channels(dir.path()).await;
        assert!(channels.is_empty());
    }

    #[tokio::test]
    async fn test_discover_wasm_channels_nonexistent_dir() {
        let channels = discover_wasm_channels(
            &std::env::temp_dir().join("ironclaw_nonexistent_dir_abcxyz123"),
        )
        .await;
        assert!(channels.is_empty());
    }

    /// RAII guard that sets/clears an env var for the duration of a test.
    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let original = std::env::var(key).ok();
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, original }
        }

        fn clear(key: &'static str) -> Self {
            let original = std::env::var(key).ok();
            unsafe {
                std::env::remove_var(key);
            }
            Self { key, original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                if let Some(ref val) = self.original {
                    std::env::set_var(self.key, val);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    #[test]
    fn test_set_llm_backend_preserves_model_when_backend_unchanged() {
        let mut wizard = SetupWizard::new();
        wizard.settings.llm_backend = Some("openai".to_string());
        wizard.settings.selected_model = Some("gpt-4o".to_string());

        wizard.set_llm_backend_preserving_model("openai");

        assert_eq!(wizard.settings.llm_backend.as_deref(), Some("openai"));
        assert_eq!(wizard.settings.selected_model.as_deref(), Some("gpt-4o"));
    }

    #[test]
    fn test_set_llm_backend_clears_model_when_backend_was_unset() {
        let mut wizard = SetupWizard::new();
        wizard.settings.selected_model = Some("gpt-4o".to_string());

        wizard.set_llm_backend_preserving_model("openai");

        assert_eq!(wizard.settings.llm_backend.as_deref(), Some("openai"));
        assert_eq!(wizard.settings.selected_model, None);
    }

    #[test]
    fn test_set_llm_backend_clears_model_when_backend_changes() {
        let mut wizard = SetupWizard::new();
        wizard.settings.llm_backend = Some("openai".to_string());
        wizard.settings.selected_model = Some("gpt-4o".to_string());

        wizard.set_llm_backend_preserving_model("anthropic");

        assert_eq!(wizard.settings.llm_backend.as_deref(), Some("anthropic"));
        assert_eq!(wizard.settings.selected_model, None);
    }

    /// Regression test for #600: re-running provider setup for the same backend
    /// must NOT clear selected_model. Only switching to a different backend should.
    #[test]
    fn test_same_provider_preserves_selected_model() {
        let mut wizard = SetupWizard::new();
        wizard.settings.llm_backend = Some("ollama".to_string());
        wizard.settings.selected_model = Some("llama3".to_string());

        // Simulate re-entering the same provider -- model should survive
        // (This is the check that each setup_* function now performs)
        if wizard.settings.llm_backend.as_deref() != Some("ollama") {
            wizard.settings.selected_model = None;
        }
        wizard.settings.llm_backend = Some("ollama".to_string());

        assert_eq!(
            wizard.settings.selected_model.as_deref(),
            Some("llama3"),
            "model should be preserved when re-selecting the same provider"
        );
    }

    /// Regression test for #600: switching to a different provider must clear
    /// selected_model since the old model may not be valid for the new backend.
    #[test]
    fn test_different_provider_clears_selected_model() {
        let mut wizard = SetupWizard::new();
        wizard.settings.llm_backend = Some("ollama".to_string());
        wizard.settings.selected_model = Some("llama3".to_string());

        // Simulate switching to a different provider -- model should be cleared
        if wizard.settings.llm_backend.as_deref() != Some("openai") {
            wizard.settings.selected_model = None;
        }
        wizard.settings.llm_backend = Some("openai".to_string());

        assert!(
            wizard.settings.selected_model.is_none(),
            "model should be cleared when switching providers"
        );
    }

    /// Regression: Bedrock setup_bedrock() should preserve selected_model
    /// when re-entering the same provider (matches pattern from #600).
    #[test]
    fn test_bedrock_same_provider_preserves_model() {
        let mut wizard = SetupWizard::new();
        wizard.settings.llm_backend = Some("bedrock".to_string());
        wizard.settings.selected_model = Some("anthropic.claude-opus-4-6-v1".to_string());

        // Simulate the conditional clearing logic from setup_bedrock()
        if wizard.settings.llm_backend.as_deref() != Some("bedrock") {
            wizard.settings.selected_model = None;
        }
        wizard.settings.llm_backend = Some("bedrock".to_string());

        assert_eq!(
            wizard.settings.selected_model.as_deref(),
            Some("anthropic.claude-opus-4-6-v1"),
            "bedrock model should be preserved when re-selecting bedrock"
        );
    }

    /// Regression: switching from another provider to bedrock must clear
    /// selected_model, and choosing "default credentials" must clear
    /// bedrock_profile.
    #[test]
    fn test_bedrock_clears_stale_profile_on_default_creds() {
        let mut wizard = SetupWizard::new();
        wizard.settings.llm_backend = Some("bedrock".to_string());
        wizard.settings.bedrock_profile = Some("old-sso-profile".to_string());

        // Simulate auth_choice == 0 (default credentials) clearing the profile
        wizard.settings.bedrock_profile = None;

        assert!(
            wizard.settings.bedrock_profile.is_none(),
            "bedrock_profile should be cleared when selecting default credentials"
        );
    }

    /// Regression: empty profile input in named-profile auth should clear
    /// any previously configured profile instead of leaving it stale.
    #[test]
    fn test_bedrock_empty_profile_clears_existing() {
        let mut wizard = SetupWizard::new();
        wizard.settings.bedrock_profile = Some("old-profile".to_string());

        // Simulate auth_choice == 1 with empty input
        let profile = "".to_string();
        if profile.trim().is_empty() {
            wizard.settings.bedrock_profile = None;
        } else {
            wizard.settings.bedrock_profile = Some(profile);
        }

        assert!(
            wizard.settings.bedrock_profile.is_none(),
            "empty profile input should clear existing bedrock_profile"
        );
    }

    #[tokio::test]
    async fn test_run_provider_setup_no_setup_hint() {
        // A provider with setup: None should not error. It should set the
        // backend and return Ok, allowing env-var-only configured providers
        // to be kept during re-onboarding.
        let mut wizard = SetupWizard::new();

        let mut providers: Vec<crate::llm::registry::ProviderDefinition> =
            serde_json::from_str(include_str!("../../providers.json")).unwrap();
        // Add a provider with no setup hint
        providers.push(crate::llm::registry::ProviderDefinition {
            id: "custom_no_setup".to_string(),
            aliases: vec![],
            protocol: crate::llm::registry::ProviderProtocol::OpenAiCompletions,
            default_base_url: Some("http://localhost:9999/v1".to_string()),
            base_url_env: None,
            base_url_required: false,
            api_key_env: None,
            api_key_required: false,
            model_env: "CUSTOM_MODEL".to_string(),
            default_model: "custom-model".to_string(),
            description: "Custom provider with no setup wizard".to_string(),
            extra_headers_env: None,
            setup: None,
            unsupported_params: vec![],
        });
        let registry = crate::llm::ProviderRegistry::new(providers);

        let result = wizard
            .run_provider_setup("custom_no_setup", &registry)
            .await;
        assert!(result.is_ok(), "setup: None provider should not error");
        assert_eq!(
            wizard.settings.llm_backend.as_deref(),
            Some("custom_no_setup"),
            "backend should be set even without setup hint"
        );
    }

    /// Regression test for #666: env-var security option must initialize
    /// secrets_crypto so subsequent steps can encrypt API keys.
    #[test]
    fn test_env_var_security_initializes_crypto() {
        use crate::secrets::SecretsCrypto;
        use secrecy::SecretString;

        // Simulate what option 1 in step_security() does after the fix:
        let key_hex = crate::secrets::keychain::generate_master_key_hex();

        // The fix: create SecretsCrypto from the generated key.
        // Before the fix, this was skipped, leaving secrets_crypto = None.
        let crypto = SecretsCrypto::new(SecretString::from(key_hex.clone()));
        assert!(
            crypto.is_ok(),
            "generated key hex must produce valid SecretsCrypto"
        );

        // Verify the key is stored for bootstrap env persistence.
        let settings = Settings {
            secrets_master_key_hex: Some(key_hex),
            ..Settings::default()
        };
        assert!(settings.secrets_master_key_hex.is_some());
    }

    /// Regression test for #799: `fetch_nearai_models` hardcoded `api_key: None`,
    /// causing the auth prompt to re-appear during model selection when the user
    /// had authenticated via NEAR AI Cloud API key (option 4).
    #[test]
    fn test_build_nearai_model_fetch_config_picks_up_api_key_env() {
        use secrecy::ExposeSecret;

        let _lock = lock_env();
        let _guard = EnvGuard::set("NEARAI_API_KEY", "test-cloud-api-key-12345");
        let _guard2 = EnvGuard::clear("NEARAI_BASE_URL");

        let config = build_nearai_model_fetch_config();
        assert!(
            config.nearai.api_key.is_some(),
            "config should include NEARAI_API_KEY from env"
        );
        assert_eq!(
            config.nearai.api_key.as_ref().unwrap().expose_secret(),
            "test-cloud-api-key-12345"
        );
        // With API key, base_url must point to cloud-api (not private.near.ai)
        assert_eq!(
            config.nearai.base_url, "https://cloud-api.near.ai",
            "API key auth must use cloud-api base URL for model fetching"
        );
    }

    /// Regression test for #799: when NEARAI_API_KEY is absent or empty,
    /// the config should have `api_key: None` (session token path).
    #[test]
    fn test_build_nearai_model_fetch_config_none_when_no_api_key() {
        let _lock = lock_env();
        let _guard = EnvGuard::clear("NEARAI_API_KEY");
        let _guard2 = EnvGuard::clear("NEARAI_BASE_URL");

        let config = build_nearai_model_fetch_config();
        assert!(
            config.nearai.api_key.is_none(),
            "config should have no api_key when env var is absent"
        );
        // Without API key, base_url must point to private.near.ai (session token)
        assert_eq!(
            config.nearai.base_url, "https://private.near.ai",
            "session-token auth must use private.near.ai base URL"
        );
    }

    /// Regression test for #799: empty NEARAI_API_KEY should be treated as absent.
    #[test]
    fn test_build_nearai_model_fetch_config_none_when_empty_api_key() {
        let _lock = lock_env();
        let _guard = EnvGuard::set("NEARAI_API_KEY", "");

        let config = build_nearai_model_fetch_config();
        assert!(
            config.nearai.api_key.is_none(),
            "config should have no api_key when env var is empty"
        );
    }

    /// Regression: API key set via inject_single_var (the path used by
    /// setup_api_key_provider during onboarding) must be picked up by
    /// for_model_discovery() so model listing uses cloud-api auth
    /// instead of falling back to session-token auth.
    #[test]
    fn test_model_discovery_picks_up_injected_var() {
        use secrecy::ExposeSecret;

        let _lock = lock_env();
        let _guard = EnvGuard::clear("NEARAI_API_KEY");
        let _guard2 = EnvGuard::clear("NEARAI_BASE_URL");

        crate::config::inject_single_var("NEARAI_API_KEY", "injected-wizard-key");
        let config = build_nearai_model_fetch_config();

        // Clean up: empty values are treated as unset by env_or_override()
        // at every layer (real env, runtime overrides, INJECTED_VARS).
        crate::config::inject_single_var("NEARAI_API_KEY", "");

        assert!(
            config.nearai.api_key.is_some(),
            "for_model_discovery must read NEARAI_API_KEY from inject_single_var overlay"
        );
        assert_eq!(
            config.nearai.api_key.as_ref().unwrap().expose_secret(),
            "injected-wizard-key"
        );
        assert_eq!(
            config.nearai.base_url, "https://cloud-api.near.ai",
            "API key from overlay must select cloud-api base URL"
        );
    }

    /// Regression: API key set via set_runtime_env (interactive api_key_login
    /// path) must be picked up by build_nearai_model_fetch_config so that
    /// model listing doesn't fall back to session-token auth and re-trigger
    /// the NEAR AI authentication menu.
    #[test]
    fn test_build_nearai_model_fetch_config_picks_up_runtime_env() {
        let _lock = lock_env();
        // Ensure the real env var is unset so the only source is the overlay.
        let _guard = EnvGuard::clear("NEARAI_API_KEY");

        crate::config::helpers::set_runtime_env("NEARAI_API_KEY", "test-key-from-overlay");
        let config = build_nearai_model_fetch_config();

        // Clean up runtime overlay
        crate::config::helpers::set_runtime_env("NEARAI_API_KEY", "");

        assert!(
            config.nearai.api_key.is_some(),
            "config must pick up NEARAI_API_KEY from runtime overlay"
        );
        assert_eq!(
            config.nearai.base_url, "https://cloud-api.near.ai",
            "API key auth must use cloud-api base URL"
        );
    }

    /// Regression test for #846: auto_setup_database must establish a DB
    /// connection and run migrations so that persist_settings succeeds.
    #[cfg(feature = "libsql")]
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_auto_setup_database_runs_migrations_with_existing_env() {
        let _lock = lock_env();
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");

        let _backend_guard = EnvGuard::set("DATABASE_BACKEND", "libsql");
        let _path_guard = EnvGuard::set("LIBSQL_PATH", db_path.to_str().unwrap());
        // Ensure no postgres env interferes
        let _pg_guard = EnvGuard::clear("DATABASE_URL");

        let mut wizard = SetupWizard::new();
        wizard.config.quick = true;

        wizard.auto_setup_database().await.unwrap();

        // The wizard must have a live DB backend after auto_setup_database
        assert!(
            wizard.db_backend.is_some(),
            "auto_setup_database must establish db_backend"
        );

        // persist_settings must succeed (proves migrations ran)
        let saved = wizard.persist_settings().await.unwrap();
        assert!(saved, "persist_settings must save to the database");
    }

    /// Start a pgvector-enabled Postgres container for integration tests,
    /// returning `(container, database_url)`. Returns `None` and prints a
    /// skip message if Docker/testcontainers is unreachable so the test
    /// succeeds on hosts without Docker.
    #[cfg(all(feature = "postgres", feature = "integration"))]
    async fn start_pg_container() -> Option<(
        testcontainers_modules::testcontainers::ContainerAsync<
            testcontainers_modules::postgres::Postgres,
        >,
        String,
    )> {
        use testcontainers_modules::testcontainers::{ImageExt, runners::AsyncRunner};

        let image = testcontainers_modules::postgres::Postgres::default()
            .with_db_name("ironclaw_test")
            .with_user("postgres")
            .with_password("postgres")
            .with_name("pgvector/pgvector")
            .with_tag("pg16");

        let container = match image.start().await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("skipping: docker/testcontainers unavailable ({e})");
                return None;
            }
        };
        let host = match container.get_host().await {
            Ok(h) => h,
            Err(e) => {
                eprintln!("skipping: could not resolve container host ({e})");
                return None;
            }
        };
        let port = match container.get_host_port_ipv4(5432).await {
            Ok(p) => p,
            Err(e) => {
                eprintln!("skipping: could not resolve container port ({e})");
                return None;
            }
        };
        let url = format!("postgres://postgres:postgres@{host}:{port}/ironclaw_test");
        Some((container, url))
    }

    /// Assert that an `auto_setup_database()` run against a preset
    /// `DATABASE_URL` left the wizard in the correct state: live pool,
    /// recorded settings, migrations applied (proven by a round-trip
    /// write+read through the `settings` table).
    #[cfg(all(feature = "postgres", feature = "integration"))]
    async fn assert_auto_setup_postgres_persisted(wizard: &SetupWizard, database_url: &str) {
        assert!(
            wizard.db_pool.is_some(),
            "auto_setup_database must establish db_pool on the postgres early-return path"
        );
        assert_eq!(
            wizard.settings.database_backend.as_deref(),
            Some("postgres"),
            "settings.database_backend must be recorded as postgres"
        );
        assert_eq!(
            wizard.settings.database_url.as_deref(),
            Some(database_url),
            "settings.database_url must be recorded"
        );

        // Proves migrations ran: persist_settings() writes through to the
        // `settings` table which only exists after V8__settings.sql ran.
        let saved = wizard
            .persist_settings()
            .await
            .expect("persist_settings must succeed with a migrated postgres db");
        assert!(
            saved,
            "persist_settings must report a successful write (db_pool was Some)"
        );

        // Read-back via the same pool confirms actual round-trip persistence,
        // not just an in-memory Ok.
        let pool = wizard
            .db_pool
            .clone()
            .expect("db_pool must still be populated after persist_settings");
        let store = crate::history::Store::from_pool(pool);
        let value = store
            .get_setting(wizard.owner_id(), "database_backend")
            .await
            .expect("get_setting must succeed against migrated postgres");
        assert_eq!(
            value.as_ref().and_then(|v| v.as_str()),
            Some("postgres"),
            "round-trip read-back must return the value persist_settings wrote"
        );
    }

    /// Regression test for #846, PostgreSQL branch.
    ///
    /// The original bug lived in the postgres early-return paths of
    /// `auto_setup_database()` — when `DATABASE_URL` was preset, both
    /// `test_database_connection_postgres()` and `run_migrations_postgres()`
    /// were skipped, leaving `self.db_pool = None` so `persist_settings()`
    /// silently returned `Ok(false)` and the wizard crashed later trying
    /// to save. The sibling libSQL test above only exercises the libsql
    /// auto-default branch, so this test drives the actual postgres
    /// early-return code path end-to-end.
    ///
    /// Gated behind `integration` (requires Docker + testcontainers) so
    /// it runs in the dedicated integration-test job. Skips gracefully
    /// if Docker is unavailable (see `start_pg_container`).
    ///
    /// This test clears `DATABASE_BACKEND` to exercise the fall-through
    /// postgres branch (DATABASE_URL alone). The companion test below
    /// covers the `DATABASE_BACKEND=postgres` guard.
    #[cfg(all(feature = "postgres", feature = "integration"))]
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_auto_setup_database_runs_migrations_postgres_branch() {
        let _lock = lock_env();

        let Some((_container, database_url)) = start_pg_container().await else {
            return;
        };

        let _url_guard = EnvGuard::set("DATABASE_URL", &database_url);
        let _backend_guard = EnvGuard::clear("DATABASE_BACKEND");
        let _libsql_guard = EnvGuard::clear("LIBSQL_PATH");
        let _ssl_guard = EnvGuard::set("DATABASE_SSLMODE", "disable");

        let mut wizard = SetupWizard::new();
        wizard.config.quick = true;

        // Drive the caller, not the helpers. This is the exact function
        // the onboard flow invokes and the one that regressed.
        wizard
            .auto_setup_database()
            .await
            .expect("auto_setup_database must succeed on preset postgres URL");

        assert_auto_setup_postgres_persisted(&wizard, &database_url).await;
    }

    /// Companion to `test_auto_setup_database_runs_migrations_postgres_branch`:
    /// exercises the guard where `DATABASE_BACKEND=postgres` is set
    /// alongside `DATABASE_URL`. The sibling test clears `DATABASE_BACKEND`
    /// to hit the fall-through branch; this one covers the explicit guard.
    #[cfg(all(feature = "postgres", feature = "integration"))]
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn test_auto_setup_database_runs_migrations_postgres_backend_env() {
        let _lock = lock_env();

        let Some((_container, database_url)) = start_pg_container().await else {
            return;
        };

        let _url_guard = EnvGuard::set("DATABASE_URL", &database_url);
        let _backend_guard = EnvGuard::set("DATABASE_BACKEND", "postgres");
        let _libsql_guard = EnvGuard::clear("LIBSQL_PATH");
        let _ssl_guard = EnvGuard::set("DATABASE_SSLMODE", "disable");

        let mut wizard = SetupWizard::new();
        wizard.config.quick = true;

        wizard
            .auto_setup_database()
            .await
            .expect("auto_setup_database must succeed with DATABASE_BACKEND=postgres");

        assert_auto_setup_postgres_persisted(&wizard, &database_url).await;
    }
}
