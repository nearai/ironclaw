//! Channel-specific setup flows.
//!
//! Each channel (Telegram, HTTP, etc.) has its own setup function that:
//! 1. Displays setup instructions
//! 2. Collects configuration (tokens, ports, etc.)
//! 3. Validates the configuration
//! 4. Saves secrets to the database

use std::sync::Arc;

use reqwest::Client;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use crate::secrets::{CreateSecretParams, PostgresSecretsStore, SecretsCrypto, SecretsStore};
use crate::setup::prompts::{
    confirm, optional_input, print_error, print_info, print_success, secret_input,
};

/// Context for saving secrets during setup.
pub struct SecretsContext {
    store: PostgresSecretsStore,
    user_id: String,
}

impl SecretsContext {
    /// Create a new secrets context.
    pub fn new(pool: deadpool_postgres::Pool, crypto: Arc<SecretsCrypto>, user_id: &str) -> Self {
        Self {
            store: PostgresSecretsStore::new(pool, crypto),
            user_id: user_id.to_string(),
        }
    }

    /// Save a secret to the database.
    pub async fn save_secret(&self, name: &str, value: &SecretString) -> Result<(), String> {
        let params = CreateSecretParams::new(name, value.expose_secret());

        self.store
            .create(&self.user_id, params)
            .await
            .map_err(|e| format!("Failed to save secret: {}", e))?;

        Ok(())
    }

    /// Check if a secret exists.
    pub async fn secret_exists(&self, name: &str) -> bool {
        self.store
            .exists(&self.user_id, name)
            .await
            .unwrap_or(false)
    }
}

/// Result of Telegram setup.
#[derive(Debug, Clone)]
pub struct TelegramSetupResult {
    pub enabled: bool,
    pub bot_username: Option<String>,
}

/// Telegram Bot API response for getMe.
#[derive(Debug, Deserialize)]
struct TelegramGetMeResponse {
    ok: bool,
    result: Option<TelegramUser>,
}

#[derive(Debug, Deserialize)]
struct TelegramUser {
    username: Option<String>,
    #[allow(dead_code)]
    first_name: String,
}

/// Set up Telegram bot channel.
///
/// Guides the user through:
/// 1. Creating a bot with @BotFather
/// 2. Entering the bot token
/// 3. Validating the token
/// 4. Saving the token to the database
pub async fn setup_telegram(secrets: &SecretsContext) -> Result<TelegramSetupResult, String> {
    println!("Telegram Setup:");
    println!();
    print_info("To create a Telegram bot:");
    print_info("1. Open Telegram and message @BotFather");
    print_info("2. Send /newbot and follow the prompts");
    print_info("3. Copy the bot token (looks like 123456:ABC-DEF...)");
    println!();

    // Check if token already exists
    if secrets.secret_exists("telegram_bot_token").await {
        print_info("Existing Telegram token found in database.");
        if !confirm("Replace existing token?", false).map_err(|e| e.to_string())? {
            return Ok(TelegramSetupResult {
                enabled: true,
                bot_username: None,
            });
        }
    }

    let token = secret_input("Bot token (from @BotFather)").map_err(|e| e.to_string())?;

    // Validate the token
    print_info("Validating bot token...");

    match validate_telegram_token(&token).await {
        Ok(username) => {
            print_success(&format!(
                "Bot validated: @{}",
                username.as_deref().unwrap_or("unknown")
            ));

            // Save to database
            secrets.save_secret("telegram_bot_token", &token).await?;
            print_success("Token saved to database");

            Ok(TelegramSetupResult {
                enabled: true,
                bot_username: username,
            })
        }
        Err(e) => {
            print_error(&format!("Token validation failed: {}", e));

            if confirm("Try again?", true).map_err(|e| e.to_string())? {
                Box::pin(setup_telegram(secrets)).await
            } else {
                Ok(TelegramSetupResult {
                    enabled: false,
                    bot_username: None,
                })
            }
        }
    }
}

/// Validate a Telegram bot token by calling the getMe API.
///
/// Returns the bot's username if valid.
pub async fn validate_telegram_token(token: &SecretString) -> Result<Option<String>, String> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let url = format!(
        "https://api.telegram.org/bot{}/getMe",
        token.expose_secret()
    );

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("API returned status {}", response.status()));
    }

    let body: TelegramGetMeResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    if body.ok {
        Ok(body.result.and_then(|u| u.username))
    } else {
        Err("Telegram API returned error".to_string())
    }
}

/// Result of HTTP webhook setup.
#[derive(Debug, Clone)]
pub struct HttpSetupResult {
    pub enabled: bool,
    pub port: u16,
    pub host: String,
}

/// Set up HTTP webhook channel.
pub async fn setup_http(secrets: &SecretsContext) -> Result<HttpSetupResult, String> {
    println!("HTTP Webhook Setup:");
    println!();
    print_info("The HTTP webhook allows external services to send messages to the agent.");
    println!();

    let port_str = optional_input("Port", Some("default: 8080")).map_err(|e| e.to_string())?;
    let port: u16 = port_str
        .as_deref()
        .unwrap_or("8080")
        .parse()
        .map_err(|e| format!("Invalid port: {}", e))?;

    if port < 1024 {
        print_info("Note: Ports below 1024 may require root privileges");
    }

    let host = optional_input("Host", Some("default: 0.0.0.0"))
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| "0.0.0.0".to_string());

    // Generate a webhook secret
    if confirm("Generate a webhook secret for authentication?", true).map_err(|e| e.to_string())? {
        let secret = generate_webhook_secret();
        secrets
            .save_secret("http_webhook_secret", &SecretString::from(secret.clone()))
            .await?;
        print_success("Webhook secret generated and saved to database");
        print_info(&format!(
            "Secret: {} (store this for your webhook clients)",
            secret
        ));
    }

    print_success(&format!("HTTP webhook will listen on {}:{}", host, port));

    Ok(HttpSetupResult {
        enabled: true,
        port,
        host,
    })
}

/// Generate a random webhook secret.
fn generate_webhook_secret() -> String {
    use rand::RngCore;
    let mut rng = rand::thread_rng();
    let mut bytes = [0u8; 32];
    rng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_webhook_secret() {
        let secret = generate_webhook_secret();
        assert_eq!(secret.len(), 64); // 32 bytes = 64 hex chars
    }
}
