use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::{Value, json};

use crate::import::openclaw::reader::{
    OpenClawConfig, OpenClawConversation, OpenClawLlmConfig, OpenClawMemoryChunk, OpenClawReader,
};
use crate::settings::{
    CustomLlmProviderSettings, LlmBuiltinOverride, builtin_secret_name, custom_secret_name,
};

use super::{
    ImportedConversation, ImportedDocument, ImportedMessage, ImportedMessageRole, ImportedSecret,
    MigrationError, MigrationServices, MigrationStats, collect_markdown_files,
    compatible_scalar_settings_patch, normalize_relative_path, slugify,
};

#[derive(Debug, Clone)]
pub struct OpenClawMigrationOptions {
    pub path: PathBuf,
    pub dry_run: bool,
}

pub fn detect() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
        .map(|home| home.join(".openclaw"))
        .filter(|path| path.join("openclaw.json").exists())
}

pub async fn migrate(
    services: &MigrationServices,
    options: &OpenClawMigrationOptions,
) -> Result<MigrationStats, MigrationError> {
    let reader = OpenClawReader::new(&options.path)?;
    let config = reader.read_config()?;
    let mut stats = MigrationStats::default();

    let (settings_patch, secrets, config_notes) = map_openclaw_config(&config);
    for note in config_notes {
        stats.push_note(note);
    }

    if options.dry_run {
        stats.settings += settings_patch.len();
        stats.secrets += secrets.len();
    } else {
        services
            .apply_settings_patch(settings_patch, &mut stats)
            .await?;
        for secret in secrets {
            services.store_secret(secret, &mut stats).await?;
        }
    }

    let workspace_root = options.path.join("workspace");
    for (relative_path, content) in collect_markdown_files(&workspace_root)? {
        let imported = ImportedDocument {
            source: "openclaw",
            namespace: "root".to_string(),
            external_id: relative_path.clone(),
            workspace_path: format!("imports/openclaw/root/workspace/{relative_path}"),
            title: format!("OpenClaw workspace: {relative_path}"),
            content,
            tags: vec![
                "migration".to_string(),
                "openclaw".to_string(),
                "workspace".to_string(),
            ],
            doc_type: ironclaw_engine::DocType::Note,
            created_at: None,
            metadata: json!({"source_path": relative_path}),
        };
        if options.dry_run {
            stats.workspace_documents += 1;
            stats.memory_docs += 1;
        } else {
            services.upsert_document(imported, &mut stats).await?;
        }
    }

    let agent_dbs = reader.list_agent_dbs()?;
    for (agent_name, db_path) in agent_dbs {
        let namespace = slugify(&agent_name);
        let chunks = reader.read_memory_chunks(&db_path).await?;
        let grouped_chunks = group_chunks_by_path(chunks);
        for (path, content) in grouped_chunks {
            let normalized = normalized_memory_path(&path);
            let imported = ImportedDocument {
                source: "openclaw",
                namespace: namespace.clone(),
                external_id: path.clone(),
                workspace_path: format!("imports/openclaw/agents/{namespace}/memory/{normalized}"),
                title: format!("OpenClaw memory ({agent_name}): {path}"),
                content,
                tags: vec![
                    "migration".to_string(),
                    "openclaw".to_string(),
                    "memory".to_string(),
                    agent_name.clone(),
                ],
                doc_type: ironclaw_engine::DocType::Note,
                created_at: None,
                metadata: json!({"agent": agent_name, "source_path": path}),
            };
            if options.dry_run {
                stats.workspace_documents += 1;
                stats.memory_docs += 1;
            } else {
                services.upsert_document(imported, &mut stats).await?;
            }
        }

        let conversations = reader.read_conversations(&db_path).await?;
        for conversation in conversations {
            let imported = imported_conversation(&agent_name, &namespace, conversation);
            if options.dry_run {
                stats.engine_threads += 1;
                stats.engine_conversations += 1;
                stats.legacy_conversations += 1;
                stats.messages += imported.messages.len();
            } else {
                services.upsert_conversation(imported, &mut stats).await?;
            }
        }
    }

    Ok(stats)
}

fn group_chunks_by_path(chunks: Vec<OpenClawMemoryChunk>) -> Vec<(String, String)> {
    let mut grouped: HashMap<String, Vec<OpenClawMemoryChunk>> = HashMap::new();
    for chunk in chunks {
        grouped.entry(chunk.path.clone()).or_default().push(chunk);
    }

    let mut docs: Vec<(String, String)> = grouped
        .into_iter()
        .map(|(path, mut parts)| {
            parts.sort_by_key(|chunk| chunk.chunk_index);
            let content = parts
                .into_iter()
                .map(|chunk| chunk.content)
                .collect::<Vec<_>>()
                .join("\n\n");
            (path, content)
        })
        .collect();
    docs.sort_by(|a, b| a.0.cmp(&b.0));
    docs
}

