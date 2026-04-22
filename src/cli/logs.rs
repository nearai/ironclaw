//! CLI command for viewing and managing gateway logs.
//!
//! Provides access to gateway logs through three mechanisms:
//! - Reading the gateway log file (`~/.ironclaw/gateway.log`)
//! - Streaming live logs via the gateway's SSE endpoint (`/api/logs/events`)
//! - Getting/setting the runtime log level via `/api/logs/level`

use std::io::{Seek, SeekFrom};
use std::path::Path;

use clap::Args;

/// View and manage gateway logs.
#[derive(Args, Debug, Clone)]
#[command(
    about = "View and manage gateway logs",
    long_about = "Tail gateway logs, stream live output, or adjust log level.\nExamples:\n  ironclaw logs                          # Show last 200 lines\n  ironclaw logs --follow                 # Stream live logs via SSE\n  ironclaw logs --limit 50 --json        # Last 50 lines as JSON\n  ironclaw logs --level                  # Show current log level\n  ironclaw logs --level debug            # Set log level to debug\n  ironclaw logs --grep ERROR             # Filter lines matching 'ERROR'\n  ironclaw logs -g 'timeout|refused' --context 3  # Regex with context"
)]
pub struct LogsCommand {
    /// Stream live logs from the running gateway via SSE.
    /// Replays recent history then streams new entries in real time.
    #[arg(short, long)]
    pub follow: bool,

    /// Maximum number of lines to show (default: 200)
    #[arg(short, long, default_value = "200")]
    pub limit: usize,

    /// Output log entries as JSON (one object per line)
    #[arg(long)]
    pub json: bool,

    /// Display timestamps in local timezone
    #[arg(long)]
    pub local_time: bool,

    /// Plain text output (no ANSI styling)
    #[arg(long)]
    pub plain: bool,

    /// Gateway URL (default: http://{GATEWAY_HOST}:{GATEWAY_PORT})
    #[arg(long)]
    pub url: Option<String>,

    /// Gateway auth token (reads GATEWAY_AUTH_TOKEN env if not set)
    #[arg(long)]
    pub token: Option<String>,

    /// Connection timeout in milliseconds (default: 5000)
    #[arg(long, default_value = "5000")]
    pub timeout: u64,

    /// Get or set runtime log level. Without a value, shows current level.
    /// With a value (trace|debug|info|warn|error), sets the level.
    #[arg(long, num_args = 0..=1, default_missing_value = "")]
    pub level: Option<String>,

    /// Filter output by regex pattern (conflicts with --follow)
    #[arg(long, short = 'g', conflicts_with = "follow")]
    pub grep: Option<String>,

    /// Ignore case when filtering (requires --grep)
    #[arg(long, requires = "grep")]
    pub ignore_case: bool,

    /// Show N lines of context around matches (requires --grep)
    #[arg(long, requires = "grep", default_value = "0")]
    pub context: usize,
}

/// Resolved gateway connection parameters.
struct GatewayParams {
    base_url: String,
    token: String,
}

/// Run the logs CLI command.
pub async fn run_logs_command(cmd: LogsCommand, config_path: Option<&Path>) -> anyhow::Result<()> {
    // --level takes priority: it's a control-plane operation, not log viewing.
    if let Some(level_arg) = &cmd.level {
        let params = resolve_gateway_params(&cmd, config_path).await?;
        if level_arg.is_empty() {
            return cmd_get_level(&cmd, &params).await;
        } else {
            return cmd_set_level(&cmd, level_arg, &params).await;
        }
    }

    if cmd.follow {
        let params = resolve_gateway_params(&cmd, config_path).await?;
        cmd_follow(&cmd, &params).await
    } else {
        cmd_show(&cmd)
    }
}

// ── Show log file ────────────────────────────────────────────────────────

/// Read the last N lines from `~/.ironclaw/gateway.log`.
///
/// Uses a reverse-scan strategy: seeks to the end of the file and reads
/// backwards in chunks to find the last `limit` newlines, so memory usage
/// is proportional to the output size, not the file size.
fn cmd_show(cmd: &LogsCommand) -> anyhow::Result<()> {
    let log_path = crate::bootstrap::ironclaw_base_dir().join("gateway.log");
    if !log_path.exists() {
        anyhow::bail!(
            "No gateway log file found at {}.\n\
             The log file is created when the gateway runs in background mode \
             (e.g. `ironclaw gateway start`).",
            log_path.display()
        );
    }

    let mut lines = tail_file(&log_path, cmd.limit)?;

    // Apply grep filtering if requested
    if let Some(ref pattern) = cmd.grep {
        lines = filter_lines(lines, pattern, cmd.ignore_case, cmd.context)?;
    }

    if lines.is_empty() {
        if cmd.grep.is_some() {
            println!("(no matching log entries)");
        } else {
            println!("(log file is empty)");
        }
        return Ok(());
    }

    if cmd.json {
        for line in &lines {
            let obj = serde_json::json!({ "line": line });
            println!("{}", obj);
        }
    } else {
        for line in &lines {
            println!("{}", line);
        }
    }

    Ok(())
}

