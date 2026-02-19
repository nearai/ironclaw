//! Conversation history import CLI command.
//!
//! Imports external chat exports into IronClaw conversations.

use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::fs::{File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, NaiveDateTime, Utc};
use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};

use crate::cli::import_claude_code::{ClaudeCodeImporter, default_claude_code_path};
use crate::cli::import_claude_web::ClaudeWebImporter;
use crate::config::Config;
use crate::db::Database;

/// A parsed message from an external history source.
#[derive(Debug, Clone)]
pub struct ImportedMessage {
    pub role: String,
    pub content: String,
    pub timestamp: Option<DateTime<Utc>>,
    pub source_metadata: serde_json::Value,
}

/// A parsed conversation from an external history source.
#[derive(Debug, Clone)]
pub struct ImportedConversation {
    pub source_id: String,
    pub title: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub messages: Vec<ImportedMessage>,
    pub source_metadata: serde_json::Value,
}

/// Parser contract for import sources.
pub trait Importer: Send + Sync + 'static {
    fn source_key(&self) -> &str;
    fn source_name(&self) -> &str;

    fn parse_stream<F>(
        &self,
        path: &Path,
        on_conversation: F,
    ) -> Result<ImportParseSummary, ImportError>
    where
        F: FnMut(ImportedConversation) -> Result<(), ImportError>;

    #[allow(dead_code)]
    fn parse(&self, path: &Path) -> Result<Vec<ImportedConversation>, ImportError> {
        let mut conversations = Vec::new();
        self.parse_stream(path, |conversation: ImportedConversation| {
            conversations.push(conversation);
            Ok(())
        })?;
        Ok(conversations)
    }
}

/// Import parser errors.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("ZIP error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("Parse error: {0}")]
    Parse(String),
}

#[derive(Subcommand, Debug, Clone)]
pub enum ImportCommand {
    /// Import local Claude Code history from ~/.claude/projects/
    ClaudeCode {
        /// Path to Claude Code projects dir (default: ~/.claude/projects/)
        #[arg(long)]
        path: Option<PathBuf>,

        /// Shared import flags (dry-run/checkpoint/channel/user).
        #[command(flatten)]
        common: ImportCommonArgs,
    },

    /// Import Claude.ai export ZIP (Settings -> Privacy -> Export Data)
    ClaudeWeb {
        /// Path to Claude.ai export ZIP
        #[arg(long)]
        path: PathBuf,

        /// Shared import flags (dry-run/checkpoint/channel/user).
        #[command(flatten)]
        common: ImportCommonArgs,
    },
}

/// CLI flags common to all import sources.
#[derive(Args, Debug, Clone)]
pub struct ImportCommonArgs {
    /// Preview parsed conversations/messages without writing to DB
    #[arg(long)]
    dry_run: bool,

    /// Target channel for imported conversations
    #[arg(long, default_value = "gateway")]
    channel: String,

    /// Target user id (default: GATEWAY_USER_ID or "default")
    #[arg(long)]
    user: Option<String>,

    /// Legacy flag (no-op): checkpointing is now enabled by default
    #[arg(long, hide = true)]
    resume: bool,

    /// Disable checkpointing for this run
    #[arg(long)]
    no_checkpoint: bool,

    /// Start fresh by deleting any existing checkpoint state before importing
    #[arg(long)]
    fresh: bool,

    /// Delete checkpoint files after a successful import
    #[arg(long)]
    cleanup_checkpoint: bool,

    /// Optional checkpoint state path (default: ~/.ironclaw/import-checkpoints/<derived>.json)
    #[arg(long)]
    checkpoint: Option<PathBuf>,
}

