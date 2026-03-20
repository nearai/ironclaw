//! Conversation history import infrastructure and import CLI command.

use std::cmp::Reverse;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use clap::{Args, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

use crate::config::Config;

#[cfg(feature = "import")]
use crate::import::ImportOptions;
#[cfg(feature = "import")]
use crate::import::openclaw::OpenClawImporter;

/// A single message parsed from an external source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedMessage {
    /// "user", "assistant", "system", "tool"
    pub role: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

/// A conversation parsed from an external source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedConversation {
    /// Stable ID from the source (for dedup on re-import).
    pub source_id: String,
    /// Human-readable title.
    pub title: Option<String>,
    /// Ordered messages.
    pub messages: Vec<ImportedMessage>,
    /// Source timestamp normalized to UTC.
    pub created_at: DateTime<Utc>,
    /// Last activity timestamp normalized to UTC.
    pub last_activity: DateTime<Utc>,
    /// Source-specific metadata preserved verbatim.
    pub source_metadata: serde_json::Value,
}

/// Supported conversation-history sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ImportSource {
    /// Local Claude Code CLI history (~/.claude/projects/).
    #[value(name = "claude-code")]
    ClaudeCode,
    /// Claude.ai browser export ZIP.
    #[value(name = "claude-web")]
    ClaudeWeb,
    /// Local OpenAI Codex CLI history.
    #[value(name = "codex-cli")]
    CodexCli,
    /// ChatGPT data export ZIP.
    #[value(name = "chatgpt")]
    ChatGpt,
    /// Google Takeout Gemini export.
    #[value(name = "gemini")]
    Gemini,
}

impl ImportSource {
    pub fn source_key(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude_code",
            Self::ClaudeWeb => "claude_web",
            Self::CodexCli => "codex_cli",
            Self::ChatGpt => "chatgpt",
            Self::Gemini => "gemini",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::ClaudeWeb => "Claude Web",
            Self::CodexCli => "Codex CLI",
            Self::ChatGpt => "ChatGPT",
            Self::Gemini => "Gemini",
        }
    }

    /// Whether this build includes a parser for this history source.
    pub fn supports_history_import(self) -> bool {
        match self {
            Self::ClaudeCode | Self::ClaudeWeb | Self::CodexCli | Self::ChatGpt | Self::Gemini => {
                false
            }
        }
    }
}

/// Source candidate discovered during onboarding auto-detection.
#[derive(Debug, Clone)]
pub struct AutoDetectedImportSource {
    pub source: ImportSource,
    pub path: PathBuf,
    pub note: String,
}

/// Trait that all source-specific parsers implement.
pub trait Importer: Send + Sync {
    /// Human-readable name for progress messages.
    fn source_name(&self) -> &str;

    /// Parse the input path into conversations.
    /// Must not touch the database.
    fn parse(&self, path: &Path) -> Result<Vec<ImportedConversation>, ImportError>;
}

/// Import parser / orchestration errors.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("file not found: {path}")]
    NotFound { path: String },
    #[error("unsupported format: {reason}")]
    UnsupportedFormat { reason: String },
    #[error("parse error: {reason}")]
    Parse { reason: String },
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database error: {0}")]
    Database(String),
    #[error("ZIP error: {0}")]
    Zip(String),
}

/// Shared flags for conversation-history import commands.
#[derive(Args, Debug, Clone)]
pub struct HistoryImportArgs {
    /// Path to export file, directory, or ZIP.
    /// If omitted, uses the default location for the source.
    pub path: Option<PathBuf>,

    /// User ID to associate imported conversations with.
    #[arg(long, default_value = "default")]
    pub user_id: String,

    /// Skip conversations that have already been imported.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub dedup: bool,

    /// Also write conversations to workspace memory as markdown.
    #[arg(long)]
    pub to_workspace: bool,

    /// Dry run: parse and report without writing.
    #[arg(long)]
    pub dry_run: bool,
}

/// Import data from other AI systems.
#[derive(Subcommand, Debug, Clone)]
pub enum ImportCommand {
    /// Import conversation history from local Claude Code sessions.
    #[command(name = "claude-code")]
    ClaudeCode(HistoryImportArgs),

    /// Import conversation history from a Claude.ai export ZIP.
    #[command(name = "claude-web")]
    ClaudeWeb(HistoryImportArgs),

