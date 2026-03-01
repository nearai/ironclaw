//! Channel-specific setup flows.
//!
//! Each channel (Telegram, HTTP, etc.) has its own setup function that:
//! 1. Displays setup instructions
//! 2. Collects configuration (tokens, ports, etc.)
//! 3. Validates the configuration
//! 4. Saves secrets to the database

use std::sync::Arc;

use base64::Engine;
use reqwest::Client;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use url::Url;
use uuid::Uuid;

#[cfg(feature = "postgres")]
use crate::secrets::SecretsCrypto;
use crate::secrets::{CreateSecretParams, SecretsStore};
use crate::settings::{Settings, TunnelSettings};
use crate::setup::prompts::{
    confirm, input, optional_input, print_error, print_info, print_success, print_warning,
    secret_input, select_one,
};

/// Typed errors for channel setup flows.
#[derive(Debug, thiserror::Error)]
pub enum ChannelSetupError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Network(String),

    #[error("{0}")]
    Secrets(String),

    #[error("{0}")]
    Validation(String),

    #[error("Setup cancelled by user")]
    Cancelled,
}

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
    pub async fn save_secret(
        &self,
        name: &str,
        value: &SecretString,
    ) -> Result<(), ChannelSetupError> {
        let params = CreateSecretParams::new(name, value.expose_secret());

        self.store
            .create(&self.user_id, params)
            .await
            .map_err(|e| ChannelSetupError::Secrets(format!("Failed to save secret: {}", e)))?;

        Ok(())
    }

    /// Check if a secret exists.
    pub async fn secret_exists(&self, name: &str) -> bool {
        match self.store.exists(&self.user_id, name).await {
            Ok(exists) => exists,
            Err(e) => {
                tracing::warn!(secret = name, error = %e, "Failed to check if secret exists, assuming absent");
                false
            }
        }
    }

    /// Read a secret from the database (decrypted).
    pub async fn get_secret(&self, name: &str) -> Result<SecretString, ChannelSetupError> {
        let decrypted = self
            .store
            .get_decrypted(&self.user_id, name)
            .await
            .map_err(|e| ChannelSetupError::Secrets(format!("Failed to read secret: {}", e)))?;
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
) -> Result<TelegramSetupResult, ChannelSetupError> {
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
        if !confirm("Replace existing token?", false)? {
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

    loop {
        let token = secret_input("Bot token (from @BotFather)")?;

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
                let webhook_secret =
                    setup_telegram_webhook_secret(secrets, &settings.tunnel).await?;

                return Ok(TelegramSetupResult {
                    enabled: true,
                    bot_username: username,
                    webhook_secret,
                    owner_id,
                });
            }
            Err(e) => {
                print_error(&format!("Token validation failed: {}", e));

                if !confirm("Try again?", true)? {
                    return Ok(TelegramSetupResult {
                        enabled: false,
                        bot_username: None,
                        webhook_secret: None,
                        owner_id: None,
                    });
                }
            }
        }
    }
}

