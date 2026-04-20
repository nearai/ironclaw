use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use secrecy::SecretString;
use serde_json::{Value, json};

use crate::settings::{
    CustomLlmProviderSettings, LlmBuiltinOverride, builtin_secret_name, custom_secret_name,
};

use super::{
    ImportedConversation, ImportedDocument, ImportedMessage, ImportedMessageRole, ImportedSecret,
    MigrationError, MigrationServices, MigrationStats, collect_markdown_files,
    normalize_relative_path, parse_timestamp_str, slugify,
};

#[derive(Debug, Clone)]
pub struct HermesMigrationOptions {
    pub path: PathBuf,
    pub dry_run: bool,
    pub profiles: Vec<String>,
    pub all_profiles: bool,
}

#[derive(Debug, Clone)]
struct HermesProviderEntry {
    id: String,
    name: String,
    base_url: Option<String>,
    key_env: Option<String>,
    transport: Option<String>,
    model: Option<String>,
    api_key: Option<SecretString>,
}

#[derive(Debug, Clone, Default)]
struct HermesConfigData {
    model_provider: Option<String>,
    default_model: Option<String>,
    model_base_url: Option<String>,
    providers: Vec<HermesProviderEntry>,
}

#[derive(Debug, Clone)]
struct HermesMessage {
    role: String,
    content: String,
    timestamp: Option<chrono::DateTime<chrono::Utc>>,
    tool_name: Option<String>,
}

#[derive(Debug, Clone)]
struct HermesSession {
    id: String,
    source: Option<String>,
    user_id: Option<String>,
    model: Option<String>,
    title: Option<String>,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
    messages: Vec<HermesMessage>,
}

#[derive(Debug, Clone)]
struct HermesScope {
    name: String,
    root: PathBuf,
}

pub fn detect() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
        .map(|home| home.join(".hermes"))
        .filter(|path| path.join("config.yaml").exists() || path.join("state.db").exists())
}