    /// Import conversation history from local Codex CLI sessions.
    #[command(name = "codex-cli")]
    CodexCli(HistoryImportArgs),

    /// Import conversation history from a ChatGPT export ZIP.
    #[command(name = "chatgpt")]
    ChatGpt(HistoryImportArgs),

    /// Import conversation history from Google Takeout Gemini exports.
    #[command(name = "gemini")]
    Gemini(HistoryImportArgs),

    /// Import from OpenClaw (memory, history, settings, credentials).
    #[cfg(feature = "import")]
    Openclaw {
        /// Path to OpenClaw directory (default: ~/.openclaw).
        #[arg(long)]
        path: Option<PathBuf>,

        /// Dry-run mode: show what would be imported without writing.
        #[arg(long)]
        dry_run: bool,

        /// Re-embed memory if dimensions don't match target provider.
        #[arg(long)]
        re_embed: bool,

        /// User ID for imported data (default: 'default').
        #[arg(long)]
        user_id: Option<String>,
    },
}

impl ImportCommand {
    fn as_history_request(&self) -> Option<(ImportSource, &HistoryImportArgs)> {
        match self {
            Self::ClaudeCode(args) => Some((ImportSource::ClaudeCode, args)),
            Self::ClaudeWeb(args) => Some((ImportSource::ClaudeWeb, args)),
            Self::CodexCli(args) => Some((ImportSource::CodexCli, args)),
            Self::ChatGpt(args) => Some((ImportSource::ChatGpt, args)),
            Self::Gemini(args) => Some((ImportSource::Gemini, args)),
            #[cfg(feature = "import")]
            Self::Openclaw { .. } => None,
        }
    }
}

#[derive(Debug, Default)]
struct ImportStats {
    parsed_conversations: usize,
    parsed_messages: usize,
    imported_conversations: usize,
    imported_messages: usize,
    skipped_duplicates: usize,
}

/// Run `ironclaw import ...`.
pub async fn run_import_command(
    cmd: &ImportCommand,
    config: &Config,
    no_db: bool,
) -> anyhow::Result<()> {
    #[cfg(feature = "import")]
    if let ImportCommand::Openclaw {
        path,
        dry_run,
        re_embed,
        user_id,
    } = cmd
    {
        if no_db {
            return Err(anyhow::anyhow!(
                "--no-db is not supported for the openclaw import source"
            ));
        }
        return run_import_openclaw(config, path.clone(), *dry_run, *re_embed, user_id.clone())
            .await;
    }

    let (source, args) = cmd
        .as_history_request()
        .ok_or_else(|| anyhow::anyhow!("Unsupported import command"))?;
    run_history_import_command(source, args, config, no_db).await
}

async fn run_history_import_command(
    source: ImportSource,
    args: &HistoryImportArgs,
    config: &Config,
    no_db: bool,
) -> anyhow::Result<()> {
    if args.dry_run {
        let (_, stats) = parse_history_source(source, args)?;
        println!(
            "Dry run: parsed {} conversation(s), {} message(s) from {}",
            stats.parsed_conversations,
            stats.parsed_messages,
            source.display_name()
        );
        return Ok(());
    }

    if no_db {
        return Err(anyhow::anyhow!(
            "--no-db is only supported with --dry-run for history imports"
        ));
    }

    let db = crate::db::connect_from_config(&config.database)
        .await
        .map_err(|err| anyhow::anyhow!("Failed to initialize database: {}", err))?;

    run_import_command_with_db(source, args, db).await
}