/// Filter lines by regex pattern, optionally with context.
fn filter_lines(
    lines: Vec<String>,
    pattern: &str,
    ignore_case: bool,
    context: usize,
) -> anyhow::Result<Vec<String>> {
    use regex::RegexBuilder;

    let re = RegexBuilder::new(pattern)
        .case_insensitive(ignore_case)
        .build()
        .map_err(|e| anyhow::anyhow!("Invalid regex pattern '{}': {}", pattern, e))?;

    if context == 0 {
        // Simple filtering: only matching lines
        Ok(lines.into_iter().filter(|line| re.is_match(line)).collect())
    } else {
        // With context: add N lines before/after each match
        let mut result_indices = std::collections::BTreeSet::new();

        // Find matching indices and add context
        for (i, line) in lines.iter().enumerate() {
            if re.is_match(line) {
                // Add context range
                let start = i.saturating_sub(context);
                let end = i.saturating_add(context).saturating_add(1).min(lines.len());
                for j in start..end {
                    result_indices.insert(j);
                }
            }
        }

        // Collect lines in order
        Ok(result_indices
            .into_iter()
            .map(|i| lines[i].clone())
            .collect())
    }
}

/// Read the last `n` lines from a file by scanning backwards from EOF.
///
/// Reads in 8 KiB chunks from the end, counting newlines until enough
/// are found or the beginning of the file is reached.
fn tail_file(path: &Path, n: usize) -> anyhow::Result<Vec<String>> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| anyhow::anyhow!("Failed to open {}: {}", path.display(), e))?;

    let file_len = file
        .seek(SeekFrom::End(0))
        .map_err(|e| anyhow::anyhow!("Failed to seek {}: {}", path.display(), e))?;

    if file_len == 0 {
        return Ok(Vec::new());
    }

    // Read backwards in chunks to find enough newlines.
    const CHUNK_SIZE: u64 = 8192;
    let mut tail_bytes = Vec::new();
    let mut newline_count = 0;
    let mut remaining = file_len;

    while remaining > 0 && newline_count <= n {
        let read_size = std::cmp::min(CHUNK_SIZE, remaining);
        remaining -= read_size;

        file.seek(SeekFrom::Start(remaining))
            .map_err(|e| anyhow::anyhow!("Seek failed: {e}"))?;

        let mut chunk = vec![0u8; read_size as usize];
        std::io::Read::read_exact(&mut file, &mut chunk)
            .map_err(|e| anyhow::anyhow!("Read failed: {e}"))?;

        // Count newlines in this chunk (backwards).
        for &byte in chunk.iter().rev() {
            if byte == b'\n' {
                newline_count += 1;
            }
        }

        // Prepend chunk to collected bytes.
        chunk.append(&mut tail_bytes);
        tail_bytes = chunk;
    }

    // Convert to string and take last N lines.
    let text = String::from_utf8_lossy(&tail_bytes);
    let all_lines: Vec<&str> = text.lines().collect();
    let start = all_lines.len().saturating_sub(n);

    Ok(all_lines[start..].iter().map(|s| s.to_string()).collect())
}

// ── Follow (live SSE stream) ─────────────────────────────────────────────