#[derive(Debug, Default)]
struct ImportStats {
    parsed_conversations: usize,
    parsed_messages: usize,
    imported_conversations: usize,
    imported_messages: usize,
    skipped_duplicates: usize,
    skipped_checkpoint: usize,
    skipped_empty: usize,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ImportParseSummary {
    pub parsed_conversations: usize,
    pub parsed_messages: usize,
}

#[derive(Debug, Default, Clone, Copy)]
struct ConversationImportOutcome {
    imported_messages: usize,
    skipped_duplicate: bool,
    skipped_empty: bool,
}

const IMPORT_CHECKPOINT_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct ImportCheckpointState {
    version: u32,
    source_key: String,
    source_name: String,
    source_path: String,
    channel: String,
    user_id: String,
    processed_conversations: usize,
    processed_messages: usize,
    imported_conversations: usize,
    imported_messages: usize,
    skipped_duplicates: usize,
    #[serde(default)]
    skipped_empty_conversations: usize,
    last_source_id: Option<String>,
    started_at: String,
    updated_at: String,
}

struct ImportCheckpoint {
    state_path: PathBuf,
    ids_path: PathBuf,
    state: ImportCheckpointState,
    processed_source_ids: HashSet<String>,
    ids_writer: File,
}

impl ImportCheckpoint {
    fn open_or_create(
        state_path: PathBuf,
        source_key: &str,
        source_name: &str,
        source_path: &Path,
        channel: &str,
        user_id: &str,
    ) -> anyhow::Result<Self> {
        if let Some(parent) = state_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let ids_path = checkpoint_ids_path(&state_path);
        let source_path_str = source_path.to_string_lossy().to_string();
        let now = Utc::now().to_rfc3339();

        let mut state = if state_path.exists() {
            let bytes = fs::read(&state_path)?;
            let existing: ImportCheckpointState = serde_json::from_slice(&bytes)?;
            ensure_checkpoint_compatible(
                &existing,
                source_key,
                source_name,
                source_path_str.as_str(),
                channel,
                user_id,
                &state_path,
            )?;
            existing
        } else {
            ImportCheckpointState {
                version: IMPORT_CHECKPOINT_VERSION,
                source_key: source_key.to_string(),
                source_name: source_name.to_string(),
                source_path: source_path_str.clone(),
                channel: channel.to_string(),
                user_id: user_id.to_string(),
                processed_conversations: 0,
                processed_messages: 0,
                imported_conversations: 0,
                imported_messages: 0,
                skipped_duplicates: 0,
                skipped_empty_conversations: 0,
                last_source_id: None,
                started_at: now.clone(),
                updated_at: now.clone(),
            }
        };

        let processed_source_ids = load_processed_source_ids(&ids_path)?;
        if state.processed_conversations < processed_source_ids.len() {
            state.processed_conversations = processed_source_ids.len();
            state.updated_at = Utc::now().to_rfc3339();
        }

        let ids_writer = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&ids_path)?;

        let mut checkpoint = Self {
            state_path,
            ids_path,
            state,
            processed_source_ids,
            ids_writer,
        };
        checkpoint.save_state()?;

        Ok(checkpoint)
    }

    fn contains(&self, source_id: &str) -> bool {
        self.processed_source_ids.contains(source_id)
    }

    fn mark_processed(
        &mut self,
        source_id: &str,
        parsed_messages: usize,
        imported_messages: usize,
        skipped_duplicate: bool,
        skipped_empty: bool,
    ) -> anyhow::Result<()> {
        if !self.processed_source_ids.insert(source_id.to_string()) {
            return Ok(());
        }

        let encoded = serde_json::to_string(source_id)?;
        writeln!(self.ids_writer, "{encoded}")?;
        self.ids_writer.flush()?;

        self.state.processed_conversations += 1;
        self.state.processed_messages += parsed_messages;
        if skipped_empty {
            self.state.skipped_empty_conversations += 1;
        } else if skipped_duplicate {
            self.state.skipped_duplicates += 1;
        } else {
            self.state.imported_conversations += 1;
            self.state.imported_messages += imported_messages;
        }
        self.state.last_source_id = Some(source_id.to_string());
        self.state.updated_at = Utc::now().to_rfc3339();

        self.save_state()
    }

    fn processed_count(&self) -> usize {
        self.processed_source_ids.len()
    }

    fn state_path(&self) -> &Path {
        &self.state_path
    }

    fn ids_path(&self) -> &Path {
        &self.ids_path
    }

    fn save_state(&mut self) -> anyhow::Result<()> {
        let tmp_path = self.state_path.with_extension("tmp");
        let bytes = serde_json::to_vec_pretty(&self.state)?;
        fs::write(&tmp_path, bytes)?;
        fs::rename(&tmp_path, &self.state_path)?;
        Ok(())
    }
}