pub async fn run_import_command_with_db(
    source: ImportSource,
    args: &HistoryImportArgs,
    db: Arc<dyn crate::db::Database>,
) -> anyhow::Result<()> {
    let (conversations, mut stats) = parse_history_source(source, args)?;
    if args.dry_run {
        println!(
            "Dry run: parsed {} conversation(s), {} message(s) from {}",
            stats.parsed_conversations,
            stats.parsed_messages,
            source.display_name()
        );
        return Ok(());
    }

    let workspace = if args.to_workspace {
        Some(crate::workspace::Workspace::new_with_db(
            args.user_id.clone(),
            db.clone(),
        ))
    } else {
        None
    };

    for conversation in conversations {
        if args.dedup {
            let existing = db
                .find_conversation_by_import_source(
                    &args.user_id,
                    source.source_key(),
                    &conversation.source_id,
                )
                .await
                .map_err(|err| anyhow::anyhow!("Dedup query failed: {}", err))?;
            if existing.is_some() {
                stats.skipped_duplicates += 1;
                continue;
            }
        }

        let metadata = serde_json::json!({
            "import_source": source.source_key(),
            "import_source_id": conversation.source_id,
            "import_title": conversation.title,
            "import_created_at": conversation.created_at.to_rfc3339(),
            "import_last_activity": conversation.last_activity.to_rfc3339(),
            "import_metadata": conversation.source_metadata,
        });

        let conversation_id = db
            .create_conversation_with_metadata_and_timestamps(
                "imported",
                &args.user_id,
                &metadata,
                conversation.created_at,
                conversation.last_activity,
            )
            .await
            .map_err(|err| anyhow::anyhow!("Failed to create imported conversation: {}", err))?;

        for (index, message) in conversation.messages.iter().enumerate() {
            db.add_conversation_message_with_metadata(
                conversation_id,
                &message.role,
                &message.content,
                message.created_at,
                Some(index as i64),
            )
            .await
            .map_err(|err| anyhow::anyhow!("Failed to add imported message: {}", err))?;
            stats.imported_messages += 1;
        }

        stats.imported_conversations += 1;

        if let Some(workspace) = workspace.as_ref() {
            write_conversation_to_workspace(workspace, source, &conversation)
                .await
                .map_err(|err| anyhow::anyhow!("Failed writing workspace memory: {}", err))?;
        }
    }

    print_history_import_summary(source, &stats);

    Ok(())
}

fn parse_history_source(
    source: ImportSource,
    args: &HistoryImportArgs,
) -> anyhow::Result<(Vec<ImportedConversation>, ImportStats)> {
    let source_path = resolve_source_path(source, args.path.as_deref())?;
    let importer = importer_for_source(source);
    let conversations = importer
        .parse(&source_path)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let stats = ImportStats {
        parsed_conversations: conversations.len(),
        parsed_messages: conversations
            .iter()
            .map(|conversation| conversation.messages.len())
            .sum(),
        ..ImportStats::default()
    };
    Ok((conversations, stats))
}

fn print_history_import_summary(source: ImportSource, stats: &ImportStats) {
    println!(
        "Imported {} conversation(s) ({} messages) from {}. Skipped {} duplicate(s).",
        stats.imported_conversations,
        stats.imported_messages,
        source.display_name(),
        stats.skipped_duplicates
    );
    println!(
        "Parsed total: {} conversation(s), {} message(s)",
        stats.parsed_conversations, stats.parsed_messages
    );
}

/// Auto-detect likely import sources for onboarding.
pub fn autodetect_import_sources() -> Vec<AutoDetectedImportSource> {
    let mut detected = Vec::new();

    if ImportSource::ClaudeCode.supports_history_import()
        && let Some(path) = default_claude_code_path().filter(|path| path.exists())
    {
        let file_count = count_files_with_extension(&path, "jsonl", 10_000);
        if file_count > 0 {
            detected.push(AutoDetectedImportSource {
                source: ImportSource::ClaudeCode,
                path,
                note: format!("{} JSONL file(s)", file_count),
            });
        }
    }

    if ImportSource::CodexCli.supports_history_import()
        && let Some(path) = default_codex_cli_path().filter(|path| path.exists())
    {
        let file_count = count_files_with_extension(&path, "jsonl", 10_000)
            + count_files_with_extension(&path, "json", 10_000);
        if file_count > 0 {
            detected.push(AutoDetectedImportSource {
                source: ImportSource::CodexCli,
                path,
                note: format!("{} history file(s)", file_count),
            });
        }
    }

    if ImportSource::ClaudeWeb.supports_history_import()
        && let Some(path) = detect_latest_zip(|name| {
            let lower = name.to_ascii_lowercase();
            lower.contains("claude") && lower.ends_with(".zip")
        })
    {
        detected.push(AutoDetectedImportSource {
            source: ImportSource::ClaudeWeb,
            path,
            note: "Found matching ZIP in Downloads".to_string(),
        });
    }

    if ImportSource::ChatGpt.supports_history_import()
        && let Some(path) = detect_latest_zip(|name| {
            let lower = name.to_ascii_lowercase();
            (lower.contains("chatgpt") || lower.contains("openai")) && lower.ends_with(".zip")
        })
    {
        detected.push(AutoDetectedImportSource {
            source: ImportSource::ChatGpt,
            path,
            note: "Found matching ZIP in Downloads".to_string(),
        });
    }

    if ImportSource::Gemini.supports_history_import()
        && let Some(path) = detect_latest_zip(|name| {
            let lower = name.to_ascii_lowercase();
            (lower.contains("takeout") || lower.contains("gemini")) && lower.ends_with(".zip")
        })
    {
        detected.push(AutoDetectedImportSource {
            source: ImportSource::Gemini,
            path,
            note: "Found matching ZIP in Downloads".to_string(),
        });
    }

    detected
}

