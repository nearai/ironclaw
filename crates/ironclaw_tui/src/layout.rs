//! User-configurable TUI layout.
//!
//! Layout is loaded from `tui/layout.json` in the workspace directory.
//! If the file doesn't exist, sensible defaults are used.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::theme::Theme;

/// Top-level layout configuration for the TUI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuiLayout {
    /// Theme name or inline theme definition.
    #[serde(default = "default_theme_name")]
    pub theme: String,

    /// Header bar configuration.
    #[serde(default)]
    pub header: HeaderConfig,

    /// Status bar configuration.
    #[serde(default)]
    pub status_bar: StatusBarConfig,

    /// Conversation area configuration.
    #[serde(default)]
    pub conversation: ConversationConfig,

    /// Key binding overrides: action name -> key combo string.
    #[serde(default)]
    pub keybindings: HashMap<String, String>,

    /// Per-widget configuration overrides.
    #[serde(default)]
    pub widgets: HashMap<String, serde_json::Value>,
}

fn default_theme_name() -> String {
    "dark".to_string()
}

impl Default for TuiLayout {
    fn default() -> Self {
        Self {
            theme: default_theme_name(),
            header: HeaderConfig::default(),
            status_bar: StatusBarConfig::default(),
            conversation: ConversationConfig::default(),
            keybindings: HashMap::new(),
            widgets: HashMap::new(),
        }
    }
}

impl TuiLayout {
    /// Load layout from a JSON file, falling back to defaults on any error.
    pub fn load_from_file(path: &Path) -> Self {
        let Ok(contents) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        let Ok(parsed) = serde_json::from_str::<Value>(&contents) else {
            return Self::default();
        };

        let mut layout: Self = serde_json::from_value(parsed.clone()).unwrap_or_default();
        let conversation = parsed.get("conversation");
        if let Some(sidebar) = parsed.get("sidebar") {
            if conversation
                .and_then(|cfg| cfg.get("show_work_sidebar"))
                .is_none()
                && let Some(visible) = sidebar.get("visible").and_then(Value::as_bool)
            {
                layout.conversation.show_work_sidebar = visible;
            }
            if conversation
                .and_then(|cfg| cfg.get("work_sidebar_width_percent"))
                .is_none()
                && let Some(width) = sidebar
                    .get("width_percent")
                    .and_then(Value::as_u64)
                    .and_then(|width| u16::try_from(width).ok())
            {
                layout.conversation.work_sidebar_width_percent = Some(width);
            }
        }

        layout
    }

    /// Resolve the theme from the layout's theme name.
    pub fn resolve_theme(&self) -> Theme {
        match self.theme.as_str() {
            "light" => Theme::light(),
            _ => Theme::dark(),
        }
    }
}

fn default_true() -> bool {
    true
}

/// Header bar configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeaderConfig {
    #[serde(default = "default_true")]
    pub visible: bool,

    #[serde(default = "default_true")]
    pub show_model: bool,

    #[serde(default = "default_true")]
    pub show_tokens: bool,

    #[serde(default = "default_true")]
    pub show_session_duration: bool,
}

impl Default for HeaderConfig {
    fn default() -> Self {
        Self {
            visible: false,
            show_model: true,
            show_tokens: true,
            show_session_duration: true,
        }
    }
}

/// Status bar configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusBarConfig {
    #[serde(default = "default_true")]
    pub visible: bool,

    #[serde(default = "default_true")]
    pub show_cost: bool,

    #[serde(default = "default_true")]
    pub show_keybinds: bool,
}

impl Default for StatusBarConfig {
    fn default() -> Self {
        Self {
            visible: true,
            show_cost: true,
            show_keybinds: true,
        }
    }
}

/// Conversation area configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationConfig {
    /// Show tool call details inline in conversation.
    #[serde(default = "default_true")]
    pub show_tool_details: bool,

    /// Show the optional right-side work summary sidebar in chat.
    #[serde(default = "default_true")]
    pub show_work_sidebar: bool,

    /// Optional fixed width percentage for the right-side work sidebar.
    /// When omitted, the TUI uses responsive built-in widths.
    #[serde(default)]
    pub work_sidebar_width_percent: Option<u16>,

    /// Maximum number of messages to keep in the visible buffer.
    #[serde(default = "default_max_messages")]
    pub max_visible_messages: usize,
}

fn default_max_messages() -> usize {
    200
}

impl Default for ConversationConfig {
    fn default() -> Self {
        Self {
            show_tool_details: true,
            show_work_sidebar: true,
            work_sidebar_width_percent: None,
            max_visible_messages: default_max_messages(),
        }
    }
}

/// Where widgets can be placed in the TUI layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TuiSlot {
    Header,
    StatusBarLeft,
    StatusBarCenter,
    StatusBarRight,
    ConversationBanner,
    InputPrefix,
    Tab,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_layout_is_valid() {
        let layout = TuiLayout::default();
        assert_eq!(layout.theme, "dark");
        assert!(!layout.header.visible);
        assert!(layout.status_bar.visible);
    }

    #[test]
    fn layout_serialization_round_trip() {
        let layout = TuiLayout::default();
        let json = serde_json::to_string(&layout).expect("serialize");
        let back: TuiLayout = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.theme, "dark");
    }

    #[test]
    fn resolve_theme_dark() {
        let layout = TuiLayout::default();
        let theme = layout.resolve_theme();
        assert_eq!(theme.name, "dark");
    }

    #[test]
    fn resolve_theme_light() {
        let layout = TuiLayout {
            theme: "light".to_string(),
            ..Default::default()
        };
        let theme = layout.resolve_theme();
        assert_eq!(theme.name, "light");
    }

    fn unique_test_layout_path(test_name: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("ironclaw-layout-{test_name}-{nanos}"));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir.join("layout.json")
    }

    #[test]
    fn load_from_file_migrates_legacy_sidebar_settings() {
        let path = unique_test_layout_path("legacy-sidebar");
        std::fs::write(
            &path,
            r#"{
                "theme": "light",
                "sidebar": {"visible": false, "width_percent": 41}
            }"#,
        )
        .expect("write layout");

        let layout = TuiLayout::load_from_file(&path);

        assert_eq!(layout.theme, "light");
        assert!(!layout.conversation.show_work_sidebar);
        assert_eq!(layout.conversation.work_sidebar_width_percent, Some(41));
    }

    #[test]
    fn load_from_file_prefers_new_conversation_sidebar_settings() {
        let path = unique_test_layout_path("conversation-sidebar");
        std::fs::write(
            &path,
            r#"{
                "sidebar": {"visible": false, "width_percent": 41},
                "conversation": {
                    "show_work_sidebar": true,
                    "work_sidebar_width_percent": 28
                }
            }"#,
        )
        .expect("write layout");

        let layout = TuiLayout::load_from_file(&path);

        assert!(layout.conversation.show_work_sidebar);
        assert_eq!(layout.conversation.work_sidebar_width_percent, Some(28));
    }
}