fn ensure_checkpoint_compatible(
    state: &ImportCheckpointState,
    source_key: &str,
    source_name: &str,
    source_path: &str,
    channel: &str,
    user_id: &str,
    state_path: &Path,
) -> anyhow::Result<()> {
    if state.version != IMPORT_CHECKPOINT_VERSION {
        anyhow::bail!(
            "checkpoint version mismatch at {} (found v{}, expected v{}). Delete checkpoint and retry.",
            state_path.display(),
            state.version,
            IMPORT_CHECKPOINT_VERSION
        );
    }

    if state.source_key != source_key
        || state.source_name != source_name
        || state.source_path != source_path
        || state.channel != channel
        || state.user_id != user_id
    {
        anyhow::bail!(
            "checkpoint {} does not match this import context (source/path/channel/user). Use a different checkpoint path or remove it.",
            state_path.display()
        );
    }

    Ok(())
}

fn load_processed_source_ids(ids_path: &Path) -> anyhow::Result<HashSet<String>> {
    if !ids_path.exists() {
        return Ok(HashSet::new());
    }

    let file = File::open(ids_path)?;
    let reader = BufReader::new(file);
    let mut out = HashSet::new();
    for line in reader.lines() {
        let line = line?;
        let source_id = line.trim();
        if source_id.is_empty() {
            continue;
        }

        // New format: one JSON string per line.
        // Legacy compatibility: plain line value.
        let parsed = serde_json::from_str::<String>(source_id)
            .ok()
            .unwrap_or_else(|| source_id.to_string());
        out.insert(parsed);
    }

    Ok(out)
}

fn checkpoint_ids_path(state_path: &Path) -> PathBuf {
    let mut ids_name = state_path
        .file_name()
        .map(|v| v.to_string_lossy().to_string())
        .unwrap_or_else(|| "import-checkpoint.json".to_string());
    ids_name.push_str(".ids");
    state_path.with_file_name(ids_name)
}

