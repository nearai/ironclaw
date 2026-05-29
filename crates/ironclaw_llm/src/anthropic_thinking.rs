use serde::Serialize;

const THINKING_BUDGET_TOKENS: u32 = 1024;

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub(crate) enum AnthropicThinking {
    #[serde(rename = "enabled")]
    Enabled {
        budget_tokens: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        display: Option<&'static str>,
    },
    #[serde(rename = "adaptive")]
    Adaptive {
        #[serde(skip_serializing_if = "Option::is_none")]
        display: Option<&'static str>,
    },
}

pub(crate) fn thinking_for_request(
    model: &str,
    max_tokens: u32,
    temperature: Option<f32>,
    forces_tool_use: bool,
) -> Option<AnthropicThinking> {
    if max_tokens <= THINKING_BUDGET_TOKENS || temperature.is_some() || forces_tool_use {
        return None;
    }

    let model = model.to_ascii_lowercase();
    if supports_adaptive_thinking(&model) {
        return Some(AnthropicThinking::Adaptive {
            display: Some("summarized"),
        });
    }
    if supports_enabled_thinking(&model) {
        return Some(AnthropicThinking::Enabled {
            budget_tokens: THINKING_BUDGET_TOKENS,
            display: Some("summarized"),
        });
    }
    None
}

fn supports_adaptive_thinking(model: &str) -> bool {
    model.contains("claude-opus-4-6")
        || model.contains("claude-sonnet-4-6")
        || model.contains("claude-opus-4-7")
}

fn supports_enabled_thinking(model: &str) -> bool {
    model.contains("claude-3-7")
        || model.contains("claude-4")
        || model.contains("sonnet-4")
        || model.contains("opus-4")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gates_thinking_to_supported_models_and_valid_request_shapes() {
        assert!(thinking_for_request("claude-sonnet-4-6", 8192, None, false).is_some());
        assert!(thinking_for_request("claude-3-7-sonnet-latest", 8192, None, false).is_some());
        assert!(thinking_for_request("claude-3-5-sonnet-latest", 8192, None, false).is_none());
        assert!(thinking_for_request("claude-sonnet-4-20250514", 1024, None, false).is_none());
        assert!(thinking_for_request("claude-sonnet-4-20250514", 8192, Some(0.2), false).is_none());
        assert!(thinking_for_request("claude-sonnet-4-20250514", 8192, None, true).is_none());
    }
}