fn imported_conversation(
    agent_name: &str,
    namespace: &str,
    conversation: OpenClawConversation,
) -> ImportedConversation {
    let title = conversation_title(&conversation);
    let created_at = conversation.created_at;
    ImportedConversation {
        source: "openclaw",
        namespace: namespace.to_string(),
        external_id: conversation.id.clone(),
        source_channel: conversation.channel.clone(),
        title,
        created_at,
        messages: conversation
            .messages
            .into_iter()
            .map(|message| ImportedMessage {
                role: match message.role.to_ascii_lowercase().as_str() {
                    "user" | "human" => ImportedMessageRole::User,
                    "assistant" | "ai" => ImportedMessageRole::Assistant,
                    "system" => ImportedMessageRole::System,
                    other => ImportedMessageRole::Tool {
                        name: Some(other.to_string()),
                    },
                },
                content: message.content,
                timestamp: message.created_at,
            })
            .collect(),
        metadata: json!({
            "agent": agent_name,
            "source_id": conversation.id,
            "source_created_at": created_at.map(|value| value.to_rfc3339()),
        }),
    }
}

fn conversation_title(conversation: &OpenClawConversation) -> String {
    if let Some(first_user) = conversation
        .messages
        .iter()
        .find(|message| matches!(message.role.to_ascii_lowercase().as_str(), "user" | "human"))
    {
        let mut title: String = first_user.content.chars().take(80).collect();
        if first_user.content.chars().count() > 80 {
            title.push('…');
        }
        return title;
    }
    format!(
        "OpenClaw {} conversation {}",
        conversation.channel, conversation.id
    )
}

fn normalized_memory_path(path: &str) -> String {
    let normalized = normalize_relative_path(path);
    if normalized.is_empty() {
        "memory.md".to_string()
    } else {
        normalized
    }
}

struct ProviderResolution {
    backend: Option<String>,
    custom_provider: Option<CustomLlmProviderSettings>,
    builtin_override: Option<(String, LlmBuiltinOverride)>,
    secret_name: Option<String>,
    notes: Vec<String>,
}

fn map_openclaw_config(
    config: &OpenClawConfig,
) -> (HashMap<String, Value>, Vec<ImportedSecret>, Vec<String>) {
    let mut patch = HashMap::new();
    let mut secrets = Vec::new();
    let mut notes = Vec::new();

    if let Some(OpenClawLlmConfig {
        provider,
        model,
        api_key,
        base_url,
    }) = config.llm.as_ref()
    {
        let resolution = resolve_provider(
            provider.as_deref(),
            base_url.as_deref(),
            model.as_deref(),
            provider.as_deref().unwrap_or("openclaw"),
        );
        if let Some(backend) = resolution.backend.clone() {
            patch.insert("llm_backend".to_string(), Value::String(backend));
        }
        if let Some(model) = model {
            patch.insert("selected_model".to_string(), Value::String(model.clone()));
        }
        if let Some(custom_provider) = resolution.custom_provider {
            patch.insert(
                "llm_custom_providers".to_string(),
                serde_json::to_value(vec![custom_provider]).unwrap_or(Value::Null),
            );
        }
        if let Some(base_url) = base_url.as_ref() {
            match resolution.backend.as_deref() {
                Some("openai_compatible") => {
                    patch.insert(
                        "openai_compatible_base_url".to_string(),
                        Value::String(base_url.clone()),
                    );
                }
                Some("ollama") => {
                    patch.insert(
                        "ollama_base_url".to_string(),
                        Value::String(base_url.clone()),
                    );
                }
                _ => {}
            }
        }
        if let Some((provider_id, override_value)) = resolution.builtin_override {
            let mut overrides = HashMap::new();
            overrides.insert(provider_id, override_value);
            patch.insert(
                "llm_builtin_overrides".to_string(),
                serde_json::to_value(overrides).unwrap_or(Value::Null),
            );
        }
        if let (Some(secret_name), Some(api_key)) = (resolution.secret_name, api_key.clone()) {
            let provider_hint = provider.clone().map(|value| value.to_ascii_lowercase());
            secrets.push(
                ImportedSecret::new(secret_name, api_key)
                    .with_provider(provider_hint.unwrap_or_else(|| "openclaw".to_string())),
            );
        }
        notes.extend(resolution.notes);
    }

    if let Some(embeddings) = &config.embeddings {
        if let Some(provider) = embeddings.provider.as_ref() {
            let normalized = normalize_builtin_provider(provider);
            if matches!(
                normalized.as_deref(),
                Some("openai" | "nearai" | "ollama" | "bedrock")
            ) {
                patch.insert("embeddings.enabled".to_string(), Value::Bool(true));
                patch.insert(
                    "embeddings.provider".to_string(),
                    Value::String(normalized.unwrap_or_else(|| provider.to_ascii_lowercase())),
                );
            } else {
                notes.push(format!(
                    "OpenClaw embeddings provider '{}' could not be mapped cleanly; skipped embeddings.provider.",
                    provider
                ));
            }
        }
        if let Some(model) = embeddings.model.as_ref() {
            patch.insert("embeddings.enabled".to_string(), Value::Bool(true));
            patch.insert("embeddings.model".to_string(), Value::String(model.clone()));
        }
        if let (Some(api_key), Some(provider)) =
            (embeddings.api_key.clone(), embeddings.provider.as_deref())
            && normalize_builtin_provider(provider).as_deref() == Some("openai")
        {
            secrets.push(
                ImportedSecret::new(builtin_secret_name("openai"), api_key).with_provider("openai"),
            );
        }
    }

    let (compatible, ignored) = compatible_scalar_settings_patch(&config.other_settings);
    patch.extend(compatible);
    if !ignored.is_empty() {
        notes.push(format!(
            "Ignored {} OpenClaw config keys that do not map cleanly onto current IronClaw settings: {}",
            ignored.len(),
            ignored.join(", ")
        ));
    }

    (patch, dedupe_secrets(secrets), notes)
}

