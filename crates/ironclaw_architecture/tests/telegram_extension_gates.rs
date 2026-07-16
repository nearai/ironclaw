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
            for needle in ["telegram_personal", "telegram_channel\"", "\"telegram_bot\""] {
                if line.contains(needle) {
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
            if trimmed.starts_with("//") || trimmed.starts_with("*") || trimmed.starts_with("///")
            {
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