/// Connect to the gateway's `/api/logs/events` SSE endpoint and stream logs.
async fn cmd_follow(cmd: &LogsCommand, params: &GatewayParams) -> anyhow::Result<()> {
    let timeout_dur = std::time::Duration::from_millis(cmd.timeout);

    let client = reqwest::Client::builder()
        .connect_timeout(timeout_dur)
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to create HTTP client: {e}"))?;

    let url = format!("{}/api/logs/events", params.base_url);
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", params.token))
        .header("Accept", "text/event-stream")
        // No per-request timeout: SSE streams are long-lived.
        .timeout(std::time::Duration::from_secs(u64::MAX / 2))
        .send()
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "Failed to connect to gateway at {url}: {e}\n\
                 Is the gateway running? Try `ironclaw gateway status`."
            )
        })?;

    if !resp.status().is_success() {
        anyhow::bail!(
            "Gateway returned HTTP {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        );
    }

    eprintln!("Connected to {} — streaming logs (Ctrl-C to stop)", url);

    // Parse SSE stream line by line.
    let mut bytes_stream = resp.bytes_stream();
    let mut buffer = String::new();
    let mut lines_shown: usize = 0;

    use futures::StreamExt;
    while let Some(chunk) = bytes_stream.next().await {
        let chunk = chunk.map_err(|e| anyhow::anyhow!("Stream error: {e}"))?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        // Process complete lines from the buffer.
        while let Some(newline_pos) = buffer.find('\n') {
            let line = buffer[..newline_pos].to_string();
            buffer = buffer[newline_pos + 1..].to_string();

            // SSE format: "data: {...}" lines carry the payload.
            if let Some(data) = line.strip_prefix("data: ")
                && let Ok(entry) = serde_json::from_str::<serde_json::Value>(data)
            {
                print_log_entry(&entry, cmd);
                lines_shown += 1;
            }
            // Skip "event:", "id:", "retry:", and empty keepalive lines.
        }
    }

    if lines_shown == 0 {
        eprintln!("(no log entries received)");
    }

    Ok(())
}

// ── Log level get/set ────────────────────────────────────────────────────

/// GET /api/logs/level — show the current log level.
async fn cmd_get_level(cmd: &LogsCommand, params: &GatewayParams) -> anyhow::Result<()> {
    let timeout_dur = std::time::Duration::from_millis(cmd.timeout);

    let client = reqwest::Client::builder()
        .timeout(timeout_dur)
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to create HTTP client: {e}"))?;

    let url = format!("{}/api/logs/level", params.base_url);
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", params.token))
        .send()
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "Failed to connect to gateway at {url}: {e}\n\
                 Is the gateway running? Try `ironclaw gateway status`."
            )
        })?;

    if !resp.status().is_success() {
        anyhow::bail!(
            "Gateway returned HTTP {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        );
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("Invalid response: {e}"))?;

    if cmd.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    } else {
        let level = body
            .get("level")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        println!("Current log level: {}", level);
    }

    Ok(())
}

/// PUT /api/logs/level — change the runtime log level.
async fn cmd_set_level(
    cmd: &LogsCommand,
    level: &str,
    params: &GatewayParams,
) -> anyhow::Result<()> {
    const VALID: &[&str] = &["trace", "debug", "info", "warn", "error"];
    let level_lower = level.to_lowercase();
    if !VALID.contains(&level_lower.as_str()) {
        anyhow::bail!(
            "Invalid log level '{}'. Must be one of: {}",
            level,
            VALID.join(", ")
        );
    }

    let timeout_dur = std::time::Duration::from_millis(cmd.timeout);

    let client = reqwest::Client::builder()
        .timeout(timeout_dur)
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to create HTTP client: {e}"))?;

    let url = format!("{}/api/logs/level", params.base_url);
    let resp = client
        .put(&url)
        .header("Authorization", format!("Bearer {}", params.token))
        .json(&serde_json::json!({ "level": level_lower }))
        .send()
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "Failed to connect to gateway at {url}: {e}\n\
                 Is the gateway running? Try `ironclaw gateway status`."
            )
        })?;

    if !resp.status().is_success() {
        anyhow::bail!(
            "Gateway returned HTTP {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        );
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("Invalid response: {e}"))?;

    if cmd.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    } else {
        let new_level = body
            .get("level")
            .and_then(|v| v.as_str())
            .unwrap_or(&level_lower);
        println!("Log level set to: {}", new_level);
    }

    Ok(())
}

// ── Helpers ──────────────────────────────────────────────────────────────