fn dedupe_secrets(secrets: Vec<ImportedSecret>) -> Vec<ImportedSecret> {
    let mut by_name = HashMap::new();
    for secret in secrets {
        by_name.insert(secret.name.clone(), secret);
    }
    let mut deduped: Vec<_> = by_name.into_values().collect();
    deduped.sort_by(|a, b| a.name.cmp(&b.name));
    deduped
}

fn resolve_provider(
    provider: Option<&str>,
    base_url: Option<&str>,
    model: Option<&str>,
    custom_hint: &str,
) -> ProviderResolution {
    let mut notes = Vec::new();
    let normalized = provider.and_then(normalize_builtin_provider);

    match normalized.as_deref() {
        Some("openai")
        | Some("anthropic")
        | Some("nearai")
        | Some("github_copilot")
        | Some("tinfoil") => {
            let provider_id = normalized.unwrap_or_else(|| "openai".to_string());
            let builtin_override = base_url.map(|url| {
                (
                    provider_id.clone(),
                    LlmBuiltinOverride {
                        api_key: None,
                        model: model.map(|value| value.to_string()),
                        base_url: Some(url.to_string()),
                    },
                )
            });
            ProviderResolution {
                backend: Some(provider_id.clone()),
                custom_provider: None,
                builtin_override,
                secret_name: Some(builtin_secret_name(&provider_id)),
                notes,
            }
        }
        Some("openai_compatible") => ProviderResolution {
            backend: Some("openai_compatible".to_string()),
            custom_provider: None,
            builtin_override: None,
            secret_name: Some(builtin_secret_name("openai_compatible")),
            notes,
        },
        Some("ollama") => ProviderResolution {
            backend: Some("ollama".to_string()),
            custom_provider: None,
            builtin_override: None,
            secret_name: None,
            notes,
        },
        Some("bedrock") => ProviderResolution {
            backend: Some("bedrock".to_string()),
            custom_provider: None,
            builtin_override: None,
            secret_name: None,
            notes,
        },
        _ => {
            let Some(base_url) = base_url else {
                notes.push(format!(
                    "OpenClaw provider '{}' has no base_url; skipping active provider migration.",
                    provider.unwrap_or("unknown")
                ));
                return ProviderResolution {
                    backend: None,
                    custom_provider: None,
                    builtin_override: None,
                    secret_name: None,
                    notes,
                };
            };

            let provider_id = slugify(custom_hint);
            let adapter = if provider
                .unwrap_or_default()
                .to_ascii_lowercase()
                .contains("anthropic")
            {
                "anthropic"
            } else if provider
                .unwrap_or_default()
                .to_ascii_lowercase()
                .contains("ollama")
            {
                "ollama"
            } else {
                "open_ai_completions"
            };

            ProviderResolution {
                backend: Some(provider_id.clone()),
                custom_provider: Some(CustomLlmProviderSettings {
                    id: provider_id.clone(),
                    name: provider.unwrap_or(custom_hint).to_string(),
                    adapter: adapter.to_string(),
                    base_url: Some(base_url.to_string()),
                    default_model: model.map(|value| value.to_string()),
                    api_key: None,
                    builtin: false,
                }),
                builtin_override: None,
                secret_name: Some(custom_secret_name(&provider_id)),
                notes,
            }
        }
    }
}

fn normalize_builtin_provider(provider: &str) -> Option<String> {
    let normalized = provider.trim().to_ascii_lowercase().replace('-', "_");
    let mapped = match normalized.as_str() {
        "claude" => "anthropic",
        "copilot" => "github_copilot",
        "openai_compatible" | "openrouter" | "vllm" | "lmstudio" | "lm_studio" => {
            "openai_compatible"
        }
        "openai" | "anthropic" | "nearai" | "github_copilot" | "ollama" | "tinfoil" | "bedrock" => {
            normalized.as_str()
        }
        _ => return None,
    };
    Some(mapped.to_string())
}
