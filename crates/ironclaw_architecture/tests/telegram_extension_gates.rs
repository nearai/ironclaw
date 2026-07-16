//! Telegram extension architecture gates.
//!
//! 1. Retired-taxonomy identifiers stay dead: the single `telegram` extension
//!    must never grow a `telegram_bot` / `telegram_personal` /
//!    `telegram_channel` companion identity (the pattern #6116's
//!    `reborn_retired_taxonomy` gate pins for Slack).
//! 2. The Reborn context stays free of the v1 pairing surface: no
//!    `/api/pairing/` route literals in `crates/` or the webui v2 frontend —
//!    Telegram pairing is the WebGeneratedCode flow under
//!    `/api/webchat/v2/channels/telegram/pairing`.

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("architecture crate lives two levels under the workspace root")
        .to_path_buf()
}

fn rust_and_frontend_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                if name == "target" || name == "node_modules" || name == ".git" || name == "dist" {
                    continue;
                }
                stack.push(path);
                continue;
            }
            let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
            if matches!(extension, "rs" | "ts" | "tsx" | "js" | "jsx" | "toml") {
                files.push(path);
            }
        }
    }
    files
}

/// Retired identifiers with the legitimate longer-identifier continuations
/// that may embed them: `telegram_bot_token` (the credential handle) and the
/// `telegram_bot_api` module are not the retired `telegram_bot` extension
/// identity; `telegram_channel_routes`/`telegram_channel_connection` are host
/// module names, not a `telegram_channel` companion extension.
const RETIRED_TELEGRAM_IDENTIFIERS: &[(&str, &[&str])] = &[
    ("telegram_personal", &[]),
    // `_after` covers `activate_telegram_channel_after_setup_save`, the
    // mirror of slack's setup-activation trait method name.
    (
        "telegram_channel",
        &["_route", "_connection", "_setup", "_after"],
    ),
    ("telegram_bot", &["_token", "_api"]),
];

/// Whether `line` uses `needle` as a retired identifier: any occurrence that
/// stands alone as an identifier token (bare, quoted, dotted, …) or that
/// continues into a longer identifier NOT on the allowlist. Catches both
/// `"telegram_bot"` string literals and bare `telegram_bot` identifiers while
/// letting `telegram_bot_token` through — a plain substring needle can do
/// only one of those.
fn uses_retired_identifier(line: &str, needle: &str, allowed_continuations: &[&str]) -> bool {
    let mut search_from = 0;
    while let Some(offset) = line[search_from..].find(needle) {
        let start = search_from + offset;
        let end = start + needle.len();
        search_from = start + 1;
        let right_continues_identifier = line[end..]
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
        if !right_continues_identifier {
            return true;
        }
        if !allowed_continuations
            .iter()
            .any(|allowed| line[end..].starts_with(allowed))
        {
            return true;
        }
    }
    false
}

#[test]
fn retired_identifier_matcher_catches_bare_and_quoted_forms() {
    for offending in [
        r#"let id = "telegram_bot";"#,
        "mod telegram_bot;",
        "telegram_bot.activate()",
        "let telegram_channel = 1;",
        "telegram_bot_service()",
        "telegram_personal_token",
    ] {
        let flagged = RETIRED_TELEGRAM_IDENTIFIERS
            .iter()
            .any(|(needle, allowed)| uses_retired_identifier(offending, needle, allowed));
        assert!(flagged, "must flag: {offending}");
    }
    for legitimate in [
        r#"let handle = "telegram_bot_token";"#,
        "use crate::telegram::telegram_bot_api::TelegramBotApi;",
        "mod telegram_channel_routes;",
        "use crate::telegram::telegram_channel_connection::TelegramPairedStatusSlot;",
        "TELEGRAM_BOT_TOKEN_HANDLE_PREFIX",
    ] {
        let flagged = RETIRED_TELEGRAM_IDENTIFIERS
            .iter()
            .any(|(needle, allowed)| uses_retired_identifier(legitimate, needle, allowed));
        assert!(!flagged, "must allow: {legitimate}");
    }
}

#[test]
fn no_retired_taxonomy_telegram_identifiers() {
    let root = workspace_root();
    let mut offenders = Vec::new();
    for file in rust_and_frontend_files(&root.join("crates")) {
        let display = file.display().to_string();
        if display.contains("telegram_extension_gates.rs") {
            continue;
        }
        // `crates/ironclaw_gateway/static` is the v1 monolith's embedded UI,
        // retained until the monolith retires — not reborn context.
        if display.contains("ironclaw_gateway/static") {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(&file) else {
            continue;
        };
        for (line_number, line) in contents.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with("*") {
                continue;
            }
            for (needle, allowed_continuations) in RETIRED_TELEGRAM_IDENTIFIERS {
                if uses_retired_identifier(line, needle, allowed_continuations) {
                    offenders.push(format!("{display}:{}: {needle}", line_number + 1));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "retired-taxonomy telegram identifiers found (one `telegram` extension only — \
         no bot/personal/channel companions):\n{}",
        offenders.join("\n")
    );
}

#[test]
fn reborn_context_free_of_v1_pairing_routes() {
    let root = workspace_root();
    let mut offenders = Vec::new();
    for file in rust_and_frontend_files(&root.join("crates")) {
        let display = file.display().to_string();
        if display.contains("telegram_extension_gates.rs")
            || display.contains("ironclaw_gateway/static")
        {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(&file) else {
            continue;
        };
        for (line_number, line) in contents.lines().enumerate() {
            if !line.contains("/api/pairing/") {
                continue;
            }
            let trimmed = line.trim_start();
            // Prose references in comments may describe the v1 monolith
            // surface; only executable string literals are violations.
            if trimmed.starts_with("//") || trimmed.starts_with("*") || trimmed.starts_with("///") {
                continue;
            }
            offenders.push(format!("{display}:{}", line_number + 1));
        }
    }
    assert!(
        offenders.is_empty(),
        "v1 pairing route literals found in the reborn context (telegram pairing is \
         /api/webchat/v2/channels/telegram/pairing):\n{}",
        offenders.join("\n")
    );
}