pub async fn migrate(
    services: &MigrationServices,
    options: &HermesMigrationOptions,
) -> Result<MigrationStats, MigrationError> {
    let scopes = resolve_scopes(&options.path, &options.profiles, options.all_profiles)?;
    let mut stats = MigrationStats::default();

    if scopes.len() > 1 {
        stats.push_note(format!(
            "Importing {} Hermes scopes. Runtime settings/secrets will be taken from the primary scope '{}'; conversations and markdown memory will be imported from all selected scopes.",
            scopes.len(),
            scopes.first().map(|scope| scope.name.as_str()).unwrap_or("default")
        ));
    }

    if let Some(primary) = scopes.first() {
        let config = read_config(&primary.root)?;
        let env = read_env_file(&primary.root.join(".env"))?;
        let auth = read_optional_json(&primary.root.join("auth.json"))?;
        let (settings_patch, secrets, notes) =
            map_primary_scope(&primary.name, &config, &env, auth.as_ref());
        for note in notes {
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
    }

    for scope in scopes {
        import_scope_markdown(services, options, &scope, &mut stats).await?;
        import_scope_sessions(services, options, &scope, &mut stats).await?;
    }

    Ok(stats)
}

fn resolve_scopes(
    root: &Path,
    profiles: &[String],
    all_profiles: bool,
) -> Result<Vec<HermesScope>, MigrationError> {
    if !root.exists() {
        return Err(MigrationError::NotFound {
            path: root.to_path_buf(),
            reason: "directory does not exist".to_string(),
        });
    }

    let mut scopes = Vec::new();
    if all_profiles {
        scopes.push(HermesScope {
            name: "default".to_string(),
            root: root.to_path_buf(),
        });
        let profiles_dir = root.join("profiles");
        if profiles_dir.exists() {
            let mut names = std::fs::read_dir(&profiles_dir)?
                .flatten()
                .filter(|entry| entry.path().is_dir())
                .filter_map(|entry| entry.file_name().to_str().map(|value| value.to_string()))
                .collect::<Vec<_>>();
            names.sort();
            for name in names {
                scopes.push(HermesScope {
                    name: name.clone(),
                    root: profiles_dir.join(name),
                });
            }
        }
        return Ok(scopes);
    }

    if profiles.is_empty() {
        scopes.push(HermesScope {
            name: "default".to_string(),
            root: root.to_path_buf(),
        });
        return Ok(scopes);
    }

    for profile in profiles {
        if profile == "default" {
            scopes.push(HermesScope {
                name: "default".to_string(),
                root: root.to_path_buf(),
            });
            continue;
        }
        let path = root.join("profiles").join(profile);
        if !path.exists() {
            return Err(MigrationError::NotFound {
                path,
                reason: format!("Hermes profile '{}' not found", profile),
            });
        }
        scopes.push(HermesScope {
            name: profile.clone(),
            root: root.join("profiles").join(profile),
        });
    }

    Ok(scopes)
}

fn read_config(root: &Path) -> Result<HermesConfigData, MigrationError> {
    let config_path = root.join("config.yaml");
    if !config_path.exists() {
        return Ok(HermesConfigData::default());
    }

    let content = std::fs::read_to_string(&config_path)?;
    let yaml: Value =
        serde_yml::from_str(&content).map_err(|e| MigrationError::ConfigParse(e.to_string()))?;

    let model_obj = yaml.get("model").and_then(Value::as_object);
    let model_provider = model_obj
        .and_then(|value| value.get("provider"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| {
            yaml.get("provider")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        });
    let default_model = model_obj
        .and_then(|value| value.get("default"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| {
            yaml.get("model")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        });
    let model_base_url = model_obj
        .and_then(|value| value.get("base_url"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| {
            yaml.get("base_url")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        });

    let mut providers = Vec::new();
    if let Some(map) = yaml.get("providers").and_then(Value::as_object) {
        for (provider_id, value) in map {
            let Some(entry) = value.as_object() else {
                continue;
            };
            let base_url = ["api", "url", "base_url"]
                .iter()
                .find_map(|key| entry.get(*key).and_then(Value::as_str))
                .map(ToString::to_string);
            let key_env = entry
                .get("key_env")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let transport = entry
                .get("transport")
                .or_else(|| entry.get("api_mode"))
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let model = entry
                .get("model")
                .or_else(|| entry.get("default_model"))
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let api_key = entry
                .get("api_key")
                .and_then(Value::as_str)
                .map(|value| SecretString::new(value.to_string().into_boxed_str()));
            providers.push(HermesProviderEntry {
                id: slugify(provider_id),
                name: entry
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(provider_id)
                    .to_string(),
                base_url,
                key_env,
                transport,
                model,
                api_key,
            });
        }
        providers.sort_by(|a, b| a.id.cmp(&b.id));
    }

    Ok(HermesConfigData {
        model_provider,
        default_model,
        model_base_url,
        providers,
    })
}

fn read_env_file(path: &Path) -> Result<HashMap<String, SecretString>, MigrationError> {
    let mut values = HashMap::new();
    if !path.exists() {
        return Ok(values);
    }
    for item in
        dotenvy::from_path_iter(path).map_err(|e| MigrationError::ConfigParse(e.to_string()))?
    {
        let (key, value) = item.map_err(|e| MigrationError::ConfigParse(e.to_string()))?;
        values.insert(key, SecretString::new(value.into_boxed_str()));
    }
    Ok(values)
}

fn read_optional_json(path: &Path) -> Result<Option<Value>, MigrationError> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)?;
    let value =
        serde_json::from_str(&content).map_err(|e| MigrationError::ConfigParse(e.to_string()))?;
    Ok(Some(value))
}

fn map_primary_scope(
    scope_name: &str,
    config: &HermesConfigData,
    env: &HashMap<String, SecretString>,
    auth: Option<&Value>,
) -> (HashMap<String, Value>, Vec<ImportedSecret>, Vec<String>) {
    let mut patch = HashMap::new();
    let mut secrets = Vec::new();
    let mut notes = Vec::new();

    let active_provider = config.model_provider.clone().or_else(|| {
        auth.and_then(|value| value.get("active_provider"))
            .and_then(Value::as_str)
            .map(ToString::to_string)
    });
    if config.model_provider.is_none() {
        if let Some(provider) = active_provider.as_ref() {
            notes.push(format!(
                "Hermes config had no active model.provider; used auth.json active_provider='{}'.",
                provider
            ));
        }
    }

    let mut custom_providers = Vec::new();
    let mut builtin_overrides: HashMap<String, LlmBuiltinOverride> = HashMap::new();

    for provider in &config.providers {
        if let Some(builtin) = normalize_builtin_provider(&provider.name)
            .or_else(|| normalize_builtin_provider(&provider.id))
        {
            if let Some(base_url) = provider.base_url.as_ref()
                && builtin != "openai_compatible"
                && builtin != "ollama"
            {
                builtin_overrides.insert(
                    builtin.clone(),
                    LlmBuiltinOverride {
                        api_key: None,
                        model: provider.model.clone(),
                        base_url: Some(base_url.clone()),
                    },
                );
            }
        } else if let Some(base_url) = provider.base_url.as_ref() {
            custom_providers.push(CustomLlmProviderSettings {
                id: provider.id.clone(),
                name: provider.name.clone(),
                adapter: provider_adapter(provider),
                base_url: Some(base_url.clone()),
                default_model: provider.model.clone(),
                api_key: None,
                builtin: false,
            });
        }

        if let Some(secret) = provider_secret(provider, env) {
            secrets.push(secret);
        }
    }

    if let Some(provider) = active_provider.as_deref() {
        if let Some(builtin) = normalize_builtin_provider(provider) {
            patch.insert("llm_backend".to_string(), Value::String(builtin.clone()));
            match builtin.as_str() {
                "ollama" => {
                    if let Some(url) = config
                        .model_base_url
                        .clone()
                        .or_else(|| provider_base_url(config, provider))
                    {
                        patch.insert("ollama_base_url".to_string(), Value::String(url));
                    }
                }
                "openai_compatible" => {
                    if let Some(url) = config
                        .model_base_url
                        .clone()
                        .or_else(|| provider_base_url(config, provider))
                    {
                        patch.insert("openai_compatible_base_url".to_string(), Value::String(url));
                    }
                }
                _ => {
                    if let Some(url) = config.model_base_url.clone() {
                        builtin_overrides.insert(
                            builtin,
                            LlmBuiltinOverride {
                                api_key: None,
                                model: config.default_model.clone(),
                                base_url: Some(url),
                            },
                        );
                    }
                }
            }
        } else if let Some(custom) = resolve_custom_provider(provider, &custom_providers) {
            patch.insert("llm_backend".to_string(), Value::String(custom.id.clone()));
        } else if provider.eq_ignore_ascii_case("custom") && custom_providers.len() == 1 {
            patch.insert(
                "llm_backend".to_string(),
                Value::String(custom_providers[0].id.clone()),
            );
        } else if let Some(url) = config.model_base_url.as_ref() {
            patch.insert(
                "llm_backend".to_string(),
                Value::String("openai_compatible".to_string()),
            );
            patch.insert(
                "openai_compatible_base_url".to_string(),
                Value::String(url.clone()),
            );
            notes.push(format!(
                "Hermes provider '{}' was treated as openai_compatible because it supplied only a base URL.",
                provider
            ));
        } else {
            notes.push(format!(
                "Hermes active provider '{}' could not be mapped onto a runnable IronClaw provider.",
                provider
            ));
        }
    }

    if let Some(model) = config.default_model.as_ref() {
        patch.insert("selected_model".to_string(), Value::String(model.clone()));
    }

    if !custom_providers.is_empty() {
        patch.insert(
            "llm_custom_providers".to_string(),
            serde_json::to_value(custom_providers).unwrap_or(Value::Null),
        );
    }
    if !builtin_overrides.is_empty() {
        patch.insert(
            "llm_builtin_overrides".to_string(),
            serde_json::to_value(builtin_overrides).unwrap_or(Value::Null),
        );
    }

    for (env_key, provider_id) in builtin_env_secret_mappings() {
        if let Some(value) = env.get(*env_key) {
            secrets.push(
                ImportedSecret::new(builtin_secret_name(provider_id), value.clone())
                    .with_provider(*provider_id),
            );
        }
    }

    if let Some(auth) = auth {
        secrets.push(
            ImportedSecret::new(
                format!("migrate_hermes_{}_auth_json", slugify(scope_name)),
                SecretString::new(auth.to_string().into_boxed_str()),
            )
            .with_provider("hermes"),
        );
        notes.push(
            "Stored raw Hermes auth.json as an encrypted backup secret. It is preserved for manual recovery but not auto-wired into an IronClaw provider."
                .to_string(),
        );
    }

    (patch, dedupe_secrets(secrets), notes)
}

fn provider_base_url(config: &HermesConfigData, provider: &str) -> Option<String> {
    let provider_slug = slugify(provider);
    config
        .providers
        .iter()
        .find(|entry| entry.id == provider_slug || entry.name.eq_ignore_ascii_case(provider))
        .and_then(|entry| entry.base_url.clone())
}

fn resolve_custom_provider<'a>(
    provider: &str,
    custom_providers: &'a [CustomLlmProviderSettings],
) -> Option<&'a CustomLlmProviderSettings> {
    let provider_slug = slugify(provider);
    custom_providers
        .iter()
        .find(|entry| entry.id == provider_slug || entry.name.eq_ignore_ascii_case(provider))
}