/// Resolve gateway connection params from CLI flags, config file, or env.
///
/// Priority: --url/--token flags > config TOML > env vars > defaults.
async fn resolve_gateway_params(
    cmd: &LogsCommand,
    config_path: Option<&Path>,
) -> anyhow::Result<GatewayParams> {
    // Load gateway config. Errors propagate when --config is explicit.
    let gw_config = load_gateway_config(config_path).await?;

    // URL: --url flag > config TOML > env vars > defaults.
    let base_url = if let Some(url) = &cmd.url {
        url.trim_end_matches('/').to_string()
    } else if let Some(cfg) = &gw_config {
        format!("http://{}:{}", cfg.host, cfg.port)
    } else {
        let host = std::env::var("GATEWAY_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let port: u16 = std::env::var("GATEWAY_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(3000);
        format!("http://{}:{}", host, port)
    };

    // Token: --token flag > config TOML > env var.
    let token = if let Some(token) = &cmd.token {
        token.clone()
    } else if let Some(t) = gw_config.as_ref().and_then(|c| c.auth_token.clone()) {
        t
    } else {
        std::env::var("GATEWAY_AUTH_TOKEN").map_err(|_| {
            anyhow::anyhow!(
                "No auth token provided. Use --token <TOKEN> or set GATEWAY_AUTH_TOKEN.\n\
                 The token is printed when the gateway starts."
            )
        })?
    };

    Ok(GatewayParams { base_url, token })
}

/// Try to load gateway config from the TOML config file.
///
/// If `config_path` was explicitly provided (via `--config`), errors are
/// propagated — the user asked for a specific file and deserves a clear
/// failure when it is missing, unreadable, or malformed.  When no path
/// was given we fall back to env-only resolution and silently return
/// `None` on failure so that `ironclaw logs` works without any config.
async fn load_gateway_config(
    config_path: Option<&Path>,
) -> anyhow::Result<Option<crate::config::GatewayConfig>> {
    if config_path.is_some() {
        // Explicit --config: propagate errors.
        let config = crate::config::Config::from_env_with_toml(config_path)
            .await
            .map_err(|e| anyhow::anyhow!("{e:#}"))?;
        Ok(config.channels.gateway)
    } else {
        // No explicit config: best-effort, swallow errors.
        let config = crate::config::Config::from_env_with_toml(None).await.ok();
        Ok(config.and_then(|c| c.channels.gateway))
    }
}

/// Print a single log entry to stdout.
fn print_log_entry(entry: &serde_json::Value, cmd: &LogsCommand) {
    if cmd.json {
        println!("{}", serde_json::to_string(entry).unwrap_or_default());
        return;
    }

    let level = entry.get("level").and_then(|v| v.as_str()).unwrap_or("?");
    let target = entry.get("target").and_then(|v| v.as_str()).unwrap_or("");
    let message = entry.get("message").and_then(|v| v.as_str()).unwrap_or("");
    let timestamp = entry
        .get("timestamp")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let display_ts = if cmd.local_time {
        convert_to_local_time(timestamp)
    } else {
        timestamp.to_string()
    };

    if cmd.plain {
        println!("{} {} [{}] {}", display_ts, level, target, message);
    } else {
        let level_colored = colorize_level(level);
        println!("{} {} [{}] {}", display_ts, level_colored, target, message);
    }
}

/// Convert an RFC 3339 timestamp to local time display.
fn convert_to_local_time(ts: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(ts)
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%Y-%m-%dT%H:%M:%S%.3f")
                .to_string()
        })
        .unwrap_or_else(|_| ts.to_string())
}