fn remove_checkpoint_files(state_path: &Path) -> anyhow::Result<usize> {
    let mut removed = 0_usize;
    let ids_path = checkpoint_ids_path(state_path);

    match fs::remove_file(state_path) {
        Ok(()) => {
            removed += 1;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }

    match fs::remove_file(&ids_path) {
        Ok(()) => {
            removed += 1;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }

    Ok(removed)
}

fn default_checkpoint_path(
    source_key: &str,
    source_path: &Path,
    channel: &str,
    user_id: &str,
) -> PathBuf {
    let base_dir = dirs::home_dir()
        .map(|home| home.join(".ironclaw").join("import-checkpoints"))
        .unwrap_or_else(|| PathBuf::from(".ironclaw/import-checkpoints"));

    let mut hasher = DefaultHasher::new();
    source_key.hash(&mut hasher);
    source_path.to_string_lossy().hash(&mut hasher);
    channel.hash(&mut hasher);
    user_id.hash(&mut hasher);
    let fingerprint = hasher.finish();

    base_dir.join(format!("{source_key}-{fingerprint:016x}.json"))
}

/// Run an import command.
pub async fn run_import_command(cmd: ImportCommand, no_db: bool) -> anyhow::Result<()> {
    match cmd {
        ImportCommand::ClaudeCode { path, common } => {
            let source_path = path.unwrap_or_else(default_claude_code_path);
            run_import_flow(ClaudeCodeImporter, source_path, common, no_db).await
        }
        ImportCommand::ClaudeWeb { path, common } => {
            run_import_flow(ClaudeWebImporter, path, common, no_db).await
        }
    }
}

async fn run_import_flow(
    importer: impl Importer,
    path: PathBuf,
    common: ImportCommonArgs,
    no_db: bool,
) -> anyhow::Result<()> {
    let ImportCommonArgs {
        dry_run,
        channel,
        user,
        resume: resume_compat,
        no_checkpoint,
        fresh,
        cleanup_checkpoint,
        checkpoint,
    } = common;

    let source_key = importer.source_key().to_string();
    let source_name = importer.source_name().to_string();

    if dry_run {
        if no_checkpoint || fresh || checkpoint.is_some() || cleanup_checkpoint || resume_compat {
            println!("Note: checkpoint options are ignored in --dry-run mode.");
        }
        let summary = importer
            .parse_stream(&path, |_conversation| Ok(()))
            .map_err(|e| anyhow::anyhow!("{} parse failed: {}", source_name, e))?;

        println!(
            "Parsed {} conversation(s) and {} message(s) from {} ({})",
            summary.parsed_conversations,
            summary.parsed_messages,
            source_name,
            path.display()
        );
        println!("Dry run enabled: no database writes performed.");
        return Ok(());
    }

    if no_db {
        anyhow::bail!("Import requires database access. Re-run without --no-db or use --dry-run.");
    }

    if no_checkpoint && checkpoint.is_some() {
        anyhow::bail!("--no-checkpoint cannot be used with --checkpoint.");
    }
    if no_checkpoint && fresh {
        anyhow::bail!("--fresh requires checkpointing; remove --no-checkpoint.");
    }
    if no_checkpoint && cleanup_checkpoint {
        anyhow::bail!("--cleanup-checkpoint requires checkpointing; remove --no-checkpoint.");
    }

    let config = Config::from_env()
        .await
        .map_err(|e| anyhow::anyhow!("config error: {}", e))?;

    let target_user = user.unwrap_or_else(|| {
        config
            .channels
            .gateway
            .as_ref()
            .map(|g| g.user_id.clone())
            .unwrap_or_else(|| "default".to_string())
    });

    let db = crate::db::connect_from_config(&config.database)
        .await
        .map_err(|e| anyhow::anyhow!("database connection failed: {}", e))?;

    if resume_compat {
        println!("Note: --resume is now a no-op; checkpointing is enabled by default.");
    }

    let checkpoint_enabled = !no_checkpoint;
    let checkpoint_path = if checkpoint_enabled {
        Some(checkpoint.unwrap_or_else(|| {
            default_checkpoint_path(
                source_key.as_str(),
                &path,
                channel.as_str(),
                target_user.as_str(),
            )
        }))
    } else {
        None
    };

    let mut checkpoint = match checkpoint_path {
        Some(cp_path) => {
            if fresh {
                let removed = remove_checkpoint_files(cp_path.as_path())?;
                if removed > 0 {
                    println!(
                        "Removed {} existing checkpoint file(s) at {} before starting fresh.",
                        removed,
                        cp_path.display()
                    );
                }
            }
            let cp = ImportCheckpoint::open_or_create(
                cp_path,
                source_key.as_str(),
                source_name.as_str(),
                &path,
                channel.as_str(),
                target_user.as_str(),
            )?;
            println!(
                "Resume checkpoint active: {} ({} processed source id(s), ids log: {})",
                cp.state_path().display(),
                cp.processed_count(),
                cp.ids_path().display()
            );
            Some(cp)
        }
        None => {
            println!("Checkpointing disabled via --no-checkpoint.");
            None
        }
    };

    println!("Importing from {}:", source_name);
    println!("  Path: {}", path.display());
    println!("  Target: channel={} user={}", channel, target_user);

    let mut stats = ImportStats::default();
    let runtime_handle = tokio::runtime::Handle::current();
    let source_key_owned = source_key;
    let channel_owned = channel;
    let user_owned = target_user.clone();
    let mut spinner_index = 0_usize;
    let spinner_frames = ["-", "\\", "|", "/"];
    let mut last_rendered_width = 0_usize;
    let mut last_tty_render = Instant::now()
        .checked_sub(Duration::from_secs(1))
        .unwrap_or_else(Instant::now);
    let mut last_non_tty_log = Instant::now();
    let progress_is_tty = std::io::stdout().is_terminal();

    let mut render_progress = |stats: &ImportStats, final_line: bool| {
        if stats.parsed_conversations == 0 {
            return;
        }

        let processed = stats.parsed_conversations;
        let line = format!(
            "Processed {} conversation(s) | Imported {} conversation(s), {} message(s) | Skipped dup {}, empty {}, checkpoint {}",
            processed,
            stats.imported_conversations,
            stats.imported_messages,
            stats.skipped_duplicates,
            stats.skipped_empty,
            stats.skipped_checkpoint
        );

        if progress_is_tty {
            if !final_line && last_tty_render.elapsed() < Duration::from_millis(120) {
                return;
            }
            let frame = if final_line {
                "*"
            } else {
                let idx = spinner_index % spinner_frames.len();
                spinner_index += 1;
                spinner_frames[idx]
            };
            let rendered = format!("{frame} {line}");
            let rendered_width = rendered.chars().count();
            let padding = if last_rendered_width > rendered_width {
                " ".repeat(last_rendered_width - rendered_width)
            } else {
                String::new()
            };

            if final_line {
                print!("\r{rendered}{padding}\n");
            } else {
                print!("\r{rendered}{padding}");
            }
            let _ = std::io::stdout().flush();
            last_rendered_width = rendered_width;
            last_tty_render = Instant::now();
            return;
        }

        let should_log = final_line
            || processed == 1
            || processed % 25 == 0
            || last_non_tty_log.elapsed() >= Duration::from_secs(2);
        if should_log {
            println!("{line}");
            last_non_tty_log = Instant::now();
        }
    };

    let parse_result = importer.parse_stream(&path, |conversation| {
        stats.parsed_conversations += 1;
        stats.parsed_messages += conversation.messages.len();
        let source_id = conversation.source_id.clone();
        let parsed_message_count = conversation.messages.len();

        if let Some(checkpoint) = checkpoint.as_ref()
            && checkpoint.contains(source_id.as_str())
        {
            stats.skipped_checkpoint += 1;
            render_progress(&stats, false);
            return Ok(());
        }

        let outcome = tokio::task::block_in_place(|| {
            runtime_handle.block_on(persist_imported_conversation(
                db.clone(),
                source_key_owned.as_str(),
                source_name.as_str(),
                channel_owned.as_str(),
                user_owned.as_str(),
                conversation,
            ))
        })
        .map_err(|e| {
            ImportError::Parse(format!(
                "database import failed for source_id {}: {}",
                source_id, e
            ))
        })?;

        if outcome.skipped_duplicate {
            stats.skipped_duplicates += 1;
        } else if outcome.skipped_empty {
            stats.skipped_empty += 1;
        } else {
            stats.imported_conversations += 1;
            stats.imported_messages += outcome.imported_messages;
        }

        if let Some(checkpoint) = checkpoint.as_mut() {
            checkpoint
                .mark_processed(
                    source_id.as_str(),
                    parsed_message_count,
                    outcome.imported_messages,
                    outcome.skipped_duplicate,
                    outcome.skipped_empty,
                )
                .map_err(|e| {
                    ImportError::Parse(format!(
                        "checkpoint update failed for source_id {}: {}",
                        source_id, e
                    ))
                })?;
        }

        render_progress(&stats, false);
        Ok(())
    });

    if let Err(e) = parse_result {
        if progress_is_tty && stats.parsed_conversations > 0 {
            println!();
        }
        return Err(anyhow::anyhow!(
            "{} parse failed: {}",
            importer.source_name(),
            e
        ));
    }

    render_progress(&stats, true);

    println!();
    println!("Import Summary");
    println!("  Source: {} ({})", source_name, path.display());
    println!(
        "  Parsed: {} conversation(s), {} message(s)",
        stats.parsed_conversations, stats.parsed_messages
    );
    println!(
        "  Imported: {} conversation(s), {} message(s)",
        stats.imported_conversations, stats.imported_messages
    );
    println!(
        "  Skipped: {} duplicate(s), {} empty conversation(s), {} from checkpoint",
        stats.skipped_duplicates, stats.skipped_empty, stats.skipped_checkpoint
    );

    if let Some(checkpoint) = checkpoint.as_ref() {
        println!(
            "  Checkpoint: updated {} ({} processed source id(s))",
            checkpoint.state_path().display(),
            checkpoint.processed_count()
        );
    }
    if cleanup_checkpoint && let Some(checkpoint) = checkpoint.take() {
        let checkpoint_path = checkpoint.state_path().to_path_buf();
        drop(checkpoint);
        let removed = remove_checkpoint_files(checkpoint_path.as_path())?;
        println!(
            "  Checkpoint cleanup: removed {} checkpoint file(s)",
            removed
        );
    }

    Ok(())
}

async fn persist_imported_conversation(
    db: Arc<dyn Database>,
    source_key: &str,
    source_name: &str,
    channel: &str,
    user_id: &str,
    conversation: ImportedConversation,
) -> anyhow::Result<ConversationImportOutcome> {
    let existing = db
        .find_conversation_by_import_source(
            user_id,
            channel,
            source_key,
            conversation.source_id.as_str(),
        )
        .await?;

    if let Some(existing_id) = existing {
        let existing_metadata = db.get_conversation_metadata(existing_id).await?;
        let existing_import_state = existing_metadata
            .as_ref()
            .and_then(import_state_from_metadata);

        if matches!(existing_import_state, Some("in_progress")) {
            tracing::warn!(
                "Found stale partial import for source_id {} (conversation id {}). Re-importing.",
                conversation.source_id,
                existing_id
            );
            db.delete_conversation(existing_id).await?;
        } else {
            return Ok(ConversationImportOutcome {
                imported_messages: 0,
                skipped_duplicate: true,
                skipped_empty: false,
            });
        }
    }

    let has_non_empty_message = conversation
        .messages
        .iter()
        .any(|m| !m.content.trim().is_empty());
    if !has_non_empty_message {
        tracing::warn!(
            "Skipping import of conversation {} because it has no non-empty user/assistant messages",
            conversation.source_id
        );
        return Ok(ConversationImportOutcome {
            imported_messages: 0,
            skipped_duplicate: false,
            skipped_empty: true,
        });
    }

    let mut import_metadata = serde_json::json!({
        "source": source_key,
        "source_name": source_name,
        "source_id": conversation.source_id.as_str(),
        "state": "in_progress",
        "started_at": conversation
            .started_at
            .as_ref()
            .map(|v| v.to_rfc3339()),
        "updated_at": conversation
            .updated_at
            .as_ref()
            .map(|v| v.to_rfc3339()),
        "source_metadata": conversation.source_metadata.clone(),
        "message_source_metadata_count": conversation.messages.len(),
        "message_source_metadata_sample": conversation
            .messages
            .iter()
            .take(32)
            .enumerate()
            .map(|(idx, msg)| serde_json::json!({
                "index": idx,
                "role": msg.role.as_str(),
                "metadata": msg.source_metadata.clone(),
            }))
            .collect::<Vec<_>>(),
        "import_started_at": Utc::now().to_rfc3339(),
    });

    let metadata = serde_json::json!({
        "thread_type": "thread",
        "title": conversation.title.as_deref(),
        "import": import_metadata.clone(),
    });

    let conversation_id = db
        .create_conversation_with_metadata(channel, user_id, &metadata)
        .await?;

    let mut imported_messages = 0_usize;
    let mut first_msg_ts: Option<DateTime<Utc>> = None;
    let mut last_msg_ts: Option<DateTime<Utc>> = None;

    for message in &conversation.messages {
        if message.content.trim().is_empty() {
            continue;
        }

        if let Some(ts) = message.timestamp.as_ref().cloned() {
            db.add_conversation_message_at(
                conversation_id,
                message.role.as_str(),
                message.content.as_str(),
                ts,
            )
            .await?;
            first_msg_ts = Some(first_msg_ts.map_or(ts, |old| old.min(ts)));
            last_msg_ts = Some(last_msg_ts.map_or(ts, |old| old.max(ts)));
        } else {
            db.add_conversation_message(
                conversation_id,
                message.role.as_str(),
                message.content.as_str(),
            )
            .await?;
        }
        imported_messages += 1;
    }

    let started_at = conversation.started_at.or(first_msg_ts);
    let last_activity = conversation.updated_at.or(last_msg_ts);
    db.set_conversation_time_bounds(conversation_id, started_at, last_activity)
        .await?;

    if let Some(import_obj) = import_metadata.as_object_mut() {
        import_obj.insert(
            "state".to_string(),
            serde_json::Value::String("complete".to_string()),
        );
        import_obj.insert(
            "imported_at".to_string(),
            serde_json::Value::String(Utc::now().to_rfc3339()),
        );
        import_obj.insert(
            "imported_message_count".to_string(),
            serde_json::Value::from(imported_messages as u64),
        );
    }
    db.update_conversation_metadata_field(conversation_id, "import", &import_metadata)
        .await?;

    Ok(ConversationImportOutcome {
        imported_messages,
        skipped_duplicate: false,
        skipped_empty: false,
    })
}

fn import_state_from_metadata(metadata: &serde_json::Value) -> Option<&str> {
    metadata.get("import")?.get("state")?.as_str()
}

pub(crate) fn parse_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        return Some(dt.with_timezone(&Utc));
    }

    let zoned_formats = [
        "%Y-%m-%d %H:%M:%S%.f%z",
        "%Y-%m-%dT%H:%M:%S%.f%z",
        "%Y-%m-%d %H:%M:%S%z",
        "%Y-%m-%dT%H:%M:%S%z",
    ];
    for format in zoned_formats {
        if let Ok(dt) = DateTime::parse_from_str(raw, format) {
            return Some(dt.with_timezone(&Utc));
        }
    }

    let naive_formats = [
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S",
    ];
    for format in naive_formats {
        if let Ok(dt) = NaiveDateTime::parse_from_str(raw, format) {
            return Some(DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc));
        }
    }

    None
}

