//! Debug inspection endpoints (admin-only).
//!
//! | Method | Path | Handler |
//! |--------|------|---------|
//! | GET | `/api/debug/prompt` | [`debug_prompt_handler`] |
//!
//! Returns a reconstructed view of the current system prompt — workspace
//! identity files (AGENTS.md, SOUL.md, USER.md, IDENTITY.md, TOOLS.md,
//! MEMORY.md), plus the assembled system prompt — so admins can inspect
//! what the agent is seeing without scraping logs. The reconstruction
//! uses the gateway's configured default timezone, so the prompt closely
//! matches what the runtime would emit but is *not* guaranteed to match
//! a specific past turn (workspace files may have changed since).
//!
//! Admin-only by virtue of the [`AdminUser`] extractor.

use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};
use serde::Serialize;

use crate::channels::web::auth::AdminUser;
use crate::channels::web::platform::state::GatewayState;

#[derive(Serialize)]
pub(crate) struct DebugPromptResponse {
    components: Vec<DebugPromptComponent>,
    total_estimated_tokens: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_prompt: Option<String>,
    model: String,
    context_limit: usize,
    note: &'static str,
}

#[derive(Serialize)]
struct DebugPromptComponent {
    source: String,
    label: String,
    content: String,
    estimated_tokens: usize,
}

/// Crude token estimator. Word-based for whitespace-separated text; falls
/// back to a character-based heuristic for CJK and other scripts where
/// `split_whitespace` produces few tokens. Always adds a small constant for
/// fixed prompt overhead. The estimate is meant for the debug panel only —
/// off by 10–20% is fine.
fn estimate_tokens(text: &str) -> usize {
    let words = text.split_whitespace().count();
    let chars = text.len();
    if words == 0 {
        return 4;
    }
    if chars / words > 10 {
        // Character-based: ~1.5 chars per token for CJK text.
        (chars as f64 / 1.5) as usize + 4
    } else {
        ((words as f64) * 1.3) as usize + 4
    }
}

pub(crate) async fn debug_prompt_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(user): AdminUser,
) -> Result<Json<DebugPromptResponse>, (StatusCode, String)> {
    tracing::debug!(user_id = %user.user_id, "debug prompt endpoint accessed");
    let workspace =
        crate::channels::web::handlers::memory::resolve_workspace(&state, &user).await?;

    let files: &[(&str, &str)] = &[
        ("AGENTS.md", "Agent Instructions"),
        ("SOUL.md", "Core Values"),
        ("USER.md", "User Context"),
        ("IDENTITY.md", "Identity"),
        ("TOOLS.md", "Tool Notes"),
        ("MEMORY.md", "Long-Term Memory"),
    ];

    let mut components = Vec::new();
    for &(path, label) in files {
        if let Ok(doc) = workspace.read(path).await
            && !doc.content.is_empty()
        {
            let est = estimate_tokens(&doc.content);
            components.push(DebugPromptComponent {
                source: path.to_string(),
                label: label.to_string(),
                content: doc.content,
                estimated_tokens: est,
            });
        }
    }

    let active_config = state.active_config.read().await.clone();

    // Empty string (the Default) and unrecognised strings both fall back to UTC.
    let prompt_tz = active_config
        .default_timezone
        .parse::<chrono_tz::Tz>()
        .unwrap_or(chrono_tz::UTC);
    let system_prompt = match workspace
        .system_prompt_for_context_tz(false, prompt_tz)
        .await
    {
        Ok(prompt) if !prompt.is_empty() => Some(prompt),
        _ => None,
    };

    let total: usize = components.iter().map(|c| c.estimated_tokens).sum();
    Ok(Json(DebugPromptResponse {
        components,
        total_estimated_tokens: total,
        system_prompt,
        model: active_config.llm_model,
        context_limit: crate::agent::context_monitor::DEFAULT_CONTEXT_LIMIT,
        note: "reconstructed, may differ from last turn",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_tokens_empty_string() {
        assert_eq!(estimate_tokens(""), 4);
    }

    #[test]
    fn test_estimate_tokens_ascii_english() {
        let text = "hello world foo bar";
        let words = 4_usize;
        let expected = ((words as f64) * 1.3) as usize + 4;
        assert_eq!(estimate_tokens(text), expected);
    }

    #[test]
    fn test_estimate_tokens_cjk() {
        let text = "你好世界测试文字内容估算";
        let chars = text.len();
        let expected = (chars as f64 / 1.5) as usize + 4;
        assert_eq!(estimate_tokens(text), expected);
    }

    #[test]
    fn test_estimate_tokens_mixed_cjk_latin() {
        let text = "hello 你好 world 世界";
        let words = text.split_whitespace().count();
        let chars = text.len();
        let result = estimate_tokens(text);
        let expected = if chars / words > 10 {
            (chars as f64 / 1.5) as usize + 4
        } else {
            ((words as f64) * 1.3) as usize + 4
        };
        assert_eq!(result, expected);
    }

    #[test]
    fn test_estimate_tokens_whitespace_only() {
        assert_eq!(estimate_tokens("   \t\n  "), 4);
    }
}
