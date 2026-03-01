//! Secret management CLI commands.

use std::sync::Arc;
use clap::Subcommand;
use crate::secrets::{SecretsStore, CreateSecretParams};
use secrecy::SecretString;

#[derive(Subcommand, Debug, Clone)]
pub enum SecretCommand {
    /// List all secrets
    List,
    /// Set a secret value
    Set {
        /// Secret name (e.g., "telegram_bot_token")
        name: String,
        /// Value to set
        value: String,
    },
    /// Get a secret metadata
    Get {
        /// Secret name
        name: String,
    },
    /// Remove a secret
    Remove {
        /// Secret name
        name: String,
    },
}

pub async fn run_secret_command(cmd: SecretCommand) -> anyhow::Result<()> {
    let secrets = get_secrets_store().await?;
    let user_id = "default";

    match cmd {
        SecretCommand::List => {
            let all = secrets.list(user_id).await
                .map_err(|e| anyhow::anyhow!("Failed to list secrets: {}", e))?;
            println!("Secrets for user '{}':", user_id);
            if all.is_empty() {
                println!("  (No secrets found)");
            }
            for s in all {
                println!("  - {} (provider: {:?})", s.name, s.provider);
            }
        }
        SecretCommand::Set { name, value } => {
            secrets.create(user_id, CreateSecretParams::new(&name, &value)).await
                .map_err(|e| anyhow::anyhow!("Failed to set secret: {}", e))?;
            println!("Secret '{}' set successfully.", name);
        }
        SecretCommand::Get { name } => {
            match secrets.get(user_id, &name).await {
                Ok(s) => {
                    println!("Secret '{}':", name);
                    println!("  ID:         {}", s.id);
                    println!("  Provider:   {:?}", s.provider);
                    println!("  Created at: {}", s.created_at);
                    println!("  Updated at: {}", s.updated_at);
                    println!("  Expires at: {:?}", s.expires_at);
                    println!("  Usage count: {}", s.usage_count);
                    println!("  Last used:  {:?}", s.last_used_at);
                    println!("  (Value is encrypted and not shown)");
                }
                Err(e) => anyhow::bail!("Secret '{}' not found: {}", name, e),
            }
        }
        SecretCommand::Remove { name } => {
            secrets.delete(user_id, &name).await
                .map_err(|e| anyhow::anyhow!("Failed to delete secret: {}", e))?;
            println!("Secret '{}' removed.", name);
        }
    }

    Ok(())
}

async fn get_secrets_store() -> anyhow::Result<Arc<dyn SecretsStore + Send + Sync>> {
    let config = crate::config::Config::from_env().await
        .map_err(|e| anyhow::anyhow!("Failed to load config: {}", e))?;
        
    let master_key = config.secrets.master_key().ok_or_else(|| {
        anyhow::anyhow!("Secrets master key not configured. Set SECRETS_MASTER_KEY.")
    })?;

    let crypto = Arc::new(crate::secrets::SecretsCrypto::new(master_key.clone()).map_err(|e| anyhow::anyhow!("Failed to initialize crypto: {}", e))?);

    match config.database.backend {
        #[cfg(feature = "postgres")]
        crate::config::DatabaseBackend::Postgres => {
            let pg = crate::db::postgres::PgBackend::new(&config.database).await
                .map_err(|e| anyhow::anyhow!("Postgres connect failed: {}", e))?;
            Ok(Arc::new(crate::secrets::PostgresSecretsStore::new(pg.pool(), crypto)))
        }
        #[cfg(feature = "libsql")]
        crate::config::DatabaseBackend::LibSql => {
             use crate::db::libsql::LibSqlBackend;
             use secrecy::ExposeSecret as _;
             let default_path = crate::config::default_libsql_path();
             let db_path = config.database.libsql_path.as_deref().unwrap_or(&default_path);
             
             let backend = if let Some(ref url) = config.database.libsql_url {
                 let token = config.database.libsql_auth_token.as_ref().ok_or_else(|| anyhow::anyhow!("token missing"))?;
                 LibSqlBackend::new_remote_replica(db_path, url, token.expose_secret()).await?
             } else {
                 LibSqlBackend::new_local(db_path).await?
             };
             Ok(Arc::new(crate::secrets::LibSqlSecretsStore::new(backend.shared_db(), crypto)))
        }
        _ => anyhow::bail!("Database backend not supported for secrets in CLI"),
    }
}