pub fn has_supported_history_import_sources() -> bool {
    [
        ImportSource::ClaudeCode,
        ImportSource::ClaudeWeb,
        ImportSource::CodexCli,
        ImportSource::ChatGpt,
        ImportSource::Gemini,
    ]
    .into_iter()
    .any(ImportSource::supports_history_import)
}

fn resolve_source_path(
    source: ImportSource,
    explicit: Option<&Path>,
) -> Result<PathBuf, ImportError> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }

    match source {
        ImportSource::ClaudeCode => {
            default_claude_code_path().ok_or_else(|| ImportError::NotFound {
                path: "~/.claude/projects".to_string(),
            })
        }
        ImportSource::CodexCli => default_codex_cli_path().ok_or_else(|| ImportError::NotFound {
            path: "~/.codex/sessions or ~/.config/codex/sessions".to_string(),
        }),
        ImportSource::ClaudeWeb | ImportSource::ChatGpt | ImportSource::Gemini => {
            autodetect_import_sources()
                .into_iter()
                .find(|source_candidate| source_candidate.source == source)
                .map(|source_candidate| source_candidate.path)
                .ok_or_else(|| ImportError::NotFound {
                    path: "~/Downloads/*.zip".to_string(),
                })
        }
    }
}

fn importer_for_source(source: ImportSource) -> Box<dyn Importer> {
    Box::new(UnimplementedImporter { source })
}

struct UnimplementedImporter {
    source: ImportSource,
}

impl Importer for UnimplementedImporter {
    fn source_name(&self) -> &str {
        self.source.display_name()
    }

    fn parse(&self, _path: &Path) -> Result<Vec<ImportedConversation>, ImportError> {
        Err(ImportError::UnsupportedFormat {
            reason: format!(
                "{} parser is not implemented yet. Add source parser in its dedicated issue.",
                self.source_name()
            ),
        })
    }
}

fn default_claude_code_path() -> Option<PathBuf> {
    home_dir().map(|home| home.join(".claude").join("projects"))
}

fn default_codex_cli_path() -> Option<PathBuf> {
    let home = home_dir()?;
    let primary = home.join(".codex").join("sessions");
    if primary.exists() {
        return Some(primary);
    }

    let fallback = home.join(".config").join("codex").join("sessions");
    if fallback.exists() {
        return Some(fallback);
    }

    Some(primary)
}

fn home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

fn detect_latest_zip(predicate: impl Fn(&str) -> bool) -> Option<PathBuf> {
    let downloads_dir = home_dir()?.join("Downloads");
    let entries = fs::read_dir(downloads_dir).ok()?;

    let mut matches = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !predicate(name) {
            continue;
        }

        let modified = entry
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        matches.push((modified, path));
    }

    matches.sort_by_key(|(modified, _)| Reverse(*modified));
    matches.into_iter().map(|(_, path)| path).next()
}

fn count_files_with_extension(root: &Path, extension: &str, limit: usize) -> usize {
    let mut stack = vec![root.to_path_buf()];
    let mut count = 0usize;

    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }

            if path
                .extension()
                .and_then(|value| value.to_str())
                .map(|value| value.eq_ignore_ascii_case(extension))
                .unwrap_or(false)
            {
                count += 1;
                if count >= limit {
                    return count;
                }
            }
        }
    }

    count
}

