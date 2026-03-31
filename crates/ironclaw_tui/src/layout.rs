//! User-configurable TUI layout.
//!
//! Layout is loaded from `tui/layout.json` in the workspace directory.
//! If the file doesn't exist, sensible defaults are used.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::theme::Theme;

/// Top-level layout configuration for the TUI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuiLayout {
    /// Theme name or inline theme definition.
    #[serde(default = "default_theme_name")]
    pub theme: String,

    /// Sidebar configuration.
    #[serde(default)]
    pub sidebar: SidebarConfig,

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
            sidebar: SidebarConfig::default(),
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
        match std::fs::read_to_string(path) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Resolve the theme from the layout's theme name.
    pub fn resolve_theme(&self) -> Theme {
        match self.theme.as_str() {
            "light" => Theme::light(),
            _ => Theme::dark(),
        }
    }
}

/// Sidebar panel configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidebarConfig {
    /// Whether the sidebar is visible.
    #[serde(default = "default_true")]
    pub visible: bool,

    /// Sidebar width as percentage of terminal width (10-50).
    #[serde(default = "default_sidebar_width")]
    pub width_percent: u16,
}

fn default_true() -> bool {
    true
}

fn default_sidebar_width() -> u16 {
    25
}

impl Default for SidebarConfig {
    fn default() -> Self {
        Self {
            visible: true,
            width_percent: default_sidebar_width(),
        }
    }
}

impl SidebarConfig {
    /// Clamp width to valid range.
    pub fn effective_width(&self) -> u16 {
        self.width_percent.clamp(10, 50)
    }
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
    Sidebar,
    SidebarSection,
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
        assert!(layout.sidebar.visible);
        assert_eq!(layout.sidebar.effective_width(), 25);
        assert!(layout.header.visible);
        assert!(layout.status_bar.visible);
    }

    #[test]
    fn sidebar_width_clamped() {
        let mut sb = SidebarConfig {
            width_percent: 5,
            ..Default::default()
        };
        assert_eq!(sb.effective_width(), 10);
        sb.width_percent = 80;
        assert_eq!(sb.effective_width(), 50);
    }

    #[test]
    fn layout_serialization_round_trip() {
        let layout = TuiLayout::default();
        let json = serde_json::to_string(&layout).expect("serialize");
        let back: TuiLayout = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.theme, "dark");
        assert_eq!(back.sidebar.width_percent, 25);
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
}
