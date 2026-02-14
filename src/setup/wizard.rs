//! Main setup wizard orchestration.
//!
//! The wizard guides users through:
//! 1. Database connection
//! 2. Security (secrets master key)
//! 3. NEAR AI authentication
//! 4. Model selection
//! 5. Embeddings
//! 6. Channel configuration
//! 7. Heartbeat (background tasks)

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[cfg(feature = "postgres")]
use deadpool_postgres::{Config as PoolConfig, Runtime};
use secrecy::SecretString;
#[cfg(feature = "postgres")]
use tokio_postgres::NoTls;

use crate::channels::wasm::{
    ChannelCapabilitiesFile, available_channel_names, install_bundled_channel,
};
use crate::llm::{SessionConfig, SessionManager};
use crate::secrets::{SecretsCrypto, SecretsStore};
use crate::settings::{KeySource, Settings};
use crate::setup::channels::{
    SecretsContext, setup_http, setup_telegram, setup_tunnel, setup_wasm_channel,
};
use crate::setup::prompts::{
    confirm, input, optional_input, print_error, print_header, print_info, print_step,
    print_success, select_many, select_one,
};

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
    db_backend: Option<crate::db::libsql_backend::LibSqlBackend>,
    /// Secrets crypto (created during setup).
    secrets_crypto: Option<Arc<SecretsCrypto>>,
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
        }
    }

    /// Set the session manager (for reusing existing auth).
    pub fn with_session(mut self, session: Arc<SessionManager>) -> Self {
        self.session_manager = Some(session);
        self
    }

    /// Run the setup wizard.
    pub async fn run(&mut self) -> Result<(), SetupError> {
        print_header("IronClaw Setup Wizard");

        if self.config.channels_only {
            // Channels-only mode: just step 6
            print_step(1, 1, "Channel Configuration");
            self.step_channels().await?;
        } else {
            let total_steps = 7;

            // Step 1: Database
            print_step(1, total_steps, "Database Connection");
            self.step_database().await?;

            // Step 2: Security
            print_step(2, total_steps, "Security");
            self.step_security().await?;

            // Step 3: Authentication (unless skipped)
            if !self.config.skip_auth {
                print_step(3, total_steps, "NEAR AI Authentication");
                self.step_authentication().await?;
            } else {
                print_info("Skipping authentication (using existing session)");
            }

            // Step 4: Model selection
            print_step(4, total_steps, "Model Selection");
            self.step_model_selection().await?;

            // Step 5: Embeddings
            print_step(5, total_steps, "Embeddings (Semantic Search)");
            self.step_embeddings()?;

            // Step 6: Channel configuration
            print_step(6, total_steps, "Channel Configuration");
            self.step_channels().await?;

            // Step 7: Heartbeat
            print_step(7, total_steps, "Background Tasks");
            self.step_heartbeat()?;
        }

        // Save settings and print summary
        self.save_and_summarize().await?;

        Ok(())
    }

    /// Step 1: Database connection.
    async fn step_database(&mut self) -> Result<(), SetupError> {
        // Determine which backend to use based on compile-time features.
        // When both features are enabled, prefer the currently configured backend
        // or default to postgres.
        #[cfg(all(feature = "postgres", feature = "libsql"))]
        {
            let backend = std::env::var("DATABASE_BACKEND")
                .ok()
                .or_else(|| self.settings.database_backend.clone())
                .unwrap_or_else(|| "postgres".to_string());

            if backend == "libsql" || backend == "turso" || backend == "sqlite" {
                return self.step_database_libsql().await;
            }
            return self.step_database_postgres().await;
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
                let token = input("Auth token").map_err(SetupError::Io)?;
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
        use crate::db::libsql_backend::LibSqlBackend;
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
        let keychain_key_exists = crate::secrets::keychain::has_master_key().await;

        if env_key_exists {
            print_info("Secrets master key found in SECRETS_MASTER_KEY environment variable.");
            self.settings.secrets_master_key_source = KeySource::Env;
            print_success("Security configured (env var)");
            return Ok(());
        }

        if keychain_key_exists {
            print_info("Existing master key found in OS keychain.");
            if confirm("Use existing keychain key?", true).map_err(SetupError::Io)? {
                self.settings.secrets_master_key_source = KeySource::Keychain;
                print_success("Security configured (keychain)");
                return Ok(());
            }
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

    /// Step 3: NEAR AI authentication.
    async fn step_authentication(&mut self) -> Result<(), SetupError> {
        // Check if we already have a session
        if let Some(ref session) = self.session_manager
            && session.has_token().await
        {
            print_info("Existing session found. Validating...");
            match session.ensure_authenticated().await {
                Ok(()) => {
                    print_success("Session valid");
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
        Ok(())
    }

    /// Step 4: Model selection.
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

        // Try to fetch available models
        let models = if let Some(ref session) = self.session_manager {
            self.fetch_available_models(session).await
        } else {
            vec![]
        };

        // Default models if we couldn't fetch
        let default_models = [
            (
                "fireworks::accounts/fireworks/models/llama4-maverick-instruct-basic",
                "Llama 4 Maverick (default, fast)",
            ),
            (
                "anthropic::claude-sonnet-4-20250514",
                "Claude Sonnet 4 (best quality)",
            ),
            ("openai::gpt-4o", "GPT-4o"),
        ];

        println!("Available models:");
        println!();

        let options: Vec<&str> = if models.is_empty() {
            default_models.iter().map(|(_, desc)| *desc).collect()
        } else {
            models.iter().map(|m| m.as_str()).collect()
        };

        // Add custom option
        let mut all_options = options.clone();
        all_options.push("Custom model ID");

        let choice = select_one("Select a model:", &all_options).map_err(SetupError::Io)?;

        let selected_model = if choice == all_options.len() - 1 {
            // Custom model
            input("Enter model ID").map_err(SetupError::Io)?
        } else if models.is_empty() {
            default_models[choice].0.to_string()
        } else {
            models[choice].clone()
        };

        self.settings.selected_model = Some(selected_model.clone());
        print_success(&format!("Selected {}", selected_model));

        Ok(())
    }

    /// Fetch available models from the API.
    async fn fetch_available_models(&self, session: &Arc<SessionManager>) -> Vec<String> {
        use crate::config::LlmConfig;
        use crate::llm::create_llm_provider;

        let base_url = std::env::var("NEARAI_BASE_URL")
            .unwrap_or_else(|_| "https://cloud-api.near.ai".to_string());
        let auth_base_url = std::env::var("NEARAI_AUTH_URL")
            .unwrap_or_else(|_| "https://private.near.ai".to_string());

        let config = LlmConfig {
            backend: crate::config::LlmBackend::NearAi,
            nearai: crate::config::NearAiConfig {
                model: "dummy".to_string(),
                base_url,
                auth_base_url,
                session_path: crate::llm::session::default_session_path(),
                api_mode: crate::config::NearAiApiMode::Responses,
                api_key: None,
                fallback_model: None,
                max_retries: 3,
            },
            openai: None,
            anthropic: None,
            ollama: None,
            openai_compatible: None,
        };

        match create_llm_provider(&config, Arc::clone(session)) {
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

        let options = [
            "NEAR AI (uses same auth, no extra cost)",
            "OpenAI (requires API key)",
        ];

        let choice = select_one("Select embeddings provider:", &options).map_err(SetupError::Io)?;

        match choice {
            0 => {
                self.settings.embeddings.enabled = true;
                self.settings.embeddings.provider = "nearai".to_string();
                self.settings.embeddings.model = "text-embedding-3-small".to_string();
                print_success("Embeddings enabled via NEAR AI");
            }
            1 => {
                // Check if API key is set
                if std::env::var("OPENAI_API_KEY").is_err() {
                    print_info("OPENAI_API_KEY not set in environment.");
                    print_info("Add it to your .env file or environment to enable embeddings.");
                }
                self.settings.embeddings.enabled = true;
                self.settings.embeddings.provider = "openai".to_string();
                self.settings.embeddings.model = "text-embedding-3-small".to_string();
                print_success("Embeddings configured for OpenAI");
            }
            _ => unreachable!(),
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

            let crypto = SecretsCrypto::new(SecretString::from(key))
                .map_err(|e| SetupError::Config(e.to_string()))?;
            self.secrets_crypto = Some(Arc::new(crypto));
            Arc::clone(self.secrets_crypto.as_ref().unwrap())
        };

        // Create backend-appropriate secrets store
        #[cfg(feature = "postgres")]
        {
            // Try postgres path first when postgres feature is available
            if let Some(store) = self.create_postgres_secrets_store(&crypto).await? {
                return Ok(SecretsContext::from_store(store, "default"));
            }
        }

        #[cfg(feature = "libsql")]
        {
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
                self.db_pool.clone().unwrap()
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
            Ok(Some(url)) => {
                self.settings.tunnel.public_url = Some(url);
            }
            Ok(None) => {
                self.settings.tunnel.public_url = None;
            }
            Err(e) => {
                print_info(&format!("Tunnel setup skipped: {}", e));
            }
        }
        println!();

        // Discover available WASM channels
        let channels_dir = dirs::home_dir()
            .unwrap_or_default()
            .join(".ironclaw/channels");

        let mut discovered_channels = discover_wasm_channels(&channels_dir).await;
        let installed_names: HashSet<String> = discovered_channels
            .iter()
            .map(|(name, _)| name.clone())
            .collect();
        let wasm_channel_names = wasm_channel_option_names(&discovered_channels);

        // Build options list dynamically
        let mut options: Vec<(String, bool)> = vec![
            ("CLI/TUI (always enabled)".to_string(), true),
            (
                "HTTP webhook".to_string(),
                self.settings.channels.http_enabled,
            ),
        ];

        // Add available WASM channels (installed + bundled)
        for name in &wasm_channel_names {
            let is_enabled = self.settings.channels.wasm_channels.contains(name);
            let display_name = format!("{} (WASM)", capitalize_first(name));
            options.push((display_name, is_enabled));
        }

        let options_refs: Vec<(&str, bool)> =
            options.iter().map(|(s, b)| (s.as_str(), *b)).collect();

        let selected = select_many("Which channels do you want to enable?", &options_refs)
            .map_err(SetupError::Io)?;

        let selected_wasm_channels: Vec<String> = wasm_channel_names
            .iter()
            .enumerate()
            .filter_map(|(idx, name)| {
                if selected.contains(&(idx + 2)) {
                    Some(name.clone())
                } else {
                    None
                }
            })
            .collect();

        if let Some(installed) = install_selected_bundled_channels(
            &channels_dir,
            &selected_wasm_channels,
            &installed_names,
        )
        .await?
            && !installed.is_empty()
        {
            print_success(&format!("Installed channels: {}", installed.join(", ")));
            discovered_channels = discover_wasm_channels(&channels_dir).await;
        }

        // Determine if we need secrets context
        let needs_secrets = selected.contains(&1) || !selected_wasm_channels.is_empty();
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

        // HTTP is index 1
        if selected.contains(&1) {
            println!();
            if let Some(ref ctx) = secrets {
                let result = setup_http(ctx).await.map_err(SetupError::Channel)?;
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

        let discovered_by_name: HashMap<String, ChannelCapabilitiesFile> =
            discovered_channels.into_iter().collect();

        // Process selected WASM channels
        let mut enabled_wasm_channels = Vec::new();
        for channel_name in selected_wasm_channels {
            println!();
            if let Some(ref ctx) = secrets {
                let result = if let Some(cap_file) = discovered_by_name.get(&channel_name) {
                    if !cap_file.setup.required_secrets.is_empty() {
                        setup_wasm_channel(ctx, &channel_name, &cap_file.setup)
                            .await
                            .map_err(SetupError::Channel)?
                    } else if channel_name == "telegram" {
                        let telegram_result = setup_telegram(ctx, &self.settings)
                            .await
                            .map_err(SetupError::Channel)?;
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

    /// Step 7: Heartbeat configuration.
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

    /// Save settings to the database and `~/.ironclaw/.env`, then print summary.
    async fn save_and_summarize(&mut self) -> Result<(), SetupError> {
        self.settings.onboard_completed = true;

        // Write all settings to the database (whichever backend is active).
        {
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
                            SetupError::Database(format!(
                                "Failed to save settings to database: {}",
                                e
                            ))
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
                    use crate::db::Database as _;
                    backend
                        .set_all_settings("default", &db_map)
                        .await
                        .map_err(|e| {
                            SetupError::Database(format!(
                                "Failed to save settings to database: {}",
                                e
                            ))
                        })?;
                    true
                } else {
                    false
                }
            } else {
                saved
            };

            if !saved {
                return Err(SetupError::Database(
                    "No database connection, cannot save settings".to_string(),
                ));
            }
        }

        // Save DATABASE_URL to ~/.ironclaw/.env (the only field that needs
        // disk persistence before the DB is available).
        if let Some(ref url) = self.settings.database_url {
            crate::bootstrap::save_database_url(url).map_err(|e| {
                SetupError::Io(std::io::Error::other(format!(
                    "Failed to save DATABASE_URL to .env: {}",
                    e
                )))
            })?;
        }

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

        if let Some(ref model) = self.settings.selected_model {
            // Truncate long model names
            let display = if model.len() > 40 {
                format!("{}...", &model[..37])
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
            println!("  Tunnel: {}", tunnel_url);
        }

        println!("  Channels:");
        println!("    - CLI/TUI: enabled");

        if self.settings.channels.http_enabled {
            let port = self.settings.channels.http_port.unwrap_or(8080);
            println!("    - HTTP: enabled (port {})", port);
        }

        for channel_name in &self.settings.channels.wasm_channels {
            let mode = if self.settings.tunnel.public_url.is_some() {
                "webhook"
            } else {
                "polling"
            };
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
        let extension = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        if !extension.ends_with(".capabilities.json") {
            continue;
        }

        // Extract channel name
        let name = extension.trim_end_matches(".capabilities.json").to_string();
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

fn wasm_channel_option_names(discovered: &[(String, ChannelCapabilitiesFile)]) -> Vec<String> {
    let mut names: Vec<String> = discovered.iter().map(|(name, _)| name.clone()).collect();

    for bundled in available_channel_names().iter().copied() {
        if !names.iter().any(|name| name == bundled) {
            names.push(bundled.to_string());
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

    #[tokio::test]
    async fn test_install_missing_bundled_channels_installs_telegram() {
        use crate::channels::wasm::available_channel_names;

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
    fn test_wasm_channel_option_names_includes_available_when_missing() {
        let discovered = Vec::new();
        let options = wasm_channel_option_names(&discovered);
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
    fn test_wasm_channel_option_names_dedupes_available() {
        let discovered = vec![(String::from("telegram"), ChannelCapabilitiesFile::default())];
        let options = wasm_channel_option_names(&discovered);
        // telegram should appear exactly once despite being both discovered and available
        assert_eq!(
            options.iter().filter(|n| *n == "telegram").count(),
            1,
            "telegram should not be duplicated"
        );
    }
}