fn provider_adapter(provider: &HermesProviderEntry) -> String {
    let transport = provider
        .transport
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if transport.contains("anthropic") || provider.name.to_ascii_lowercase().contains("anthropic") {
        "anthropic".to_string()
    } else if transport.contains("ollama") || provider.name.to_ascii_lowercase().contains("ollama")
    {
        "ollama".to_string()
    } else {
        "open_ai_completions".to_string()
    }
}

fn provider_secret(
    provider: &HermesProviderEntry,
    env: &HashMap<String, SecretString>,
) -> Option<ImportedSecret> {
    let value = provider
        .key_env
        .as_ref()
        .and_then(|key| env.get(key).cloned())
        .or_else(|| provider.api_key.clone())?;

    if let Some(builtin) = normalize_builtin_provider(&provider.name)
        .or_else(|| normalize_builtin_provider(&provider.id))
    {
        if builtin == "ollama" || builtin == "bedrock" {
            return None;
        }
        return Some(
            ImportedSecret::new(builtin_secret_name(&builtin), value).with_provider(builtin),
        );
    }

    Some(
        ImportedSecret::new(custom_secret_name(&provider.id), value)
            .with_provider(provider.id.clone()),
    )
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

fn builtin_env_secret_mappings() -> &'static [(&'static str, &'static str)] {
    &[
        ("OPENAI_API_KEY", "openai"),
        ("ANTHROPIC_API_KEY", "anthropic"),
        ("NEARAI_API_KEY", "nearai"),
        ("TINFOIL_API_KEY", "tinfoil"),
        ("OPENAI_COMPATIBLE_API_KEY", "openai_compatible"),
    ]
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

async fn import_scope_markdown(
    services: &MigrationServices,
    options: &HermesMigrationOptions,
    scope: &HermesScope,
    stats: &mut MigrationStats,
) -> Result<(), MigrationError> {
    let file_sets = [
        scope.root.join("SOUL.md"),
        scope.root.join("memories"),
        scope.root.join("skills"),
        scope.root.join("workspace"),
        scope.root.join("plans"),
        scope.root.join("home"),
        scope.root.join("cron"),
    ];

    let mut seen = HashSet::new();
    for path in file_sets {
        if path.is_file() {
            let rel = normalize_relative_path(
                &path
                    .strip_prefix(&scope.root)
                    .unwrap_or(&path)
                    .to_string_lossy(),
            );
            if seen.insert(rel.clone()) {
                let content = std::fs::read_to_string(&path)?;
                let imported = ImportedDocument {
                    source: "hermes",
                    namespace: scope.name.clone(),
                    external_id: rel.clone(),
                    workspace_path: format!("imports/hermes/{}/{}", slugify(&scope.name), rel),
                    title: format!("Hermes {}: {}", scope.name, rel),
                    content,
                    tags: vec![
                        "migration".to_string(),
                        "hermes".to_string(),
                        scope.name.clone(),
                    ],
                    doc_type: ironclaw_engine::DocType::Note,
                    created_at: None,
                    metadata: json!({"scope": scope.name.clone(), "source_path": rel}),
                };
                if options.dry_run {
                    stats.workspace_documents += 1;
                    stats.memory_docs += 1;
                } else {
                    services.upsert_document(imported, stats).await?;
                }
            }
            continue;
        }

        for (rel, content) in collect_markdown_files(&path)? {
            let full_rel = if let Ok(stripped) = path.strip_prefix(&scope.root) {
                let base = normalize_relative_path(&stripped.to_string_lossy());
                if base.is_empty() {
                    rel.clone()
                } else {
                    format!("{base}/{rel}")
                }
            } else {
                rel.clone()
            };
            if !seen.insert(full_rel.clone()) {
                continue;
            }
            let imported = ImportedDocument {
                source: "hermes",
                namespace: scope.name.clone(),
                external_id: full_rel.clone(),
                workspace_path: format!("imports/hermes/{}/{}", slugify(&scope.name), full_rel),
                title: format!("Hermes {}: {}", scope.name, full_rel),
                content,
                tags: vec![
                    "migration".to_string(),
                    "hermes".to_string(),
                    scope.name.clone(),
                ],
                doc_type: ironclaw_engine::DocType::Note,
                created_at: None,
                metadata: json!({"scope": scope.name.clone(), "source_path": full_rel}),
            };
            if options.dry_run {
                stats.workspace_documents += 1;
                stats.memory_docs += 1;
            } else {
                services.upsert_document(imported, stats).await?;
            }
        }
    }

    Ok(())
}

async fn import_scope_sessions(
    services: &MigrationServices,
    options: &HermesMigrationOptions,
    scope: &HermesScope,
    stats: &mut MigrationStats,
) -> Result<(), MigrationError> {
    let state_db = scope.root.join("state.db");
    if !state_db.exists() {
        stats.push_note(format!(
            "Hermes scope '{}' has no state.db; skipped session history import.",
            scope.name
        ));
        return Ok(());
    }

    let sessions = read_sessions(&state_db).await?;
    for session in sessions {
        let title = session
            .title
            .clone()
            .unwrap_or_else(|| session_title(&session));
        let messages = session
            .messages
            .iter()
            .map(|message| ImportedMessage {
                role: hermes_message_role(message),
                content: hermes_message_content(message),
                timestamp: message.timestamp,
            })
            .collect::<Vec<_>>();

        let imported = ImportedConversation {
            source: "hermes",
            namespace: scope.name.clone(),
            external_id: session.id.clone(),
            source_channel: session
                .source
                .clone()
                .unwrap_or_else(|| format!("hermes:{}", scope.name)),
            title,
            created_at: session.started_at,
            messages,
            metadata: json!({
                "scope": scope.name.clone(),
                "hermes_user_id": session.user_id,
                "model": session.model,
                "source": session.source,
            }),
        };

        if options.dry_run {
            stats.engine_threads += 1;
            stats.engine_conversations += 1;
            stats.legacy_conversations += 1;
            stats.messages += imported.messages.len();
        } else {
            services.upsert_conversation(imported, stats).await?;
        }
    }

    Ok(())
}

fn session_title(session: &HermesSession) -> String {
    if let Some(first_user) = session
        .messages
        .iter()
        .find(|message| message.role.eq_ignore_ascii_case("user"))
    {
        let mut title: String = first_user.content.chars().take(80).collect();
        if first_user.content.chars().count() > 80 {
            title.push('…');
        }
        return title;
    }
    format!("Hermes session {}", session.id)
}

fn hermes_message_role(message: &HermesMessage) -> ImportedMessageRole {
    match message.role.to_ascii_lowercase().as_str() {
        "user" => ImportedMessageRole::User,
        "assistant" => ImportedMessageRole::Assistant,
        "system" => ImportedMessageRole::System,
        _ => ImportedMessageRole::Tool {
            name: message
                .tool_name
                .clone()
                .or_else(|| Some(message.role.clone())),
        },
    }
}

fn hermes_message_content(message: &HermesMessage) -> String {
    if message.content.is_empty() {
        if let Some(tool) = message.tool_name.as_ref() {
            return format!("[{tool}] (empty output)");
        }
    }
    message.content.clone()
}

async fn read_sessions(db_path: &Path) -> Result<Vec<HermesSession>, MigrationError> {
    let db = libsql::Builder::new_local(db_path)
        .build()
        .await
        .map_err(|e| MigrationError::Sqlite(e.to_string()))?;
    let conn = db
        .connect()
        .map_err(|e| MigrationError::Sqlite(e.to_string()))?;

    let session_columns = table_columns(&conn, "sessions").await?;
    if session_columns.is_empty() {
        return Ok(Vec::new());
    }
    let message_columns = table_columns(&conn, "messages").await?;

    let order_expr = if session_columns.contains("started_at") {
        "CAST(started_at AS TEXT)"
    } else if session_columns.contains("created_at") {
        "CAST(created_at AS TEXT)"
    } else {
        "CAST(id AS TEXT)"
    };

    let query = format!(
        "SELECT \
            {} , {} , {} , {} , {} , {} \
         FROM sessions ORDER BY {}",
        select_text_expr(&session_columns, &["id"], "id"),
        select_text_expr(&session_columns, &["source"], "source"),
        select_text_expr(&session_columns, &["user_id"], "user_id"),
        select_text_expr(&session_columns, &["model"], "model"),
        select_text_expr(&session_columns, &["title"], "title"),
        select_text_expr(
            &session_columns,
            &["started_at", "created_at"],
            "started_at"
        ),
        order_expr,
    );

    let mut rows = conn
        .query(&query, ())
        .await
        .map_err(|e| MigrationError::Sqlite(e.to_string()))?;

    let mut sessions = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| MigrationError::Sqlite(e.to_string()))?
    {
        let id: Option<String> = row
            .get(0)
            .map_err(|e| MigrationError::Sqlite(e.to_string()))?;
        let Some(id) = id else { continue };
        let source: Option<String> = row
            .get(1)
            .map_err(|e| MigrationError::Sqlite(e.to_string()))?;
        let user_id: Option<String> = row
            .get(2)
            .map_err(|e| MigrationError::Sqlite(e.to_string()))?;
        let model: Option<String> = row
            .get(3)
            .map_err(|e| MigrationError::Sqlite(e.to_string()))?;
        let title: Option<String> = row
            .get(4)
            .map_err(|e| MigrationError::Sqlite(e.to_string()))?;
        let started_at: Option<String> = row
            .get(5)
            .map_err(|e| MigrationError::Sqlite(e.to_string()))?;

        let messages = read_messages(&conn, &message_columns, &id).await?;
        sessions.push(HermesSession {
            id,
            source,
            user_id,
            model,
            title,
            started_at: started_at.as_deref().and_then(parse_timestamp_str),
            messages,
        });
    }

    Ok(sessions)
}

