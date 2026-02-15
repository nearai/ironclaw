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

#[cfg(feature = "postgres")]
use crate::secrets::SecretsCrypto;
use crate::secrets::{CreateSecretParams, SecretsStore};
use crate::settings::{Settings, TunnelSettings};
use crate::setup::prompts::{
    confirm, input, optional_input, print_error, print_info, print_success, secret_input,
};

/// Context for saving secrets during setup.
pub struct SecretsContext {
    store: Arc<dyn SecretsStore>,
    user_id: String,
}

impl SecretsContext {
    /// Create a new secrets context from a trait-object store.
    pub fn from_store(store: Arc<dyn SecretsStore>, user_id: &str) -> Self {
        Self {
            store,
            user_id: user_id.to_string(),
        }
    }

    /// Create a new secrets context from a PostgreSQL pool and crypto.
    #[cfg(feature = "postgres")]
    pub fn new(pool: deadpool_postgres::Pool, crypto: Arc<SecretsCrypto>, user_id: &str) -> Self {
        Self {
            store: Arc::new(crate::secrets::PostgresSecretsStore::new(pool, crypto)),
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

    /// Read a secret from the database (decrypted).
    pub async fn get_secret(&self, name: &str) -> Result<SecretString, String> {
        let decrypted = self
            .store
            .get_decrypted(&self.user_id, name)
            .await
            .map_err(|e| format!("Failed to read secret: {}", e))?;
        Ok(SecretString::from(decrypted.expose().to_string()))
    }
}

/// Result of Telegram setup.
#[derive(Debug, Clone)]
pub struct TelegramSetupResult {
    pub enabled: bool,
    pub bot_username: Option<String>,
    pub webhook_secret: Option<String>,
    pub owner_id: Option<i64>,
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

/// Telegram Bot API response for getUpdates.
#[derive(Debug, Deserialize)]
struct TelegramGetUpdatesResponse {
    ok: bool,
    result: Vec<TelegramUpdate>,
}

#[derive(Debug, Deserialize)]
struct TelegramUpdate {
    #[allow(dead_code)]
    update_id: i64,
    message: Option<TelegramUpdateMessage>,
}

#[derive(Debug, Deserialize)]
struct TelegramUpdateMessage {
    from: Option<TelegramUpdateUser>,
}

#[derive(Debug, Deserialize)]
struct TelegramUpdateUser {
    id: i64,
    first_name: String,
    username: Option<String>,
}

/// Set up Telegram bot channel.
///
/// Guides the user through:
/// 1. Creating a bot with @BotFather
/// 2. Entering the bot token
/// 3. Validating the token
/// 4. Saving the token to the database
pub async fn setup_telegram(
    secrets: &SecretsContext,
    settings: &Settings,
) -> Result<TelegramSetupResult, String> {
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
            // Still offer to configure webhook secret and owner binding
            let webhook_secret = setup_telegram_webhook_secret(secrets, &settings.tunnel).await?;
            let owner_id = bind_telegram_owner_flow(secrets, settings).await?;
            return Ok(TelegramSetupResult {
                enabled: true,
                bot_username: None,
                webhook_secret,
                owner_id,
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

            // Bind bot to owner's Telegram account
            let owner_id = bind_telegram_owner(&token).await?;

            // Offer webhook secret configuration
            let webhook_secret = setup_telegram_webhook_secret(secrets, &settings.tunnel).await?;

            Ok(TelegramSetupResult {
                enabled: true,
                bot_username: username,
                webhook_secret,
                owner_id,
            })
        }
        Err(e) => {
            print_error(&format!("Token validation failed: {}", e));

            if confirm("Try again?", true).map_err(|e| e.to_string())? {
                Box::pin(setup_telegram(secrets, settings)).await
            } else {
                Ok(TelegramSetupResult {
                    enabled: false,
                    bot_username: None,
                    webhook_secret: None,
                    owner_id: None,
                })
            }
        }
    }
}

/// Bind the bot to the owner's Telegram account by having them send a message.
///
/// Polls `getUpdates` until a message arrives, then captures the sender's user ID.
/// Returns `None` if the user declines or the flow times out.
async fn bind_telegram_owner(token: &SecretString) -> Result<Option<i64>, String> {
    println!();
    print_info("Account Binding (recommended):");
    print_info("Binding restricts the bot so only YOU can use it.");
    print_info("Without this, anyone who finds your bot can send it messages.");
    println!();

    if !confirm("Bind bot to your Telegram account?", true).map_err(|e| e.to_string())? {
        print_info("Skipping account binding. Bot will accept messages from all users.");
        return Ok(None);
    }

    print_info("Send any message (e.g. /start) to your bot in Telegram.");
    print_info("Waiting for your message (up to 120 seconds)...");

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(35))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    // Clear any existing webhook so getUpdates works
    let delete_url = format!(
        "https://api.telegram.org/bot{}/deleteWebhook",
        token.expose_secret()
    );
    let _ = client.post(&delete_url).send().await;

    let updates_url = format!(
        "https://api.telegram.org/bot{}/getUpdates",
        token.expose_secret()
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);

    while std::time::Instant::now() < deadline {
        let response = client
            .get(&updates_url)
            .query(&[("timeout", "30"), ("allowed_updates", "[\"message\"]")])
            .send()
            .await
            .map_err(|e| format!("getUpdates request failed: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("getUpdates returned status {}", response.status()));
        }

        let body: TelegramGetUpdatesResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse getUpdates response: {}", e))?;

        if !body.ok {
            return Err("Telegram API returned error for getUpdates".to_string());
        }

        // Find the first message with a sender
        for update in &body.result {
            if let Some(ref msg) = update.message
                && let Some(ref from) = msg.from
            {
                let display_name = from
                    .username
                    .as_ref()
                    .map(|u| format!("@{}", u))
                    .unwrap_or_else(|| from.first_name.clone());

                print_success(&format!(
                    "Received message from {} (ID: {})",
                    display_name, from.id
                ));

                // Acknowledge the update so it doesn't pile up
                let ack_url = format!(
                    "https://api.telegram.org/bot{}/getUpdates",
                    token.expose_secret()
                );
                let _ = client
                    .get(&ack_url)
                    .query(&[("offset", &(update.update_id + 1).to_string())])
                    .send()
                    .await;

                return Ok(Some(from.id));
            }
        }
    }

    print_error("Timed out waiting for a message. You can re-run setup to try again.");
    print_info("Bot will accept messages from all users until owner is bound.");
    Ok(None)
}

/// Bind flow when the token already exists (reads from secrets store).
///
/// Retrieves the saved bot token and delegates to `bind_telegram_owner`.
async fn bind_telegram_owner_flow(
    secrets: &SecretsContext,
    settings: &Settings,
) -> Result<Option<i64>, String> {
    if settings.channels.telegram_owner_id.is_some() {
        print_info("Bot is already bound to a Telegram account.");
        if !confirm("Re-bind to a different account?", false).map_err(|e| e.to_string())? {
            return Ok(settings.channels.telegram_owner_id);
        }
    }

    // We need the token to poll getUpdates
    let token = secrets.get_secret("telegram_bot_token").await?;

    bind_telegram_owner(&token).await
}

/// Set up a tunnel for exposing the agent to the internet.
///
/// This is shared across all channels that need webhook endpoints.
/// Returns the tunnel URL if configured.
pub fn setup_tunnel(settings: &Settings) -> Result<Option<String>, String> {
    if let Some(ref url) = settings.tunnel.public_url {
        print_info(&format!("Existing tunnel configured: {}", url));
        if !confirm("Change tunnel configuration?", false).map_err(|e| e.to_string())? {
            return Ok(Some(url.clone()));
        }
    }

    println!();
    print_info("Tunnel Configuration (for webhook endpoints):");
    print_info("A tunnel exposes your local agent to the internet, enabling:");
    print_info("  - Instant Telegram message delivery (instead of polling)");
    print_info("  - Future: Slack, Discord, GitHub webhooks");
    print_info("");
    print_info("Supported tunnel providers:");
    print_info("  - ngrok: ngrok http 8080");
    print_info("  - Cloudflare: cloudflared tunnel --url http://localhost:8080");
    print_info("  - localtunnel: lt --port 8080");
    print_info("");
    print_info("Security note: Webhook endpoints don't use tunnel-level auth.");
    print_info("Security comes from provider-specific secrets (e.g., Telegram webhook secret).");
    println!();

    if !confirm("Configure a tunnel?", false).map_err(|e| e.to_string())? {
        return Ok(None);
    }

    let tunnel_url =
        input("Tunnel URL (e.g., https://abc123.ngrok.io)").map_err(|e| e.to_string())?;

    // Validate URL format
    if !tunnel_url.starts_with("https://") {
        print_error("URL must start with https:// (webhooks require HTTPS)");
        return Err("Invalid tunnel URL: must use HTTPS".to_string());
    }

    // Remove trailing slash if present
    let tunnel_url = tunnel_url.trim_end_matches('/').to_string();

    print_success(&format!("Tunnel URL configured: {}", tunnel_url));
    print_info("");
    print_info("Make sure your tunnel is running before starting the agent.");
    print_info("You can also set TUNNEL_URL environment variable to override.");

    Ok(Some(tunnel_url))
}

/// Set up Telegram webhook secret for signature validation.
///
/// Returns the webhook secret if configured.
async fn setup_telegram_webhook_secret(
    secrets: &SecretsContext,
    tunnel: &TunnelSettings,
) -> Result<Option<String>, String> {
    if tunnel.public_url.is_none() {
        print_info("");
        print_info("No tunnel configured. Telegram will use polling mode (30s+ delay).");
        print_info("Run setup again to configure a tunnel for instant delivery.");
        return Ok(None);
    }

    println!();
    print_info("Telegram Webhook Security:");
    print_info("A webhook secret adds an extra layer of security by validating");
    print_info("that requests actually come from Telegram's servers.");

    if !confirm("Generate a webhook secret?", true).map_err(|e| e.to_string())? {
        return Ok(None);
    }

    let secret = generate_webhook_secret();
    secrets
        .save_secret(
            "telegram_webhook_secret",
            &SecretString::from(secret.clone()),
        )
        .await?;
    print_success("Webhook secret generated and saved");

    Ok(Some(secret))
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
pub fn generate_webhook_secret() -> String {
    use rand::RngCore;
    let mut rng = rand::thread_rng();
    let mut bytes = [0u8; 32];
    rng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Result of WASM channel setup.
#[derive(Debug, Clone)]
pub struct WasmChannelSetupResult {
    pub enabled: bool,
    pub channel_name: String,
}

/// Set up a WASM channel using its capabilities file setup schema.
///
/// Reads setup requirements from the channel's capabilities file and
/// prompts the user for each required secret.
pub async fn setup_wasm_channel(
    secrets: &SecretsContext,
    channel_name: &str,
    setup: &crate::channels::wasm::SetupSchema,
) -> Result<WasmChannelSetupResult, String> {
    println!("{} Setup:", channel_name);
    println!();

    for secret_config in &setup.required_secrets {
        // Check if this secret already exists
        if secrets.secret_exists(&secret_config.name).await {
            print_info(&format!(
                "Existing {} found in database.",
                secret_config.name
            ));
            if !confirm("Replace existing value?", false).map_err(|e| e.to_string())? {
                continue;
            }
        }

        // Get the value from user or auto-generate
        let value = if secret_config.optional {
            let input_value =
                optional_input(&secret_config.prompt, Some("leave empty to auto-generate"))
                    .map_err(|e| e.to_string())?;

            if let Some(v) = input_value {
                if !v.is_empty() {
                    SecretString::from(v)
                } else if let Some(ref auto_gen) = secret_config.auto_generate {
                    let generated = generate_secret_with_length(auto_gen.length);
                    print_info(&format!(
                        "Auto-generated {} ({} bytes)",
                        secret_config.name, auto_gen.length
                    ));
                    SecretString::from(generated)
                } else {
                    continue; // Skip optional secret with no auto-generate
                }
            } else if let Some(ref auto_gen) = secret_config.auto_generate {
                let generated = generate_secret_with_length(auto_gen.length);
                print_info(&format!(
                    "Auto-generated {} ({} bytes)",
                    secret_config.name, auto_gen.length
                ));
                SecretString::from(generated)
            } else {
                continue; // Skip optional secret with no auto-generate
            }
        } else {
            // Required secret
            let input_value = secret_input(&secret_config.prompt).map_err(|e| e.to_string())?;

            // Validate if pattern is provided
            if let Some(ref pattern) = secret_config.validation {
                let re = regex::Regex::new(pattern)
                    .map_err(|e| format!("Invalid validation pattern: {}", e))?;
                if !re.is_match(input_value.expose_secret()) {
                    print_error(&format!(
                        "Value does not match expected format: {}",
                        pattern
                    ));
                    return Err("Validation failed".to_string());
                }
            }

            input_value
        };

        // Save the secret
        secrets.save_secret(&secret_config.name, &value).await?;
        print_success(&format!("{} saved to database", secret_config.name));
    }

    // Optionally validate the configuration
    if let Some(ref validation_endpoint) = setup.validation_endpoint {
        print_info("Validating configuration...");
        // The validation endpoint may contain placeholders like {telegram_bot_token}
        // For now, we skip validation since we'd need to substitute secrets
        // A full implementation would fetch secrets and substitute them
        print_info(&format!(
            "Validation endpoint configured: {} (validation skipped)",
            validation_endpoint
        ));
    }

    print_success(&format!("{} channel configured", channel_name));

    Ok(WasmChannelSetupResult {
        enabled: true,
        channel_name: channel_name.to_string(),
    })
}

/// Generate a random secret of specified length (in bytes).
fn generate_secret_with_length(length: usize) -> String {
    use rand::RngCore;
    let mut rng = rand::thread_rng();
    let mut bytes = vec![0u8; length];
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