async fn write_conversation_to_workspace(
    workspace: &crate::workspace::Workspace,
    source: ImportSource,
    conversation: &ImportedConversation,
) -> Result<(), ImportError> {
    let path = workspace_document_path(source, conversation);

    let mut body = String::new();
    body.push_str("# ");
    body.push_str(
        conversation
            .title
            .as_deref()
            .filter(|title| !title.trim().is_empty())
            .unwrap_or("Imported Conversation"),
    );
    body.push_str("\n\n");
    body.push_str("- Source: ");
    body.push_str(source.display_name());
    body.push('\n');
    body.push_str("- Source ID: ");
    body.push_str(&conversation.source_id);
    body.push('\n');
    body.push_str("- Created At: ");
    body.push_str(&conversation.created_at.to_rfc3339());
    body.push('\n');
    body.push_str("- Last Activity: ");
    body.push_str(&conversation.last_activity.to_rfc3339());
    body.push_str("\n\n");

    for message in &conversation.messages {
        body.push_str("## ");
        body.push_str(&message.role);
        body.push_str("\n\n");
        body.push_str(&message.content);
        body.push_str("\n\n");
    }

    workspace
        .write(&path, &body)
        .await
        .map_err(|err| ImportError::Database(err.to_string()))?;

    Ok(())
}

fn workspace_document_path(source: ImportSource, conversation: &ImportedConversation) -> String {
    let slug_base = conversation
        .title
        .as_deref()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or(&conversation.source_id);

    format!(
        "imported/{}/{}-{}.md",
        source.source_key(),
        slugify(slug_base),
        source_id_suffix(&conversation.source_id)
    )
}

fn source_id_suffix(source_id: &str) -> String {
    let mut hasher = DefaultHasher::new();
    source_id.hash(&mut hasher);
    format!("{:012x}", hasher.finish() & 0x0000_ffff_ffff_ffff)
}

fn slugify(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut previous_dash = false;

    for character in input.chars() {
        if character.is_ascii_alphanumeric() {
            output.push(character.to_ascii_lowercase());
            previous_dash = false;
        } else if !previous_dash {
            output.push('-');
            previous_dash = true;
        }
    }

    let output = output.trim_matches('-');
    if output.is_empty() {
        "conversation".to_string()
    } else {
        output.to_string()
    }
}