/// Apply ANSI color to log level for terminal display.
fn colorize_level(level: &str) -> String {
    match level {
        "ERROR" => format!("\x1b[31m{}\x1b[0m", level), // red
        "WARN" => format!("\x1b[33m{}\x1b[0m", level),  // yellow
        "INFO" => format!("\x1b[32m{}\x1b[0m", level),  // green
        "DEBUG" => format!("\x1b[36m{}\x1b[0m", level), // cyan
        "TRACE" => format!("\x1b[90m{}\x1b[0m", level), // gray
        _ => level.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_colorize_level() {
        assert!(colorize_level("ERROR").contains("\x1b[31m"));
        assert!(colorize_level("WARN").contains("\x1b[33m"));
        assert!(colorize_level("INFO").contains("\x1b[32m"));
        assert!(colorize_level("DEBUG").contains("\x1b[36m"));
        assert!(colorize_level("TRACE").contains("\x1b[90m"));
        assert_eq!(colorize_level("UNKNOWN"), "UNKNOWN");
    }

    #[test]
    fn test_convert_to_local_time_valid() {
        let ts = "2024-01-15T10:30:00.000Z";
        let result = convert_to_local_time(ts);
        assert!(result.contains("2024-01-15"));
    }

    #[test]
    fn test_convert_to_local_time_invalid() {
        let ts = "not-a-timestamp";
        assert_eq!(convert_to_local_time(ts), "not-a-timestamp");
    }

    #[test]
    fn test_print_log_entry_json() {
        let entry = serde_json::json!({
            "level": "INFO",
            "target": "ironclaw::agent",
            "message": "test message",
            "timestamp": "2024-01-15T10:30:00.000Z"
        });
        let cmd = LogsCommand {
            follow: false,
            limit: 200,
            json: true,
            local_time: false,
            plain: false,
            url: None,
            token: None,
            timeout: 5000,
            level: None,
            grep: None,
            ignore_case: false,
            context: 0,
        };
        // Should not panic
        print_log_entry(&entry, &cmd);
    }

    #[test]
    fn test_tail_file_small() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.log");
        std::fs::write(&path, "line1\nline2\nline3\nline4\nline5\n").unwrap();

        let result = tail_file(&path, 3).unwrap();
        assert_eq!(result, vec!["line3", "line4", "line5"]);
    }

    #[test]
    fn test_tail_file_fewer_lines_than_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.log");
        std::fs::write(&path, "a\nb\n").unwrap();

        let result = tail_file(&path, 200).unwrap();
        assert_eq!(result, vec!["a", "b"]);
    }

    #[test]
    fn test_tail_file_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.log");
        std::fs::write(&path, "").unwrap();

        let result = tail_file(&path, 10).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_tail_file_large() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.log");
        // Write 10000 lines to test chunked reading.
        let content: String = (0..10000).map(|i| format!("line {}\n", i)).collect();
        std::fs::write(&path, &content).unwrap();

        let result = tail_file(&path, 5).unwrap();
        assert_eq!(result.len(), 5);
        assert_eq!(result[0], "line 9995");
        assert_eq!(result[4], "line 9999");
    }

    #[test]
    fn test_tail_file_no_trailing_newline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.log");
        std::fs::write(&path, "line1\nline2\nline3").unwrap();

        let result = tail_file(&path, 2).unwrap();
        assert_eq!(result, vec!["line2", "line3"]);
    }

    // ── New tests for --grep functionality ─────────────────────────────

    #[test]
    fn test_filter_lines_exact_match() -> anyhow::Result<()> {
        let lines = vec![
            "INFO: started".to_string(),
            "ERROR: failed".to_string(),
            "DEBUG: test".to_string(),
        ];

        let filtered = filter_lines(lines, "ERROR", false, 0)?;
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0], "ERROR: failed");

        Ok(())
    }

    #[test]
    fn test_filter_lines_ignore_case() -> anyhow::Result<()> {
        let lines = vec![
            "ERROR: fail".to_string(),
            "error: also fail".to_string(),
            "INFO: ok".to_string(),
        ];

        let filtered = filter_lines(lines, "error", true, 0)?;
        assert_eq!(filtered.len(), 2);

        Ok(())
    }

    #[test]
    fn test_filter_lines_regex() -> anyhow::Result<()> {
        let lines = vec![
            "timeout after 5s".to_string(),
            "connection refused".to_string(),
            "ok".to_string(),
        ];

        let filtered = filter_lines(lines, "timeout|refused", false, 0)?;
        assert_eq!(filtered.len(), 2);

        Ok(())
    }

    #[test]
    fn test_filter_lines_with_context() -> anyhow::Result<()> {
        let lines = vec![
            "line 1".to_string(),
            "ERROR: panic".to_string(), // match
            "line 3".to_string(),
            "line 4".to_string(),
            "ERROR: crash".to_string(), // match
            "line 6".to_string(),
        ];

        let filtered = filter_lines(lines, "ERROR", false, 1)?;
        // Should include: line 1, ERROR: panic, line 3, line 4, ERROR: crash, line 6
        // (with deduplication for overlapping context)
        assert_eq!(filtered.len(), 6);

        Ok(())
    }

    #[test]
    fn test_filter_lines_with_non_overlapping_context() -> anyhow::Result<()> {
        // Create 10 lines: "line 0" .. "line 9"
        let mut lines: Vec<String> = (0..10).map(|i| format!("line {}", i)).collect();

        // Patch in matches at indices 1 and 8 (far apart)
        lines[1] = "ERROR: first".to_string();
        lines[8] = "ERROR: second".to_string();

        // Filter with context=1:
        // - Match at index 1 → window [0,1,2]
        // - Match at index 8 → window [7,8,9]
        // Total: 6 unique lines (no overlap between groups)
        let filtered = filter_lines(lines, "ERROR", false, 1)?;

        assert_eq!(filtered.len(), 6);
        assert_eq!(filtered[0], "line 0"); // start of first group
        assert_eq!(filtered[2], "line 2"); // end of first group
        assert_eq!(filtered[3], "line 7"); // start of second group
        assert_eq!(filtered[5], "line 9"); // end of second group

        Ok(())
    }

    #[test]
    fn test_filter_lines_no_matches() -> anyhow::Result<()> {
        let lines = vec!["INFO: ok".to_string(), "DEBUG: test".to_string()];

        let filtered = filter_lines(lines, "ERROR", false, 0)?;
        assert!(filtered.is_empty());

        Ok(())
    }

    #[test]
    fn test_filter_lines_invalid_regex() {
        let lines = vec!["test".to_string()];
        let result = filter_lines(lines, "[invalid", false, 0);
        assert!(result.unwrap_err().to_string().contains("Invalid regex"));
    }
}
