//! Main setup wizard orchestration.
//!
//! The wizard guides users through:
//! 1. Database connection
//! 2. Security (secrets master key)
//! 3. Inference provider (NEAR AI, Anthropic, OpenAI, Ollama, OpenAI-compatible)
//! 4. Model selection
//! 5. Embeddings
//! 6. Channel configuration
//! 7. Extensions (tool installation from registry)
//! 8. Docker sandbox
//! 9. Heartbeat (background tasks)

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[cfg(feature = "postgres")]
use deadpool_postgres::{Config as PoolConfig, Runtime};
use secrecy::{ExposeSecret, SecretString};
#[cfg(feature = "postgres")]
use tokio_postgres::NoTls;

use crate::channels::wasm::{
    ChannelCapabilitiesFile, available_channel_names, install_bundled_channel,
};
use crate::llm::{SessionConfig, SessionManager};
use crate::secrets::{SecretsCrypto, SecretsStore};
use crate::settings::{KeySource, Settings};
use crate::setup::channels::{
    SecretsContext, setup_http, setup_signal, setup_telegram, setup_tunnel, setup_wasm_channel,
};
use crate::setup::prompts::{
    confirm, input, optional_input, print_error, print_header, print_info, print_step,
    print_success, secret_input, select_many, select_one,
};

// unused const, keep commented for clarity / future use
// const CHANNEL_INDEX_CLI: usize = 0;
const CHANNEL_INDEX_HTTP: usize = 1;
const CHANNEL_INDEX_SIGNAL: usize = 2;

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
}

/// Interactive setup wizard for IronClaw.
pub struct SetupWizard {
    config: SetupConfig,
    settings: Settings,
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
    /// Create a new setup wizard.
    pub fn new() -> Self {
        Self {
            config: SetupConfig::default(),
            settings: Settings::default(),
            session_manager: None,
            #[cfg(feature = "postgres")]
            db_pool: None,
            #[cfg(feature = "libsql")]
            db_backend: None,
            secrets_crypto: None,
            llm_api_key: None,
        }
    }