#[cfg(feature = "import")]
async fn run_import_openclaw(
    config: &Config,
    openclaw_path: Option<PathBuf>,
    dry_run: bool,
    re_embed: bool,
    user_id: Option<String>,
) -> anyhow::Result<()> {
    use secrecy::SecretString;

    let openclaw_path = if let Some(path) = openclaw_path {
        path
    } else if let Some(path) = OpenClawImporter::detect() {
        path
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".openclaw")
    };

    let user_id = user_id.unwrap_or_else(|| "default".to_string());

    println!("OpenClaw Import");
    println!("  Path: {}", openclaw_path.display());
    println!("  User: {}", user_id);
    if dry_run {
        println!("  Mode: DRY RUN (no data will be written)");
    }
    println!();

    let db = crate::db::connect_from_config(&config.database)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to initialize database: {}", e))?;

    let secrets_crypto = if let Ok(master_key_hex) = std::env::var("SECRETS_MASTER_KEY") {
        Arc::new(
            crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(master_key_hex))
                .map_err(|e| anyhow::anyhow!("Failed to initialize secrets: {}", e))?,
        )
    } else {
        match crate::secrets::keychain::get_master_key().await {
            Ok(key_bytes) => {
                let key_hex: String = key_bytes.iter().map(|b| format!("{:02x}", b)).collect();
                Arc::new(
                    crate::secrets::SecretsCrypto::new(SecretString::from(key_hex))
                        .map_err(|e| anyhow::anyhow!("Failed to initialize secrets: {}", e))?,
                )
            }
            Err(_) => {
                return Err(anyhow::anyhow!(
                    "No secrets master key found. Set SECRETS_MASTER_KEY env var or run 'ironclaw onboard' first."
                ));
            }
        }
    };

    let secrets: Arc<dyn crate::secrets::SecretsStore> = Arc::new(
        crate::secrets::InMemorySecretsStore::new(secrets_crypto.clone()),
    );

    let workspace = crate::workspace::Workspace::new_with_db(user_id.clone(), db.clone());

    let opts = ImportOptions {
        openclaw_path,
        dry_run,
        re_embed,
        user_id,
    };

    let importer = OpenClawImporter::new(db, workspace, secrets, opts);
    let stats = importer.import().await?;

    println!("Import Complete");
    println!();
    println!("Summary:");
    println!("  Documents:    {}", stats.documents);
    println!("  Chunks:       {}", stats.chunks);
    println!("  Conversations: {}", stats.conversations);
    println!("  Messages:     {}", stats.messages);
    println!("  Settings:     {}", stats.settings);
    println!("  Secrets:      {}", stats.secrets);
    if stats.skipped > 0 {
        println!("  Skipped:      {}", stats.skipped);
    }
    if stats.re_embed_queued > 0 {
        println!("  Re-embed queued: {}", stats.re_embed_queued);
    }
    println!();
    println!("Total imported: {}", stats.total_imported());

    if dry_run {
        println!();
        println!("[DRY RUN] No data was written.");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::workspace_document_path;
    use chrono::DateTime;
    use clap::ValueEnum;

    use crate::cli::import::{ImportSource, autodetect_import_sources};
    use crate::cli::import::{ImportedConversation, ImportedMessage, slugify};
    use chrono::Utc;

    #[test]
    fn imported_conversation_serde_roundtrip() {
        let created_at = DateTime::parse_from_rfc3339("2024-01-15T10:00:00Z")
            .expect("valid timestamp")
            .with_timezone(&Utc);
        let last_activity = DateTime::parse_from_rfc3339("2024-01-15T10:05:00Z")
            .expect("valid timestamp")
            .with_timezone(&Utc);

        let conversation = ImportedConversation {
            source_id: "conv-123".to_string(),
            title: Some("Hello".to_string()),
            messages: vec![ImportedMessage {
                role: "user".to_string(),
                content: "Hi".to_string(),
                created_at,
            }],
            created_at,
            last_activity,
            source_metadata: serde_json::json!({"k": "v"}),
        };

        let encoded = serde_json::to_string(&conversation).expect("serialize");
        let decoded: ImportedConversation = serde_json::from_str(&encoded).expect("deserialize");

        assert_eq!(decoded.source_id, conversation.source_id);
        assert_eq!(decoded.title, conversation.title);
        assert_eq!(decoded.messages.len(), 1);
        assert_eq!(decoded.messages[0].role, "user");
        assert_eq!(decoded.messages[0].content, "Hi");
        assert_eq!(decoded.created_at, conversation.created_at);
        assert_eq!(decoded.last_activity, conversation.last_activity);
        assert_eq!(decoded.source_metadata["k"], "v");
    }

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Hello World"), "hello-world");
        assert_eq!(slugify("   "), "conversation");
    }

    #[test]
    fn import_source_kebab_names_are_stable() {
        let values = ImportSource::value_variants()
            .iter()
            .filter_map(|value| value.to_possible_value())
            .map(|value| value.get_name().to_string())
            .collect::<Vec<_>>();

        assert!(values.contains(&"claude-code".to_string()));
        assert!(values.contains(&"claude-web".to_string()));
        assert!(values.contains(&"codex-cli".to_string()));
        assert!(values.contains(&"chatgpt".to_string()));
        assert!(values.contains(&"gemini".to_string()));
    }

    #[test]
    fn autodetect_is_safe_when_nothing_exists() {
        let detected = autodetect_import_sources();
        assert!(detected.len() <= 5);
        assert!(
            detected
                .iter()
                .all(|candidate| candidate.source.supports_history_import())
        );
    }

    #[test]
    fn workspace_document_paths_are_unique_for_duplicate_titles() {
        let timestamp = DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .expect("valid timestamp")
            .with_timezone(&Utc);

        let first = ImportedConversation {
            source_id: "conv-1".to_string(),
            title: Some("Repeated Title".to_string()),
            messages: Vec::new(),
            created_at: timestamp,
            last_activity: timestamp,
            source_metadata: serde_json::json!({}),
        };
        let second = ImportedConversation {
            source_id: "conv-2".to_string(),
            title: Some("Repeated Title".to_string()),
            messages: Vec::new(),
            created_at: timestamp,
            last_activity: timestamp,
            source_metadata: serde_json::json!({}),
        };

        let first_path = workspace_document_path(ImportSource::ChatGpt, &first);
        let second_path = workspace_document_path(ImportSource::ChatGpt, &second);

        assert!(first_path.starts_with("imported/chatgpt/repeated-title-"));
        assert!(second_path.starts_with("imported/chatgpt/repeated-title-"));
        assert_ne!(first_path, second_path);
    }
}
