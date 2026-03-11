use std::collections::HashSet;
use std::sync::Arc;

use crate::secrets::SecretsStore;

use super::wrapper::WasmChannel;

#[doc(hidden)]
pub async fn resolve_channel_secret(
    secrets: Option<&(dyn SecretsStore + Send + Sync)>,
    user_id: &str,
    secret_name: &str,
) -> Option<String> {
    if let Some(secrets) = secrets
        && let Ok(secret) = secrets.get_decrypted(user_id, secret_name).await
    {
        return Some(secret.expose().to_string());
    }

    std::env::var(secret_name.to_ascii_uppercase()).ok()
}

/// Inject credentials for a channel from the secrets store, with env-var fallback.
///
/// Returns the number of credentials injected.
#[doc(hidden)]
pub async fn inject_channel_credentials(
    channel: &Arc<WasmChannel>,
    secrets: Option<&(dyn SecretsStore + Send + Sync)>,
    channel_name: &str,
    declared_secret_names: &[String],
    reserved_host_secret_names: &[String],
    user_id: &str,
) -> Result<usize, String> {
    let prefix = format!("{}_", channel_name);
    let reserved: HashSet<&str> = reserved_host_secret_names
        .iter()
        .map(String::as_str)
        .collect();
    let mut injected = HashSet::new();
    let mut count = 0;

    if let Some(secrets) = secrets {
        let all_secrets = secrets
            .list(user_id)
            .await
            .map_err(|e| format!("Failed to list secrets: {}", e))?;

        for secret_meta in all_secrets {
            if !secret_meta.name.starts_with(&prefix) || reserved.contains(secret_meta.name.as_str()) {
                continue;
            }

            let decrypted = match secrets.get_decrypted(user_id, &secret_meta.name).await {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!(
                        secret = %secret_meta.name,
                        error = %e,
                        "Failed to decrypt secret for channel credential injection"
                    );
                    continue;
                }
            };

            let placeholder = secret_meta.name.to_uppercase();

            tracing::debug!(
                channel = %channel_name,
                secret = %secret_meta.name,
                placeholder = %placeholder,
                "Injecting credential from secrets store"
            );

            channel
                .set_credential(&placeholder, decrypted.expose().to_string())
                .await;
            injected.insert(placeholder);
            count += 1;
        }
    }

    for secret_name in declared_secret_names {
        if reserved.contains(secret_name.as_str()) {
            continue;
        }

        let placeholder = secret_name.to_ascii_uppercase();
        if injected.contains(&placeholder) {
            continue;
        }

        let Ok(value) = std::env::var(&placeholder) else {
            continue;
        };

        tracing::debug!(
            channel = %channel_name,
            env = %placeholder,
            "Injecting credential from environment"
        );

        channel.set_credential(&placeholder, value).await;
        injected.insert(placeholder);
        count += 1;
    }

    // Fall back to environment variables for required HTTP credentials not
    // covered by setup secrets. This keeps env-configured channels working
    // without requiring an interactive setup flow first.
    let caps = channel.capabilities();
    if let Some(ref http_cap) = caps.tool_capabilities.http {
        for cred_mapping in http_cap.credentials.values() {
            let placeholder = cred_mapping.secret_name.to_uppercase();
            if injected.contains(&placeholder) {
                continue;
            }
            if let Ok(env_value) = std::env::var(&placeholder)
                && !env_value.is_empty()
            {
                tracing::debug!(
                    channel = %channel_name,
                    placeholder = %placeholder,
                    "Injecting credential from environment variable"
                );
                channel.set_credential(&placeholder, env_value).await;
                injected.insert(placeholder);
                count += 1;
            }
        }
    }

    Ok(count)
}