    /// Create a wizard with custom configuration.
    pub fn with_config(config: SetupConfig) -> Self {
        Self {
            config,
            settings: Settings::default(),
            session_manager: None,
            #[cfg(feature = "postgres")]
            db_pool: None,
            #[cfg(feature = "libsql")]
            db_backend: None,
            secrets_crypto: None,
            llm_api_key: None,
        }
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
        print_header("IronClaw Setup Wizard");

        if self.config.channels_only {
            // Channels-only mode: reconnect to existing DB and load settings
            // before running the channel step, so secrets and save work.
            self.reconnect_existing_db().await?;
            print_step(1, 1, "Channel Configuration");
            self.step_channels().await?;
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
            if let Ok(map) = store.get_all_settings("default").await {
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
            if let Ok(map) = db.get_all_settings("default").await {
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
    #[cfg(feature = "postgres")]
    async fn test_database_connection_postgres(&mut self, url: &str) -> Result<(), SetupError> {
        let mut cfg = PoolConfig::new();
        cfg.url = Some(url.to_string());
        cfg.pool = Some(deadpool_postgres::PoolConfig {
            max_size: 5,
            ..Default::default()
        });

        let pool = cfg
            .create_pool(Some(Runtime::Tokio1), NoTls)
            .map_err(|e| SetupError::Database(format!("Failed to create pool: {}", e)))?;

        let _ = pool
            .get()
            .await
            .map_err(|e| SetupError::Database(format!("Failed to connect: {}", e)))?;

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
    #[cfg(feature = "postgres")]
    async fn run_migrations_postgres(&self) -> Result<(), SetupError> {
        if let Some(ref pool) = self.db_pool {
            use refinery::embed_migrations;
            embed_migrations!("migrations");

            print_info("Running migrations...");

            let mut client = pool
                .get()
                .await
                .map_err(|e| SetupError::Database(format!("Pool error: {}", e)))?;

            migrations::runner()
                .run_async(&mut **client)
                .await
                .map_err(|e| SetupError::Database(format!("Migration failed: {}", e)))?;

            print_success("Migrations applied");
        }
        Ok(())
    }

    /// Run libSQL migrations.
    #[cfg(feature = "libsql")]
    async fn run_migrations_libsql(&self) -> Result<(), SetupError> {
        if let Some(ref backend) = self.db_backend {
            use crate::db::Database;

            print_info("Running migrations...");

            backend
                .run_migrations()
                .await
                .map_err(|e| SetupError::Database(format!("Migration failed: {}", e)))?;

            print_success("Migrations applied");
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
                // Env var mode
                print_info("Generate a key and add it to your environment:");
                let key_hex = crate::secrets::keychain::generate_master_key_hex();
                println!();
                println!("  export SECRETS_MASTER_KEY={}", key_hex);
                println!();
                print_info("Add this to your shell profile or .env file.");

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

    /// Step 3: Inference provider selection.
    ///
    /// Lets the user pick from all supported LLM backends, then runs the
    /// provider-specific auth sub-flow (API key entry, NEAR AI login, etc.).
    async fn step_inference_provider(&mut self) -> Result<(), SetupError> {
        // Show current provider if already configured
        if let Some(ref current) = self.settings.llm_backend {
            let is_openrouter = current == "openai_compatible"
                && self
                    .settings
                    .openai_compatible_base_url
                    .as_deref()
                    .is_some_and(|u| u.contains("openrouter.ai"));

            let display = if is_openrouter {
                "OpenRouter"
            } else {
                match current.as_str() {
                    "nearai" => "NEAR AI",
                    "anthropic" => "Anthropic (Claude)",
                    "openai" => "OpenAI",
                    "ollama" => "Ollama (local)",
                    "openai_compatible" => "OpenAI-compatible endpoint",
                    other => other,
                }
            };
            print_info(&format!("Current provider: {}", display));
            println!();

            let is_known = matches!(
                current.as_str(),
                "nearai" | "anthropic" | "openai" | "ollama" | "openai_compatible"
            );

            if is_known && confirm("Keep current provider?", true).map_err(SetupError::Io)? {
                // Still run the auth sub-flow in case they need to update keys
                if is_openrouter {
                    return self.setup_openrouter().await;
                }
                match current.as_str() {
                    "nearai" => return self.setup_nearai().await,
                    "anthropic" => return self.setup_anthropic().await,
                    "openai" => return self.setup_openai().await,
                    "ollama" => return self.setup_ollama(),
                    "openai_compatible" => return self.setup_openai_compatible().await,
                    _ => {
                        return Err(SetupError::Config(format!(
                            "Unhandled provider: {}",
                            current
                        )));
                    }
                }
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

        let options = &[
            "NEAR AI          - multi-model access via NEAR account",
            "Anthropic        - Claude models (direct API key)",
            "OpenAI           - GPT models (direct API key)",
            "Ollama           - local models, no API key needed",
            "OpenRouter       - 200+ models via single API key",
            "OpenAI-compatible - custom endpoint (vLLM, LiteLLM, etc.)",
        ];

        let choice = select_one("Provider:", options).map_err(SetupError::Io)?;

        match choice {
            0 => self.setup_nearai().await?,
            1 => self.setup_anthropic().await?,
            2 => self.setup_openai().await?,
            3 => self.setup_ollama()?,
            4 => self.setup_openrouter().await?,
            5 => self.setup_openai_compatible().await?,
            _ => return Err(SetupError::Config("Invalid provider selection".to_string())),
        }

        Ok(())
    }

    /// NEAR AI provider setup (extracted from the old step_authentication).
    async fn setup_nearai(&mut self) -> Result<(), SetupError> {
        self.settings.llm_backend = Some("nearai".to_string());

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
            let config = SessionConfig::default();
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
        // in the environment. Persist it to the encrypted secrets store
        // so inject_llm_keys_from_secrets() can load it on future runs.
        if let Ok(api_key) = std::env::var("NEARAI_API_KEY")
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

    /// Anthropic OAuth setup: extract token from `claude login` credentials.
    async fn setup_anthropic_oauth(&mut self) -> Result<(), SetupError> {
        self.settings.llm_backend = Some("anthropic".to_string());
        if self.settings.selected_model.is_some() {
            self.settings.selected_model = None;
        }

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
                // Give user time to run claude login
                if let Some(token) = crate::config::ClaudeCodeConfig::extract_oauth_token() {
                    print_info(&format!("Found OAuth token: {}", mask_api_key(&token)));
                    return self.save_anthropic_oauth_token(&token).await;
                }
                print_error("Still no OAuth token found.");
            }
        }

        // Fallback: let user paste the token manually
        print_info("You can paste your OAuth token directly (starts with sk-ant-oat01-).");
        let token = secret_input("Anthropic OAuth token").map_err(SetupError::Io)?;
        let token_str = token.expose_secret();
        if token_str.is_empty() {
            return Err(SetupError::Config(
                "OAuth token cannot be empty".to_string(),
            ));
        }
        self.save_anthropic_oauth_token(token_str).await
    }

    /// Save an Anthropic OAuth token to secrets and set env for immediate use.
    async fn save_anthropic_oauth_token(&mut self, token: &str) -> Result<(), SetupError> {
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

        // Set env var for immediate use (model selection step)
        // SAFETY: Single-threaded wizard context.
        unsafe {
            std::env::set_var("ANTHROPIC_OAUTH_TOKEN", token);
        }

        // Cache for model fetching
        self.llm_api_key = Some(SecretString::from(token.to_string()));

        print_success("Anthropic OAuth configured");
        Ok(())
    }

    /// OpenAI provider setup: API key or Codex OAuth token.
    async fn setup_openai(&mut self) -> Result<(), SetupError> {
        let options = &["Direct API Key", "OAuth Token (from Codex CLI)"];
        let choice = select_one("How do you want to authenticate with OpenAI?", options)
            .map_err(SetupError::Io)?;

        if choice == 0 {
            // Standard API key flow
            self.setup_api_key_provider(
                "openai",
                "OPENAI_API_KEY",
                "llm_openai_api_key",
                "OpenAI API key",
                "https://platform.openai.com/api-keys",
                None,
            )
            .await
        } else {
            // Codex OAuth token flow
            self.setup_codex_oauth().await
        }
    }

    /// Codex OAuth setup: extract token from `~/.codex/auth.json`.
    async fn setup_codex_oauth(&mut self) -> Result<(), SetupError> {
        self.settings.llm_backend = Some("openai".to_string());
        if self.settings.selected_model.is_some() {
            self.settings.selected_model = None;
        }

        // Try to extract existing Codex token
        if let Some(token) = crate::config::extract_codex_oauth_token() {
            print_info(&format!("Found Codex token: {}", mask_api_key(&token)));
            if confirm("Use this token?", true).map_err(SetupError::Io)? {
                return self.save_codex_oauth_token(&token).await;
            }
        } else {
            print_info("No Codex token found in ~/.codex/auth.json.");
        }

        // Fallback: let user paste the token
        print_info("You can paste your Codex/OpenAI OAuth token directly.");
        let token = secret_input("Codex OAuth token").map_err(SetupError::Io)?;
        let token_str = token.expose_secret();
        if token_str.is_empty() {
            return Err(SetupError::Config(
                "OAuth token cannot be empty".to_string(),
            ));
        }
        self.save_codex_oauth_token(token_str).await
    }

    /// Save a Codex OAuth token — stored as the OpenAI API key since it's functionally equivalent.
    async fn save_codex_oauth_token(&mut self, token: &str) -> Result<(), SetupError> {
        // Store as the OpenAI API key (Codex tokens use the same Bearer auth)
        if let Ok(ctx) = self.init_secrets_context().await {
            let key = SecretString::from(token.to_string());
            ctx.save_secret("llm_openai_api_key", &key)
                .await
                .map_err(|e| SetupError::Config(format!("Failed to save Codex token: {e}")))?;
            print_success("Codex token encrypted and saved");
        } else {
            print_info("Secrets not available. Set OPENAI_API_KEY in your environment.");
        }

        // Set env var for immediate use (model selection step)
        // SAFETY: Single-threaded wizard context.
        unsafe {
            std::env::set_var("OPENAI_API_KEY", token);
        }

        // Cache for model fetching
        self.llm_api_key = Some(SecretString::from(token.to_string()));

        print_success("Codex OAuth configured (as OpenAI API key)");
        Ok(())
    }

    /// Shared setup flow for API-key-based providers (Anthropic, OpenAI, OpenRouter).
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

        self.settings.llm_backend = Some(backend.to_string());
        if self.settings.selected_model.is_some() {
            self.settings.selected_model = None;
        }

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

        // Set env var so write_bootstrap_env() can persist it, and so
        // subsequent wizard steps (model selection) can use it immediately.
        // SAFETY: Single-threaded wizard context.
        unsafe { std::env::set_var(env_var, key_str) };

        // Cache key in memory for model fetching later in the wizard
        self.llm_api_key = Some(SecretString::from(key_str.to_string()));

        print_success(&format!("{display_name} configured"));
        Ok(())
    }

    /// Ollama provider setup: just needs a base URL, no API key.
    fn setup_ollama(&mut self) -> Result<(), SetupError> {
        self.settings.llm_backend = Some("ollama".to_string());
        if self.settings.selected_model.is_some() {
            self.settings.selected_model = None;
        }

        let default_url = self
            .settings
            .ollama_base_url
            .as_deref()
            .unwrap_or("http://localhost:11434");

        let url_input = optional_input(
            "Ollama base URL",
            Some(&format!("default: {}", default_url)),
        )
        .map_err(SetupError::Io)?;

        let url = url_input.unwrap_or_else(|| default_url.to_string());
        self.settings.ollama_base_url = Some(url.clone());

        print_success(&format!("Ollama configured ({})", url));
        Ok(())
    }

    /// OpenRouter provider setup: pre-configured OpenAI-compatible endpoint.
    ///
    /// Sets the base URL to `https://openrouter.ai/api/v1` and delegates
    /// API key collection to `setup_api_key_provider` with a display name
    /// override so messages say "OpenRouter" instead of "openai_compatible".
    async fn setup_openrouter(&mut self) -> Result<(), SetupError> {
        self.settings.openai_compatible_base_url = Some("https://openrouter.ai/api/v1".to_string());
        self.setup_api_key_provider(
            "openai_compatible",
            "LLM_API_KEY",
            "llm_compatible_api_key",
            "OpenRouter API key",
            "https://openrouter.ai/settings/keys",
            Some("OpenRouter"),
        )
        .await
    }

    /// OpenAI-compatible provider setup: base URL + optional API key.
    async fn setup_openai_compatible(&mut self) -> Result<(), SetupError> {
        self.settings.llm_backend = Some("openai_compatible".to_string());
        if self.settings.selected_model.is_some() {
            self.settings.selected_model = None;
        }

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
            return Err(SetupError::Config(
                "Base URL is required for OpenAI-compatible provider".to_string(),
            ));
        }

        self.settings.openai_compatible_base_url = Some(url.clone());

        // Optional API key
        if confirm("Does this endpoint require an API key?", false).map_err(SetupError::Io)? {
            let key = secret_input("API key").map_err(SetupError::Io)?;
            let key_str = key.expose_secret();

            if !key_str.is_empty() {
                if let Ok(ctx) = self.init_secrets_context().await {
                    ctx.save_secret("llm_compatible_api_key", &key)
                        .await
                        .map_err(|e| {
                            SetupError::Config(format!("Failed to save API key: {}", e))
                        })?;
                    print_success("API key encrypted and saved");
                } else {
                    print_info("Secrets not available. Set LLM_API_KEY in your environment.");
                }
            }
        }

        print_success(&format!("OpenAI-compatible configured ({})", url));
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

        match backend {
            "anthropic" => {
                let cached = self
                    .llm_api_key
                    .as_ref()
                    .map(|k| k.expose_secret().to_string());
                let models = fetch_anthropic_models(cached.as_deref()).await;
                self.select_from_model_list(&models)?;
            }
            "openai" => {
                let cached = self
                    .llm_api_key
                    .as_ref()
                    .map(|k| k.expose_secret().to_string());
                let models = fetch_openai_models(cached.as_deref()).await;
                self.select_from_model_list(&models)?;
            }
            "ollama" => {
                let base_url = self
                    .settings
                    .ollama_base_url
                    .as_deref()
                    .unwrap_or("http://localhost:11434");
                let models = fetch_ollama_models(base_url).await;
                if models.is_empty() {
                    print_info("No models found. Pull one first: ollama pull llama3");
                }
                self.select_from_model_list(&models)?;
            }
            "openai_compatible" => {
                // No standard API for listing models on arbitrary endpoints
                let model_id = input("Model name (e.g., meta-llama/Llama-3-8b-chat-hf)")
                    .map_err(SetupError::Io)?;
                if model_id.is_empty() {
                    return Err(SetupError::Config("Model name is required".to_string()));
                }
                self.settings.selected_model = Some(model_id.clone());
                print_success(&format!("Selected {}", model_id));
            }
            _ => {
                // NEAR AI: use existing provider list_models()
                let fetched = self.fetch_nearai_models().await;
                let default_models: Vec<(String, String)> = vec![
                    (
                        "zai-org/GLM-latest".into(),
                        "GLM Latest (default, fast)".into(),
                    ),
                    (
                        "anthropic::claude-sonnet-4-20250514".into(),
                        "Claude Sonnet 4 (best quality)".into(),
                    ),
                    (
                        "openai::gpt-5.3-codex".into(),
                        "GPT-5.3 Codex (flagship)".into(),
                    ),
                    ("openai::gpt-5.2".into(), "GPT-5.2".into()),
                    ("openai::gpt-4o".into(), "GPT-4o".into()),
                ];

                let models = if fetched.is_empty() {
                    default_models
                } else {
                    fetched.iter().map(|m| (m.clone(), m.clone())).collect()
                };
                self.select_from_model_list(&models)?;
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
    async fn fetch_nearai_models(&self) -> Vec<String> {
        let session = match self.session_manager {
            Some(ref s) => Arc::clone(s),
            None => return vec![],
        };

        use crate::config::LlmConfig;
        use crate::llm::create_llm_provider;

        let base_url = std::env::var("NEARAI_BASE_URL")
            .unwrap_or_else(|_| "https://private.near.ai".to_string());
        let auth_base_url = std::env::var("NEARAI_AUTH_URL")
            .unwrap_or_else(|_| "https://private.near.ai".to_string());

        let config = LlmConfig {
            backend: crate::config::LlmBackend::NearAi,
            nearai: crate::config::NearAiConfig {
                model: "dummy".to_string(),
                cheap_model: None,
                base_url,
                auth_base_url,
                session_path: crate::llm::session::default_session_path(),
                api_key: None,
                fallback_model: None,
                max_retries: 3,
                circuit_breaker_threshold: None,
                circuit_breaker_recovery_secs: 30,
                response_cache_enabled: false,
                response_cache_ttl_secs: 3600,
                response_cache_max_entries: 1000,
                failover_cooldown_secs: 300,
                failover_cooldown_threshold: 3,
                smart_routing_cascade: true,
            },
            openai: None,
            anthropic: None,
            ollama: None,
            openai_compatible: None,
            tinfoil: None,
        };

        match create_llm_provider(&config, session) {
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

        // If the LLM backend is OpenAI and we already have a key, default to OpenAI embeddings
        if backend == "openai" && has_openai_key {
            self.settings.embeddings.enabled = true;
            self.settings.embeddings.provider = "openai".to_string();
            self.settings.embeddings.model = "text-embedding-3-small".to_string();
            print_success("Embeddings enabled via OpenAI (using existing API key)");
            return Ok(());
        }

        // If no NEAR AI session and no OpenAI key, only OpenAI is viable
        if !has_nearai && !has_openai_key {
            print_info("No NEAR AI session or OpenAI key found for embeddings.");
            print_info("Set OPENAI_API_KEY in your environment to enable embeddings.");
            self.settings.embeddings.enabled = false;
            return Ok(());
        }

        let mut options = Vec::new();
        if has_nearai {
            options.push("NEAR AI (uses same auth, no extra cost)");
        }
        options.push("OpenAI (requires API key)");

        let choice = select_one("Select embeddings provider:", &options).map_err(SetupError::Io)?;

        // Map choice back to provider name
        let provider = if has_nearai && choice == 0 {
            "nearai"
        } else {
            "openai"
        };

        match provider {
            "nearai" => {
                self.settings.embeddings.enabled = true;
                self.settings.embeddings.provider = "nearai".to_string();
                self.settings.embeddings.model = "text-embedding-3-small".to_string();
                print_success("Embeddings enabled via NEAR AI");
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
        // Respect the user's selected backend when both features are compiled,
        // so we don't accidentally use a postgres pool from DATABASE_URL when
        // libsql was chosen (or vice versa).
        let selected_backend = self
            .settings
            .database_backend
            .as_deref()
            .unwrap_or("postgres");

        #[cfg(all(feature = "libsql", feature = "postgres"))]
        {
            if selected_backend == "libsql" {
                if let Some(store) = self.create_libsql_secrets_store(&crypto)? {
                    return Ok(SecretsContext::from_store(store, "default"));
                }
                if let Some(store) = self.create_postgres_secrets_store(&crypto).await? {
                    return Ok(SecretsContext::from_store(store, "default"));
                }
            } else {
                if let Some(store) = self.create_postgres_secrets_store(&crypto).await? {
                    return Ok(SecretsContext::from_store(store, "default"));
                }
                if let Some(store) = self.create_libsql_secrets_store(&crypto)? {
                    return Ok(SecretsContext::from_store(store, "default"));
                }
            }
        }

        #[cfg(all(feature = "postgres", not(feature = "libsql")))]
        {
            let _ = selected_backend;
            if let Some(store) = self.create_postgres_secrets_store(&crypto).await? {
                return Ok(SecretsContext::from_store(store, "default"));
            }
        }

        #[cfg(all(feature = "libsql", not(feature = "postgres")))]
        {
            let _ = selected_backend;
            if let Some(store) = self.create_libsql_secrets_store(&crypto)? {
                return Ok(SecretsContext::from_store(store, "default"));
            }
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
        match setup_tunnel(&self.settings) {
            Ok(tunnel_settings) => {
                self.settings.tunnel = tunnel_settings;
            }
            Err(e) => {
                print_info(&format!("Tunnel setup skipped: {}", e));
            }
        }
        println!();

        // Discover available WASM channels
        let channels_dir = dirs::home_dir()
            .ok_or_else(|| SetupError::Config("Could not determine home directory".into()))?
            .join(".ironclaw/channels");

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
                    } else if channel_name == "telegram" {
                        let telegram_result = setup_telegram(ctx, &self.settings).await?;
                        if let Some(owner_id) = telegram_result.owner_id {
                            self.settings.channels.telegram_owner_id = Some(owner_id);
                        }
                        crate::setup::channels::WasmChannelSetupResult {
                            enabled: telegram_result.enabled,
                            channel_name: "telegram".to_string(),
                        }
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
        let tools_dir = dirs::home_dir()
            .ok_or_else(|| SetupError::Config("Could not determine home directory".into()))?
            .join(".ironclaw/tools");

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
            dirs::home_dir()
                .unwrap_or_default()
                .join(".ironclaw/channels"),
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

        // Check for Anthropic credentials (API key or OAuth token)
        let has_api_key = std::env::var("ANTHROPIC_API_KEY")
            .is_ok_and(|v| !v.is_empty() && v != "oauth-placeholder");
        let has_oauth = crate::config::ClaudeCodeConfig::extract_oauth_token().is_some()
            || std::env::var("ANTHROPIC_OAUTH_TOKEN").is_ok_and(|v| !v.is_empty());

        if has_api_key || has_oauth {
            self.settings.sandbox.claude_code_enabled = true;
            print_success("Claude Code sandbox enabled");
        } else {
            print_error("No Anthropic credentials found.");
            print_info(
                "Claude Code needs ANTHROPIC_API_KEY or an OAuth token from `claude login`.",
            );
            println!();

            if confirm("Retry after setting up credentials?", false).map_err(SetupError::Io)? {
                let has_oauth_retry =
                    crate::config::ClaudeCodeConfig::extract_oauth_token().is_some();
                let has_key_retry = std::env::var("ANTHROPIC_API_KEY")
                    .is_ok_and(|v| !v.is_empty() && v != "oauth-placeholder");

                if has_key_retry || has_oauth_retry {
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
                    .set_all_settings("default", &db_map)
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
                    .set_all_settings("default", &db_map)
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
    /// These are the chicken-and-egg settings needed before the database is
    /// connected (DATABASE_BACKEND, DATABASE_URL, LLM_BACKEND, etc.).
    fn write_bootstrap_env(&self) -> Result<(), SetupError> {
        let mut env_vars: Vec<(&str, String)> = Vec::new();

        if let Some(ref backend) = self.settings.database_backend {
            env_vars.push(("DATABASE_BACKEND", backend.clone()));
        }
        if let Some(ref url) = self.settings.database_url {
            env_vars.push(("DATABASE_URL", url.clone()));
        }
        if let Some(ref path) = self.settings.libsql_path {
            env_vars.push(("LIBSQL_PATH", path.clone()));
        }
        if let Some(ref url) = self.settings.libsql_url {
            env_vars.push(("LIBSQL_URL", url.clone()));
        }

        // LLM bootstrap vars: same chicken-and-egg problem as DATABASE_BACKEND.
        // Config::from_env() needs the backend before the DB is connected.
        if let Some(ref backend) = self.settings.llm_backend {
            env_vars.push(("LLM_BACKEND", backend.clone()));
        }
        if let Some(ref url) = self.settings.openai_compatible_base_url {
            env_vars.push(("LLM_BASE_URL", url.clone()));
        }
        if let Some(ref url) = self.settings.ollama_base_url {
            env_vars.push(("OLLAMA_BASE_URL", url.clone()));
        }

        // Preserve LLM credentials: Config::from_env() runs before the DB is
        // connected, so secrets stored only in the encrypted DB aren't available
        // yet. Write any present API keys / OAuth tokens to the bootstrap .env
        // so the initial config resolution succeeds.
        let credential_vars = [
            "NEARAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_OAUTH_TOKEN",
            "OPENAI_API_KEY",
            "CODEX_OAUTH_TOKEN",
            "LLM_API_KEY",
            "TINFOIL_API_KEY",
        ];
        for var in credential_vars {
            if let Ok(val) = std::env::var(var)
                && !val.is_empty()
                && val != "oauth-placeholder"
            {
                env_vars.push((var, val));
            }
        }

        // Always write ONBOARD_COMPLETED so that check_onboard_needed()
        // (which runs before the DB is connected) knows to skip re-onboarding.
        if self.settings.onboard_completed {
            env_vars.push(("ONBOARD_COMPLETED", "true".to_string()));
        }

        // Claude Code sandbox mode
        if self.settings.sandbox.claude_code_enabled {
            env_vars.push(("CLAUDE_CODE_ENABLED", "true".to_string()));
        }

        // Signal channel env vars (chicken-and-egg: config resolves before DB).
        if let Some(ref url) = self.settings.channels.signal_http_url {
            env_vars.push(("SIGNAL_HTTP_URL", url.clone()));
        }
        if let Some(ref account) = self.settings.channels.signal_account {
            env_vars.push(("SIGNAL_ACCOUNT", account.clone()));
        }
        if let Some(ref allow_from) = self.settings.channels.signal_allow_from {
            env_vars.push(("SIGNAL_ALLOW_FROM", allow_from.clone()));
        }
        if let Some(ref allow_from_groups) = self.settings.channels.signal_allow_from_groups
            && !allow_from_groups.is_empty()
        {
            env_vars.push(("SIGNAL_ALLOW_FROM_GROUPS", allow_from_groups.clone()));
        }
        if let Some(ref dm_policy) = self.settings.channels.signal_dm_policy {
            env_vars.push(("SIGNAL_DM_POLICY", dm_policy.clone()));
        }
        if let Some(ref group_policy) = self.settings.channels.signal_group_policy {
            env_vars.push(("SIGNAL_GROUP_POLICY", group_policy.clone()));
        }
        if let Some(ref group_allow_from) = self.settings.channels.signal_group_allow_from
            && !group_allow_from.is_empty()
        {
            env_vars.push(("SIGNAL_GROUP_ALLOW_FROM", group_allow_from.clone()));
        }

        if !env_vars.is_empty() {
            let pairs: Vec<(&str, &str)> = env_vars.iter().map(|(k, v)| (*k, v.as_str())).collect();
            crate::bootstrap::save_bootstrap_env(&pairs).map_err(|e| {
                SetupError::Io(std::io::Error::other(format!(
                    "Failed to save bootstrap env to .env: {}",
                    e
                )))
            })?;
        }

        Ok(())
    }

    /// Persist the NEAR AI session token to the database.
    ///
    /// The session manager writes to disk during `ensure_authenticated()` but
    /// doesn't have a DB store attached during onboarding. This reads the
    /// session file from disk and stores it under the `nearai.session_token`
    /// key so the runtime's `attach_store()` finds it without fallback.
    ///
    /// Best-effort: silently ignores errors (no DB connection yet, no
    /// session file, etc.).
    async fn persist_session_to_db(&self) {
        let session_path = crate::llm::session::default_session_path();
        let data = match std::fs::read_to_string(&session_path) {
            Ok(d) if !d.trim().is_empty() => d,
            _ => return,
        };
        let value: serde_json::Value = match serde_json::from_str(&data) {
            Ok(v) => v,
            Err(_) => return,
        };

        #[cfg(feature = "postgres")]
        if let Some(ref pool) = self.db_pool {
            let store = crate::history::Store::from_pool(pool.clone());
            if let Err(e) = store
                .set_setting("default", "nearai.session_token", &value)
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
                .set_setting("default", "nearai.session_token", &value)
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
    async fn try_load_existing_settings(&mut self) {
        let loaded = false;

        #[cfg(feature = "postgres")]
        let loaded = if !loaded {
            if let Some(ref pool) = self.db_pool {
                let store = crate::history::Store::from_pool(pool.clone());
                match store.get_all_settings("default").await {
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
                match backend.get_all_settings("default").await {
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

        // Suppress unused variable warning when only one backend is compiled.
        let _ = loaded;
    }

    /// Save settings to the database and `~/.ironclaw/.env`, then print summary.
    async fn save_and_summarize(&mut self) -> Result<(), SetupError> {
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

        println!();
        print_success("Configuration saved to database");
        println!();

        // Print summary
        println!("Configuration Summary:");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        let backend = self
            .settings
            .database_backend
            .as_deref()
            .unwrap_or("postgres");
        match backend {
            "libsql" => {
                if let Some(ref path) = self.settings.libsql_path {
                    println!("  Database: libSQL ({})", path);
                } else {
                    println!("  Database: libSQL (default path)");
                }
                if self.settings.libsql_url.is_some() {
                    println!("  Turso sync: enabled");
                }
            }
            _ => {
                if self.settings.database_url.is_some() {
                    println!("  Database: PostgreSQL (configured)");
                }
            }
        }

        match self.settings.secrets_master_key_source {
            KeySource::Keychain => println!("  Security: OS keychain"),
            KeySource::Env => println!("  Security: environment variable"),
            KeySource::None => println!("  Security: disabled"),
        }

        if let Some(ref provider) = self.settings.llm_backend {
            let display = match provider.as_str() {
                "nearai" => "NEAR AI",
                "anthropic" => "Anthropic",
                "openai" => "OpenAI",
                "ollama" => "Ollama",
                "openai_compatible" => "OpenAI-compatible",
                other => other,
            };
            println!("  Provider: {}", display);
        }

        if let Some(ref model) = self.settings.selected_model {
            // Truncate long model names (char-based to avoid UTF-8 panic)
            let display = if model.chars().count() > 40 {
                let truncated: String = model.chars().take(37).collect();
                format!("{}...", truncated)
            } else {
                model.clone()
            };
            println!("  Model: {}", display);
        }

        if self.settings.embeddings.enabled {
            println!(
                "  Embeddings: {} ({})",
                self.settings.embeddings.provider, self.settings.embeddings.model
            );
        } else {
            println!("  Embeddings: disabled");
        }

        if let Some(ref tunnel_url) = self.settings.tunnel.public_url {
            println!("  Tunnel: {} (static)", tunnel_url);
        } else if let Some(ref provider) = self.settings.tunnel.provider {
            println!("  Tunnel: {} (managed, starts at boot)", provider);
        }

        let has_tunnel =
            self.settings.tunnel.public_url.is_some() || self.settings.tunnel.provider.is_some();

        println!("  Channels:");
        println!("    - CLI/TUI: enabled");

        if self.settings.channels.http_enabled {
            let port = self.settings.channels.http_port.unwrap_or(8080);
            println!("    - HTTP: enabled (port {})", port);
        }

        for channel_name in &self.settings.channels.wasm_channels {
            let mode = if has_tunnel { "webhook" } else { "polling" };
            println!(
                "    - {}: enabled ({})",
                capitalize_first(channel_name),
                mode
            );
        }

        if self.settings.heartbeat.enabled {
            println!(
                "  Heartbeat: every {} minutes",
                self.settings.heartbeat.interval_secs / 60
            );
        }

        println!();
        println!("To start the agent, run:");
        println!("  ironclaw");
        println!();
        println!("To change settings later:");
        println!("  ironclaw config set <setting> <value>");
        println!("  ironclaw onboard");
        println!();

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

/// Fetch models from the Anthropic API.
///
/// Returns `(model_id, display_label)` pairs. Falls back to static defaults on error.
async fn fetch_anthropic_models(cached_key: Option<&str>) -> Vec<(String, String)> {
    let static_defaults = vec![
        (
            "claude-opus-4-6".into(),
            "Claude Opus 4.6 (latest flagship)".into(),
        ),
        ("claude-sonnet-4-6".into(), "Claude Sonnet 4.6".into()),
        ("claude-opus-4-5".into(), "Claude Opus 4.5".into()),
        ("claude-sonnet-4-5".into(), "Claude Sonnet 4.5".into()),
        ("claude-haiku-4-5".into(), "Claude Haiku 4.5 (fast)".into()),
    ];

    let api_key = cached_key
        .map(String::from)
        .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
        .filter(|k| !k.is_empty());

    let api_key = match api_key {
        Some(k) => k,
        None => return static_defaults,
    };

    let client = reqwest::Client::new();
    let resp = match client
        .get("https://api.anthropic.com/v1/models")
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        _ => return static_defaults,
    };

    #[derive(serde::Deserialize)]
    struct ModelEntry {
        id: String,
    }
    #[derive(serde::Deserialize)]
    struct ModelsResponse {
        data: Vec<ModelEntry>,
    }

    match resp.json::<ModelsResponse>().await {
        Ok(body) => {
            let mut models: Vec<(String, String)> = body
                .data
                .into_iter()
                .filter(|m| !m.id.contains("embedding") && !m.id.contains("audio"))
                .map(|m| {
                    let label = m.id.clone();
                    (m.id, label)
                })
                .collect();
            if models.is_empty() {
                return static_defaults;
            }
            models.sort_by(|a, b| a.0.cmp(&b.0));
            models
        }
        Err(_) => static_defaults,
    }
}

/// Fetch models from the OpenAI API.
///
/// Returns `(model_id, display_label)` pairs. Falls back to static defaults on error.
async fn fetch_openai_models(cached_key: Option<&str>) -> Vec<(String, String)> {
    let static_defaults = vec![
        (
            "gpt-5.3-codex".into(),
            "GPT-5.3 Codex (latest flagship)".into(),
        ),
        ("gpt-5.2-codex".into(), "GPT-5.2 Codex".into()),
        ("gpt-5.2".into(), "GPT-5.2".into()),
        (
            "gpt-5.1-codex-mini".into(),
            "GPT-5.1 Codex Mini (fast)".into(),
        ),
        ("gpt-5".into(), "GPT-5".into()),
        ("gpt-5-mini".into(), "GPT-5 Mini".into()),
        ("gpt-4.1".into(), "GPT-4.1".into()),
        ("gpt-4.1-mini".into(), "GPT-4.1 Mini".into()),
        ("o4-mini".into(), "o4-mini (fast reasoning)".into()),
        ("o3".into(), "o3 (reasoning)".into()),
    ];

    let api_key = cached_key
        .map(String::from)
        .or_else(|| std::env::var("OPENAI_API_KEY").ok())
        .filter(|k| !k.is_empty());

    let api_key = match api_key {
        Some(k) => k,
        None => return static_defaults,
    };

    let client = reqwest::Client::new();
    let resp = match client
        .get("https://api.openai.com/v1/models")
        .bearer_auth(&api_key)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        _ => return static_defaults,
    };

    #[derive(serde::Deserialize)]
    struct ModelEntry {
        id: String,
    }
    #[derive(serde::Deserialize)]
    struct ModelsResponse {
        data: Vec<ModelEntry>,
    }

    match resp.json::<ModelsResponse>().await {
        Ok(body) => {
            let mut models: Vec<(String, String)> = body
                .data
                .into_iter()
                .filter(|m| is_openai_chat_model(&m.id))
                .map(|m| {
                    let label = m.id.clone();
                    (m.id, label)
                })
                .collect();
            if models.is_empty() {
                return static_defaults;
            }
            sort_openai_models(&mut models);
            models
        }
        Err(_) => static_defaults,
    }
}

fn is_openai_chat_model(model_id: &str) -> bool {
    let id = model_id.to_ascii_lowercase();

    let is_chat_family = id.starts_with("gpt-")
        || id.starts_with("chatgpt-")
        || id.starts_with("o1")
        || id.starts_with("o3")
        || id.starts_with("o4")
        || id.starts_with("o5");

    let is_non_chat_variant = id.contains("realtime")
        || id.contains("audio")
        || id.contains("transcribe")
        || id.contains("tts")
        || id.contains("embedding")
        || id.contains("moderation")
        || id.contains("image");

    is_chat_family && !is_non_chat_variant
}

fn openai_model_priority(model_id: &str) -> usize {
    let id = model_id.to_ascii_lowercase();

    const EXACT_PRIORITY: &[&str] = &[
        "gpt-5.3-codex",
        "gpt-5.2-codex",
        "gpt-5.2",
        "gpt-5.1-codex-mini",
        "gpt-5",
        "gpt-5-mini",
        "gpt-5-nano",
        "o4-mini",
        "o3",
        "o1",
        "gpt-4.1",
        "gpt-4.1-mini",
        "gpt-4o",
        "gpt-4o-mini",
    ];
    if let Some(pos) = EXACT_PRIORITY.iter().position(|m| id == *m) {
        return pos;
    }

    const PREFIX_PRIORITY: &[&str] = &[
        "gpt-5.", "gpt-5-", "o3-", "o4-", "o1-", "gpt-4.1-", "gpt-4o-", "gpt-3.5-", "chatgpt-",
    ];
    if let Some(pos) = PREFIX_PRIORITY
        .iter()
        .position(|prefix| id.starts_with(prefix))
    {
        return EXACT_PRIORITY.len() + pos;
    }

    EXACT_PRIORITY.len() + PREFIX_PRIORITY.len() + 1
}

fn sort_openai_models(models: &mut [(String, String)]) {
    models.sort_by(|a, b| {
        openai_model_priority(&a.0)
            .cmp(&openai_model_priority(&b.0))
            .then_with(|| a.0.cmp(&b.0))
    });
}

/// Fetch installed models from a local Ollama instance.
///
/// Returns `(model_name, display_label)` pairs. Falls back to static defaults on error.
async fn fetch_ollama_models(base_url: &str) -> Vec<(String, String)> {
    let static_defaults = vec![
        ("llama3".into(), "llama3".into()),
        ("mistral".into(), "mistral".into()),
        ("codellama".into(), "codellama".into()),
    ];

    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let client = reqwest::Client::new();

    let resp = match client
        .get(&url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        Ok(_) => return static_defaults,
        Err(_) => {
            print_info("Could not connect to Ollama. Is it running?");
            return static_defaults;
        }
    };

    #[derive(serde::Deserialize)]
    struct ModelEntry {
        name: String,
    }
    #[derive(serde::Deserialize)]
    struct TagsResponse {
        models: Vec<ModelEntry>,
    }

    match resp.json::<TagsResponse>().await {
        Ok(body) => {
            let models: Vec<(String, String)> = body
                .models
                .into_iter()
                .map(|m| {
                    let label = m.name.clone();
                    (m.name, label)
                })
                .collect();
            if models.is_empty() {
                return static_defaults;
            }
            models
        }
        Err(_) => static_defaults,
    }
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
            dirs::home_dir().unwrap_or_default().join(".ironclaw/tools"),
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

    use tempfile::tempdir;

    use super::*;

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
        };
        let wizard = SetupWizard::with_config(config);
        assert!(wizard.config.skip_auth);
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

        let dir = tempdir().unwrap();
        let installed = HashSet::<String>::new();

        install_missing_bundled_channels(dir.path(), &installed)
            .await
            .unwrap();

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
        let dir = tempdir().unwrap();
        let channels = discover_wasm_channels(dir.path()).await;
        assert!(channels.is_empty());
    }

    #[tokio::test]
    async fn test_discover_wasm_channels_nonexistent_dir() {
        let channels =
            discover_wasm_channels(std::path::Path::new("/tmp/ironclaw_nonexistent_dir")).await;
        assert!(channels.is_empty());
    }

    /// RAII guard that sets/clears an env var for the duration of a test.
    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
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
}