async fn read_messages(
    conn: &libsql::Connection,
    message_columns: &HashSet<String>,
    session_id: &str,
) -> Result<Vec<HermesMessage>, MigrationError> {
    if message_columns.is_empty() {
        return Ok(Vec::new());
    }

    let query = format!(
        "SELECT \
            {} , {} , {} , {} \
         FROM messages WHERE session_id = ?1 ORDER BY {}",
        select_text_expr(message_columns, &["role"], "role"),
        select_text_expr(message_columns, &["content"], "content"),
        select_text_expr(message_columns, &["timestamp", "created_at"], "timestamp"),
        select_text_expr(message_columns, &["tool_name"], "tool_name"),
        if message_columns.contains("timestamp") {
            "CAST(timestamp AS TEXT)"
        } else if message_columns.contains("created_at") {
            "CAST(created_at AS TEXT)"
        } else {
            "rowid"
        }
    );

    let mut rows = conn
        .query(&query, libsql::params![session_id])
        .await
        .map_err(|e| MigrationError::Sqlite(e.to_string()))?;

    let mut messages = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| MigrationError::Sqlite(e.to_string()))?
    {
        let role: Option<String> = row
            .get(0)
            .map_err(|e| MigrationError::Sqlite(e.to_string()))?;
        let content: Option<String> = row
            .get(1)
            .map_err(|e| MigrationError::Sqlite(e.to_string()))?;
        let timestamp: Option<String> = row
            .get(2)
            .map_err(|e| MigrationError::Sqlite(e.to_string()))?;
        let tool_name: Option<String> = row
            .get(3)
            .map_err(|e| MigrationError::Sqlite(e.to_string()))?;
        messages.push(HermesMessage {
            role: role.unwrap_or_else(|| "assistant".to_string()),
            content: content.unwrap_or_default(),
            timestamp: timestamp.as_deref().and_then(parse_timestamp_str),
            tool_name,
        });
    }

    Ok(messages)
}

async fn table_columns(
    conn: &libsql::Connection,
    table: &str,
) -> Result<HashSet<String>, MigrationError> {
    let mut rows = conn
        .query(&format!("PRAGMA table_info({table})"), ())
        .await
        .map_err(|e| MigrationError::Sqlite(e.to_string()))?;
    let mut columns = HashSet::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| MigrationError::Sqlite(e.to_string()))?
    {
        let name: String = row
            .get(1)
            .map_err(|e| MigrationError::Sqlite(e.to_string()))?;
        columns.insert(name);
    }
    Ok(columns)
}

fn select_text_expr(columns: &HashSet<String>, candidates: &[&str], alias: &str) -> String {
    let available = candidates
        .iter()
        .filter(|candidate| columns.contains(**candidate))
        .map(|candidate| format!("CAST({candidate} AS TEXT)"))
        .collect::<Vec<_>>();
    if available.is_empty() {
        format!("NULL AS {alias}")
    } else if available.len() == 1 {
        format!("{} AS {alias}", available[0])
    } else {
        format!("COALESCE({}) AS {alias}", available.join(", "))
    }
}