/// Bind the bot to the owner's Telegram account by having them send a message.
///
/// Polls `getUpdates` until a message arrives, then captures the sender's user ID.
/// Returns `None` if the user declines or the flow times out.
async fn bind_telegram_owner(token: &SecretString) -> Result<Option<i64>, ChannelSetupError> {
    println!();
    print_info("Account Binding (recommended):");
    print_info("Binding restricts the bot so only YOU can use it.");
    print_info("Without this, anyone who finds your bot can send it messages.");
    println!();

    if !confirm("Bind bot to your Telegram account?", true)? {
        print_info("Skipping account binding. Bot will accept messages from all users.");
        return Ok(None);
    }

    print_info("Send any message (e.g. /start) to your bot in Telegram.");
    print_info("Waiting for your message (up to 120 seconds)...");

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(35))
        .build()
        .map_err(|e| ChannelSetupError::Network(format!("Failed to create HTTP client: {}", e)))?;

    // Clear any existing webhook so getUpdates works
    let delete_url = format!(
        "https://api.telegram.org/bot{}/deleteWebhook",
        token.expose_secret()
    );
    if let Err(e) = client.post(&delete_url).send().await {
        tracing::warn!("Failed to delete webhook (getUpdates may not work): {e}");
    }

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
            .map_err(|e| ChannelSetupError::Network(format!("getUpdates request failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(ChannelSetupError::Network(format!(
                "getUpdates returned status {}",
                response.status()
            )));
        }

        let body: TelegramGetUpdatesResponse = response.json().await.map_err(|e| {
            ChannelSetupError::Network(format!("Failed to parse getUpdates response: {}", e))
        })?;

        if !body.ok {
            return Err(ChannelSetupError::Network(
                "Telegram API returned error for getUpdates".to_string(),
            ));
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
                if let Err(e) = client
                    .get(&ack_url)
                    .query(&[("offset", &(update.update_id + 1).to_string())])
                    .send()
                    .await
                {
                    tracing::warn!("Failed to acknowledge Telegram update: {e}");
                }

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
) -> Result<Option<i64>, ChannelSetupError> {
    if settings.channels.telegram_owner_id.is_some() {
        print_info("Bot is already bound to a Telegram account.");
        if !confirm("Re-bind to a different account?", false)? {
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
/// Returns a `TunnelSettings` with provider config (managed tunnel)
/// or a static URL.
pub fn setup_tunnel(settings: &Settings) -> Result<TunnelSettings, ChannelSetupError> {
    // Show existing config
    let has_existing = settings.tunnel.public_url.is_some() || settings.tunnel.provider.is_some();
    if has_existing {
        println!();
        print_info("Current tunnel configuration:");
        let t = &settings.tunnel;
        match t.provider.as_deref() {
            Some("ngrok") => {
                print_info("  Provider:  ngrok");
                if let Some(ref domain) = t.ngrok_domain {
                    print_info(&format!("  Domain:    {}", domain));
                }
                if t.ngrok_token.is_some() {
                    print_info("  Auth:      token configured");
                }
            }
            Some("cloudflare") => {
                print_info("  Provider:  Cloudflare Tunnel");
                if t.cf_token.is_some() {
                    print_info("  Auth:      token configured");
                }
            }
            Some("tailscale") => {
                let mode = if t.ts_funnel {
                    "Funnel (public)"
                } else {
                    "Serve (tailnet-only)"
                };
                print_info(&format!("  Provider:  Tailscale {}", mode));
                if let Some(ref hostname) = t.ts_hostname {
                    print_info(&format!("  Hostname:  {}", hostname));
                }
            }
            Some("custom") => {
                print_info("  Provider:  Custom command");
                if let Some(ref cmd) = t.custom_command {
                    print_info(&format!("  Command:   {}", cmd));
                }
                if let Some(ref url) = t.custom_health_url {
                    print_info(&format!("  Health:    {}", url));
                }
            }
            Some(other) => {
                print_info(&format!("  Provider:  {}", other));
            }
            None => {}
        }
        if let Some(ref url) = t.public_url {
            print_info(&format!("  URL:       {}", url));
        }
        println!();
        if !confirm("Change tunnel configuration?", false)? {
            return Ok(settings.tunnel.clone());
        }
    }

    println!();
    print_info("Tunnel Configuration (for webhook endpoints):");
    print_info("A tunnel exposes your local agent to the internet, enabling:");
    print_info("  - Instant Telegram message delivery (instead of polling)");
    print_info("  - Slack, Discord, GitHub webhooks");
    println!();

    if !confirm("Configure a tunnel?", false)? {
        return Ok(TunnelSettings::default());
    }

    let options = &[
        "ngrok         - managed tunnel, starts automatically",
        "Cloudflare    - cloudflared tunnel, starts automatically",
        "Tailscale     - Tailscale Funnel/Serve, starts automatically",
        "Custom        - your own tunnel command",
        "Static URL    - you manage the tunnel yourself",
    ];

    let choice = select_one("Select tunnel provider:", options)?;

    match choice {
        0 => setup_tunnel_ngrok(),
        1 => setup_tunnel_cloudflare(),
        2 => setup_tunnel_tailscale(),
        3 => setup_tunnel_custom(),
        4 => setup_tunnel_static(),
        _ => Ok(TunnelSettings::default()),
    }
}

fn setup_tunnel_ngrok() -> Result<TunnelSettings, ChannelSetupError> {
    print_info("Get your auth token from: https://dashboard.ngrok.com/get-started/your-authtoken");
    println!();

    let token = secret_input("ngrok auth token")?;
    let domain = optional_input("Custom domain", Some("leave empty for auto-assigned"))?;

    print_success("ngrok configured. Tunnel will start automatically at boot.");

    Ok(TunnelSettings {
        provider: Some("ngrok".to_string()),
        ngrok_token: Some(token.expose_secret().to_string()),
        ngrok_domain: domain,
        ..Default::default()
    })
}

fn setup_tunnel_cloudflare() -> Result<TunnelSettings, ChannelSetupError> {
    // Check if cloudflared binary is on PATH
    let cloudflared_found = crate::skills::gating::binary_exists("cloudflared");

    if !cloudflared_found {
        print_error("cloudflared not found in PATH.");
        print_info("Install it:");
        print_info("  macOS:   brew install cloudflared");
        print_info("  Ubuntu:  https://pkg.cloudflare.com/");
        print_info(
            "  Other:   https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/",
        );
        println!();
        if !confirm(
            "Continue anyway (you can install cloudflared later)?",
            false,
        )? {
            return Err(ChannelSetupError::Validation(
                "cloudflared binary not found. Install it and re-run setup.".to_string(),
            ));
        }
    }

    // Detect existing cloudflared services that may conflict
    if let Some(warning) = detect_existing_cloudflared() {
        print_warning(&warning);
        if !confirm("Continue anyway?", true)? {
            return Err(ChannelSetupError::Cancelled);
        }
        println!();
    }

    print_info("Get your tunnel token from the Cloudflare Zero Trust dashboard:");
    print_info("  https://one.dash.cloudflare.com/ > Networks > Tunnels");
    println!();

    let token = secret_input("Cloudflare tunnel token")?;

    let token_valid = validate_cloudflare_token_format(token.expose_secret());

    if !token_valid {
        print_error("Token does not appear to be a valid Cloudflare tunnel token.");
        print_info("Tokens are base64-encoded and contain account/tunnel identifiers.");
        print_info(
            "Copy the full token from: Zero Trust dashboard > Networks > Tunnels > your tunnel",
        );
        println!();
        if !confirm("Save this token anyway?", false)? {
            return Err(ChannelSetupError::Validation(
                "Invalid Cloudflare tunnel token format.".to_string(),
            ));
        }
    }

    print_success("Cloudflare tunnel token saved.");
    if cloudflared_found {
        print_info("Start the tunnel with: cloudflared tunnel --no-autoupdate run --token <token>");
        print_info("For auto-start, install cloudflared as a system service:");
        print_info("  sudo cloudflared service install <token>");
    } else {
        print_info("After installing cloudflared, start the tunnel with:");
        print_info("  cloudflared tunnel --no-autoupdate run --token <token>");
    }

    Ok(TunnelSettings {
        provider: Some("cloudflare".to_string()),
        cf_token: Some(token.expose_secret().to_string()),
        ..Default::default()
    })
}

/// Detect running cloudflared processes or managed services that could conflict
/// with IronClaw's tunnel management.
fn detect_existing_cloudflared() -> Option<String> {
    let mut conflicts = Vec::new();

    // Check for running cloudflared processes (all platforms)
    #[cfg(unix)]
    {
        let output = std::process::Command::new("pgrep")
            .args(["-x", "cloudflared"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output();
        if let Ok(out) = output
            && out.status.success()
        {
            let pids = String::from_utf8_lossy(&out.stdout);
            let pids: Vec<&str> = pids.trim().lines().collect();
            if !pids.is_empty() {
                conflicts.push(format!(
                    "Running cloudflared process(es): PID {}",
                    pids.join(", ")
                ));
            }
        }
    }

    // macOS: check brew services
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("brew")
            .args(["services", "list"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output();
        if let Ok(out) = output {
            let stdout = String::from_utf8_lossy(&out.stdout);
            for line in stdout.lines() {
                if line.contains("cloudflared") && line.contains("started") {
                    conflicts.push("Homebrew service: cloudflared (started)".to_string());
                    break;
                }
            }
        }

        let output = std::process::Command::new("launchctl")
            .args(["list"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output();
        if let Ok(out) = output {
            let stdout = String::from_utf8_lossy(&out.stdout);
            for line in stdout.lines() {
                if line.contains("cloudflared") {
                    conflicts.push("launchd service: cloudflared detected".to_string());
                    break;
                }
            }
        }
    }

    // Linux: check systemd
    #[cfg(target_os = "linux")]
    {
        let output = std::process::Command::new("systemctl")
            .args(["is-active", "cloudflared"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output();
        if let Ok(out) = output {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if stdout.trim() == "active" {
                conflicts.push("systemd service: cloudflared (active)".to_string());
            }
        }
    }

    if conflicts.is_empty() {
        None
    } else {
        Some(format!(
            "Detected existing cloudflared service(s) that may conflict:\n  {}\n\
             Consider stopping them first (e.g., `brew services stop cloudflared` or \
             `sudo systemctl stop cloudflared`).",
            conflicts.join("\n  ")
        ))
    }
}

fn setup_tunnel_tailscale() -> Result<TunnelSettings, ChannelSetupError> {
    let funnel = confirm("Use Tailscale Funnel (public internet)?", true)?;
    let hostname = optional_input("Hostname override", Some("leave empty for auto-detect"))?;

    let mode = if funnel {
        "Funnel (public)"
    } else {
        "Serve (tailnet-only)"
    };
    print_success(&format!("Tailscale {} configured.", mode));

    Ok(TunnelSettings {
        provider: Some("tailscale".to_string()),
        ts_funnel: funnel,
        ts_hostname: hostname,
        ..Default::default()
    })
}

fn setup_tunnel_custom() -> Result<TunnelSettings, ChannelSetupError> {
    print_info("Enter a shell command to start your tunnel.");
    print_info("Use {port} and {host} as placeholders.");
    print_info("Example: bore local {port} --to bore.pub");
    println!();

    let command = input("Tunnel command")?;
    if command.is_empty() {
        return Err(ChannelSetupError::Validation(
            "Tunnel command cannot be empty".to_string(),
        ));
    }

    let health_url = optional_input("Health check URL", Some("optional"))?;
    let url_pattern = optional_input(
        "URL pattern (substring to match in stdout)",
        Some("optional"),
    )?;

    print_success("Custom tunnel configured.");

    Ok(TunnelSettings {
        provider: Some("custom".to_string()),
        custom_command: Some(command),
        custom_health_url: health_url,
        custom_url_pattern: url_pattern,
        ..Default::default()
    })
}

fn setup_tunnel_static() -> Result<TunnelSettings, ChannelSetupError> {
    print_info("Enter the public URL of your externally managed tunnel.");
    println!();

    let tunnel_url = input("Tunnel URL (e.g., https://abc123.ngrok.io)")?;

    if !tunnel_url.starts_with("https://") {
        print_error("URL must start with https:// (webhooks require HTTPS)");
        return Err(ChannelSetupError::Validation(
            "Invalid tunnel URL: must use HTTPS".to_string(),
        ));
    }

    let tunnel_url = tunnel_url.trim_end_matches('/').to_string();

    print_success(&format!("Static tunnel URL configured: {}", tunnel_url));
    print_info("Make sure your tunnel is running before starting the agent.");

    Ok(TunnelSettings {
        public_url: Some(tunnel_url),
        ..Default::default()
    })
}

/// Set up Telegram webhook secret for signature validation.
///
/// Returns the webhook secret if configured.
async fn setup_telegram_webhook_secret(
    secrets: &SecretsContext,
    tunnel: &TunnelSettings,
) -> Result<Option<String>, ChannelSetupError> {
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

    if !confirm("Generate a webhook secret?", true)? {
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
pub async fn validate_telegram_token(
    token: &SecretString,
) -> Result<Option<String>, ChannelSetupError> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| ChannelSetupError::Network(format!("Failed to create HTTP client: {}", e)))?;

    let url = format!(
        "https://api.telegram.org/bot{}/getMe",
        token.expose_secret()
    );

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| ChannelSetupError::Network(format!("Request failed: {}", e)))?;

    if !response.status().is_success() {
        return Err(ChannelSetupError::Network(format!(
            "API returned status {}",
            response.status()
        )));
    }

    let body: TelegramGetMeResponse = response
        .json()
        .await
        .map_err(|e| ChannelSetupError::Network(format!("Failed to parse response: {}", e)))?;

    if body.ok {
        Ok(body.result.and_then(|u| u.username))
    } else {
        Err(ChannelSetupError::Network(
            "Telegram API returned error".to_string(),
        ))
    }
}

/// Result of HTTP webhook setup.
#[derive(Debug, Clone)]
pub struct HttpSetupResult {
    pub enabled: bool,
    pub port: u16,
    pub host: String,
}

/// Result of Signal channel setup.
#[derive(Debug, Clone)]
pub struct SignalSetupResult {
    pub enabled: bool,
    pub http_url: String,
    pub account: String,
    pub allow_from: String,
    pub allow_from_groups: String,
    pub dm_policy: String,
    pub group_policy: String,
    pub group_allow_from: String,
}

/// Set up HTTP webhook channel.
pub async fn setup_http(secrets: &SecretsContext) -> Result<HttpSetupResult, ChannelSetupError> {
    println!("HTTP Webhook Setup:");
    println!();
    print_info("The HTTP webhook allows external services to send messages to the agent.");
    println!();

    let port_str = optional_input("Port", Some("default: 8080"))?;
    let port: u16 = port_str
        .as_deref()
        .unwrap_or("8080")
        .parse()
        .map_err(|e| ChannelSetupError::Validation(format!("Invalid port: {}", e)))?;

    if port < 1024 {
        print_info("Note: Ports below 1024 may require root privileges");
    }

    let host =
        optional_input("Host", Some("default: 0.0.0.0"))?.unwrap_or_else(|| "0.0.0.0".to_string());

    // Generate a webhook secret
    if confirm("Generate a webhook secret for authentication?", true)? {
        let secret = generate_webhook_secret();
        secrets
            .save_secret("http_webhook_secret", &SecretString::from(secret))
            .await?;
        print_success("Webhook secret generated and saved to database");
        print_info("Retrieve it later with: ironclaw secret get http_webhook_secret");
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
    generate_secret_with_length(32)
}

fn validate_e164(account: &str) -> Result<(), String> {
    if !account.starts_with('+') {
        return Err("E.164 account must start with '+'".to_string());
    }
    let digits = &account[1..];
    if digits.is_empty() {
        return Err("E.164 account must have digits after '+'".to_string());
    }
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err("E.164 account must contain only digits after '+'".to_string());
    }
    if digits.len() < 7 || digits.len() > 15 {
        return Err("E.164 account must be 7-15 digits after '+'".to_string());
    }
    Ok(())
}

fn validate_allow_from_list(list: &str) -> Result<(), String> {
    if list.is_empty() {
        return Ok(());
    }
    for (i, item) in list.split(',').enumerate() {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == "*" {
            continue;
        }
        if let Some(uuid_part) = trimmed.strip_prefix("uuid:") {
            if Uuid::parse_str(uuid_part).is_err() {
                return Err(format!(
                    "allow_from[{}]: '{}' is not a valid UUID (after 'uuid:' prefix)",
                    i, trimmed
                ));
            }
            continue;
        }
        if validate_e164(trimmed).is_ok() {
            continue;
        }
        if Uuid::parse_str(trimmed).is_ok() {
            continue;
        }
        return Err(format!(
            "allow_from[{}]: '{}' must be '*', E.164 phone number, UUID, or 'uuid:<id>'",
            i, trimmed
        ));
    }
    Ok(())
}

fn validate_allow_from_groups_list(list: &str) -> Result<(), String> {
    if list.is_empty() {
        return Ok(());
    }
    for (i, item) in list.split(',').enumerate() {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == "*" {
            continue;
        }
        if trimmed.is_empty() {
            return Err(format!(
                "allow_from_groups[{}]: group ID cannot be empty",
                i
            ));
        }
    }
    Ok(())
}

/// Set up Signal channel.
/// `Settings` is reserved for future use
pub async fn setup_signal(_settings: &Settings) -> Result<SignalSetupResult, ChannelSetupError> {
    println!("Signal Channel Setup:");
    println!();
    print_info("Signal channel connects to a signal-cli daemon running in HTTP mode.");
    println!();

    let http_url = input("Signal-cli HTTP URL")?;
    match Url::parse(&http_url) {
        Ok(url) if url.scheme() == "http" || url.scheme() == "https" => {}
        Ok(_) => {
            print_error("URL must use http or https scheme");
            return Err(ChannelSetupError::Validation(
                "Invalid HTTP URL: must use http or https scheme".to_string(),
            ));
        }
        Err(e) => {
            print_error(&format!("Invalid URL: {}", e));
            return Err(ChannelSetupError::Validation(format!(
                "Invalid HTTP URL: {}",
                e
            )));
        }
    }

    let account = input("Signal account (E.164)")?;
    if let Err(e) = validate_e164(&account) {
        print_error(&e);
        return Err(ChannelSetupError::Validation(e));
    }

    let allow_from = optional_input(
        "Allow from (comma-separated: E.164 numbers, '*' for anyone, UUIDs or 'uuid:<id>'; empty for self-only)",
        Some(&format!("default: {} (self-only)", account)),
    )?
    .unwrap_or_else(|| account.clone());

    let dm_policy = optional_input(
        "DM policy (open, allowlist, pairing)",
        Some("default: pairing"),
    )?
    .unwrap_or_else(|| "pairing".to_string());

    let allow_from_groups = optional_input(
        "Allow from groups (comma-separated group IDs, '*' for any group; empty for none)",
        Some("default: (none)"),
    )?
    .unwrap_or_default();

    let group_policy = optional_input(
        "Group policy (allowlist, open, disabled)",
        Some("default: allowlist"),
    )?
    .unwrap_or_else(|| "allowlist".to_string());

    let group_allow_from = optional_input(
        "Group allow from (comma-separated member IDs; empty to inherit from allow_from)",
        Some("default: (inherit from allow_from)"),
    )?
    .unwrap_or_default();

    if let Err(e) = validate_allow_from_list(&allow_from) {
        print_error(&e);
        return Err(ChannelSetupError::Validation(e));
    }

    if let Err(e) = validate_allow_from_groups_list(&allow_from_groups) {
        print_error(&e);
        return Err(ChannelSetupError::Validation(e));
    }

    println!();
    print_success(&format!(
        "Signal channel configured for account: {}",
        account
    ));
    print_info(&format!("HTTP URL: {}", http_url));
    if allow_from == account {
        print_info("Allow from: self-only");
    } else {
        print_info(&format!("Allow from: {}", allow_from));
    }
    print_info(&format!("DM policy: {}", dm_policy));
    if allow_from_groups.is_empty() {
        print_info("Allow from groups: (none)");
    } else {
        print_info(&format!("Allow from groups: {}", allow_from_groups));
    }
    print_info(&format!("Group policy: {}", group_policy));
    if group_allow_from.is_empty() {
        print_info("Group allow from: (inherits from allow_from)");
    } else {
        print_info(&format!("Group allow from: {}", group_allow_from));
    }

    Ok(SignalSetupResult {
        enabled: true,
        http_url,
        account,
        allow_from,
        allow_from_groups,
        dm_policy,
        group_policy,
        group_allow_from,
    })
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
) -> Result<WasmChannelSetupResult, ChannelSetupError> {
    println!("{} Setup:", channel_name);
    println!();

    for secret_config in &setup.required_secrets {
        // Check if this secret already exists
        if secrets.secret_exists(&secret_config.name).await {
            print_info(&format!(
                "Existing {} found in database.",
                secret_config.name
            ));
            if !confirm("Replace existing value?", false)? {
                continue;
            }
        }

        // Get the value from user or auto-generate
        let value = if secret_config.optional {
            let input_value =
                optional_input(&secret_config.prompt, Some("leave empty to auto-generate"))?;

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
            let input_value = secret_input(&secret_config.prompt)?;

            // Validate if pattern is provided
            if let Some(ref pattern) = secret_config.validation {
                let re = regex::Regex::new(pattern).map_err(|e| {
                    ChannelSetupError::Validation(format!("Invalid validation pattern: {}", e))
                })?;
                if !re.is_match(input_value.expose_secret()) {
                    print_error(&format!(
                        "Value does not match expected format: {}",
                        pattern
                    ));
                    return Err(ChannelSetupError::Validation(
                        "Validation failed".to_string(),
                    ));
                }
            }

            input_value
        };

        // Save the secret
        secrets.save_secret(&secret_config.name, &value).await?;
        print_success(&format!("{} saved to database", secret_config.name));
    }

    // TODO: Substitute secrets into the validation URL and make a
    // GET request to verify the configured credentials actually work.
    if let Some(ref validation_endpoint) = setup.validation_endpoint {
        print_info(&format!(
            "Validation endpoint configured: {} (validation not yet implemented)",
            validation_endpoint
        ));
    }

    print_success(&format!("{} channel configured", channel_name));

    Ok(WasmChannelSetupResult {
        enabled: true,
        channel_name: channel_name.to_string(),
    })
}

/// Validate that a Cloudflare tunnel token has the expected format.
///
/// Cloudflare tunnel tokens are base64-encoded JSON objects containing
/// at least `"a"` (account tag) and `"t"` (tunnel ID) fields.
fn validate_cloudflare_token_format(token: &str) -> bool {
    base64::engine::general_purpose::STANDARD
        .decode(token)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(token))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .is_some_and(|json| json.get("a").is_some() && json.get("t").is_some())
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
    use base64::Engine;

    use crate::setup::channels::{generate_webhook_secret, validate_cloudflare_token_format};

    #[test]
    fn test_generate_webhook_secret() {
        let secret = generate_webhook_secret();
        assert_eq!(secret.len(), 64); // 32 bytes = 64 hex chars
    }

    #[test]
    fn test_generate_secret_with_length() {
        use super::generate_secret_with_length;

        let s = generate_secret_with_length(16);
        assert_eq!(s.len(), 32); // 16 bytes = 32 hex chars
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));

        let s2 = generate_secret_with_length(1);
        assert_eq!(s2.len(), 2);
    }

    #[test]
    fn test_validate_cloudflare_token_valid() {
        // Simulate a valid Cloudflare tunnel token: base64-encoded JSON with "a" and "t" fields
        let payload = serde_json::json!({"a": "account-tag", "t": "tunnel-id", "s": "secret"});
        let token =
            base64::engine::general_purpose::STANDARD.encode(payload.to_string().as_bytes());
        assert!(validate_cloudflare_token_format(&token));
    }

    #[test]
    fn test_validate_cloudflare_token_missing_fields() {
        // JSON but missing required "a" and "t" fields
        let payload = serde_json::json!({"foo": "bar"});
        let token =
            base64::engine::general_purpose::STANDARD.encode(payload.to_string().as_bytes());
        assert!(!validate_cloudflare_token_format(&token));
    }

    #[test]
    fn test_validate_cloudflare_token_not_base64() {
        assert!(!validate_cloudflare_token_format("not-base64!!!"));
    }

    #[test]
    fn test_validate_cloudflare_token_not_json() {
        let token = base64::engine::general_purpose::STANDARD.encode(b"not json at all");
        assert!(!validate_cloudflare_token_format(&token));
    }

    #[test]
    fn test_validate_cloudflare_token_empty() {
        assert!(!validate_cloudflare_token_format(""));
    }
}