pub(crate) fn truncate_chars(input: &str, max_chars: usize) -> String {
    input.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{
        ImportCheckpoint, default_checkpoint_path, import_state_from_metadata,
        load_processed_source_ids, remove_checkpoint_files,
    };

    #[test]
    fn default_checkpoint_path_is_stable_for_same_inputs() {
        let a = default_checkpoint_path(
            "claude_web",
            std::path::Path::new("/tmp/a.zip"),
            "gateway",
            "user-1",
        );
        let b = default_checkpoint_path(
            "claude_web",
            std::path::Path::new("/tmp/a.zip"),
            "gateway",
            "user-1",
        );
        assert_eq!(a, b);
    }

    #[test]
    fn checkpoint_roundtrip_persists_processed_ids() {
        let temp = tempdir().expect("tempdir");
        let state_path = temp.path().join("resume.json");
        let source_path = temp.path().join("claude.zip");
        std::fs::write(&source_path, "placeholder").expect("write source file");

        let mut checkpoint = ImportCheckpoint::open_or_create(
            state_path.clone(),
            "claude_web",
            "Claude.ai",
            &source_path,
            "gateway",
            "user-1",
        )
        .expect("create checkpoint");
        checkpoint
            .mark_processed("src-1", 2, 2, false, false)
            .expect("mark src-1");
        checkpoint
            .mark_processed("src-2", 3, 0, true, false)
            .expect("mark src-2");

        let reopened = ImportCheckpoint::open_or_create(
            state_path,
            "claude_web",
            "Claude.ai",
            &source_path,
            "gateway",
            "user-1",
        )
        .expect("reopen checkpoint");

        assert!(reopened.contains("src-1"));
        assert!(reopened.contains("src-2"));
        assert_eq!(reopened.processed_count(), 2);
    }

    #[test]
    fn checkpoint_rejects_context_mismatch() {
        let temp = tempdir().expect("tempdir");
        let state_path = temp.path().join("resume.json");
        let source_path = temp.path().join("claude.zip");
        std::fs::write(&source_path, "placeholder").expect("write source file");

        let _checkpoint = ImportCheckpoint::open_or_create(
            state_path.clone(),
            "claude_web",
            "Claude.ai",
            &source_path,
            "gateway",
            "user-1",
        )
        .expect("create checkpoint");

        let result = ImportCheckpoint::open_or_create(
            state_path,
            "claude_web",
            "Claude.ai",
            &source_path,
            "gateway",
            "user-2",
        );
        assert!(result.is_err());
        let msg = format!("{}", result.err().expect("error should exist"));
        assert!(msg.contains("does not match this import context"));
    }

    #[test]
    fn checkpoint_supports_control_chars_in_source_id() {
        let temp = tempdir().expect("tempdir");
        let state_path = temp.path().join("resume.json");
        let source_path = temp.path().join("claude.zip");
        std::fs::write(&source_path, "placeholder").expect("write source file");

        let tricky_id = "source\nid\twith\"quotes\\slashes";

        let mut checkpoint = ImportCheckpoint::open_or_create(
            state_path.clone(),
            "claude_web",
            "Claude.ai",
            &source_path,
            "gateway",
            "user-1",
        )
        .expect("create checkpoint");
        checkpoint
            .mark_processed(tricky_id, 1, 1, false, false)
            .expect("mark tricky id");
        drop(checkpoint);

        let reopened = ImportCheckpoint::open_or_create(
            state_path,
            "claude_web",
            "Claude.ai",
            &source_path,
            "gateway",
            "user-1",
        )
        .expect("reopen checkpoint");

        assert!(reopened.contains(tricky_id));
        assert_eq!(reopened.processed_count(), 1);
    }

    #[test]
    fn remove_checkpoint_files_deletes_state_and_ids() {
        let temp = tempdir().expect("tempdir");
        let state_path = temp.path().join("resume.json");
        let source_path = temp.path().join("claude.zip");
        std::fs::write(&source_path, "placeholder").expect("write source file");

        let mut checkpoint = ImportCheckpoint::open_or_create(
            state_path.clone(),
            "claude_web",
            "Claude.ai",
            &source_path,
            "gateway",
            "user-1",
        )
        .expect("create checkpoint");
        checkpoint
            .mark_processed("src-1", 1, 1, false, false)
            .expect("mark src-1");
        drop(checkpoint);

        let removed = remove_checkpoint_files(&state_path).expect("cleanup checkpoint");
        assert_eq!(removed, 2);
        assert!(!state_path.exists());
        assert!(!state_path.with_file_name("resume.json.ids").exists());
    }

    #[test]
    fn load_processed_source_ids_supports_json_and_legacy_lines() {
        let temp = tempdir().expect("tempdir");
        let ids_path = temp.path().join("resume.json.ids");

        std::fs::write(
            &ids_path,
            "legacy-id\n\"json-id-with\\n-escape\"\n\n\"quoted\\\"id\"\n",
        )
        .expect("write ids file");

        let ids = load_processed_source_ids(&ids_path).expect("load ids");
        assert!(ids.contains("legacy-id"));
        assert!(ids.contains("json-id-with\n-escape"));
        assert!(ids.contains("quoted\"id"));
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn remove_checkpoint_files_is_idempotent_for_missing_paths() {
        let temp = tempdir().expect("tempdir");
        let missing = temp.path().join("does-not-exist.json");
        let removed = remove_checkpoint_files(&missing).expect("remove missing files");
        assert_eq!(removed, 0);
    }

    #[test]
    fn checkpoint_tracks_skipped_empty_conversations() {
        let temp = tempdir().expect("tempdir");
        let state_path = temp.path().join("resume.json");
        let source_path = temp.path().join("claude.zip");
        std::fs::write(&source_path, "placeholder").expect("write source file");

        let mut checkpoint = ImportCheckpoint::open_or_create(
            state_path.clone(),
            "claude_web",
            "Claude.ai",
            &source_path,
            "gateway",
            "user-1",
        )
        .expect("create checkpoint");
        checkpoint
            .mark_processed("empty-src", 0, 0, false, true)
            .expect("mark empty conversation");
        drop(checkpoint);

        let raw = std::fs::read_to_string(state_path).expect("read state");
        let state: serde_json::Value = serde_json::from_str(&raw).expect("parse state");
        assert_eq!(
            state
                .get("skipped_empty_conversations")
                .and_then(|v| v.as_u64()),
            Some(1)
        );
    }

    #[test]
    fn import_state_from_metadata_reads_nested_state() {
        let metadata = serde_json::json!({
            "import": {
                "state": "in_progress"
            }
        });
        assert_eq!(import_state_from_metadata(&metadata), Some("in_progress"));
    }

    #[test]
    fn import_state_from_metadata_returns_none_when_missing() {
        let metadata = serde_json::json!({
            "thread_type": "thread"
        });
        assert_eq!(import_state_from_metadata(&metadata), None);
    }
}
