//! User-configurable TUI layout.
//!
//! Layout is loaded from `tui/layout.json` in the workspace directory.
//! If the file doesn't exist, sensible defaults are used.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::theme::{Theme, ThemeOverrides};

/// Top-level layout configuration for the TUI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuiLayout {
    /// Theme name or inline theme definition.
    #[serde(default = "default_theme_name")]
    pub theme: String,

    /// Optional color-token overrides applied on top of the named theme.
    #[serde(default)]
    pub theme_overrides: ThemeOverrides,

    /// Header bar configuration.
    #[serde(default)]
    pub header: HeaderConfig,

    /// Status bar configuration.
    #[serde(default)]
    pub status_bar: StatusBarConfig,

    /// Conversation area configuration.
    #[serde(default)]
    pub conversation: ConversationConfig,

    /// Shell chrome configuration.
    #[serde(default)]
    pub shell: ShellConfig,

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
            theme_overrides: ThemeOverrides::default(),
            header: HeaderConfig::default(),
            status_bar: StatusBarConfig::default(),
            conversation: ConversationConfig::default(),
            shell: ShellConfig::default(),
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
        let base = match self.theme.as_str() {
            "light" => Theme::light(),
            _ => Theme::dark(),
        };
        base.apply_overrides(&self.theme_overrides)
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TopTabBarMode {
    #[default]
    Auto,
    Full,
    Compact,
    Hidden,
}

/// Shell chrome configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellConfig {
    #[serde(default = "default_true")]
    pub show_nav_rail: bool,

    #[serde(default = "default_nav_rail_width")]
    pub nav_rail_width: u16,

    #[serde(default = "default_true")]
    pub show_surface_header: bool,

    #[serde(default = "default_surface_header_height")]
    pub surface_header_height: u16,

    #[serde(default)]
    pub top_tab_bar_mode: TopTabBarMode,

    #[serde(default = "default_true")]
    pub nav_badges: bool,

    #[serde(default = "default_true")]
    pub nav_hints: bool,
}

fn default_nav_rail_width() -> u16 {
    18
}

fn default_surface_header_height() -> u16 {
    4
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            show_nav_rail: true,
            nav_rail_width: default_nav_rail_width(),
            show_surface_header: true,
            surface_header_height: default_surface_header_height(),
            top_tab_bar_mode: TopTabBarMode::Auto,
            nav_badges: true,
            nav_hints: true,
        }
    }
}

/// Where widgets can be placed in the TUI layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TuiSlot {
    Header,
    NavRail,
    SurfaceHeader,
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
    use crate::theme::ThemeColor;

    #[test]
    fn default_layout_is_valid() {
        let layout = TuiLayout::default();
        assert_eq!(layout.theme, "dark");
        assert!(!layout.header.visible);
        assert!(layout.status_bar.visible);
        assert!(layout.shell.show_nav_rail);
    }

    #[test]
    fn layout_serialization_round_trip() {
        let layout = TuiLayout::default();
        let json = serde_json::to_string(&layout).expect("serialize");
        let back: TuiLayout = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.theme, "dark");
        assert_eq!(back.shell.nav_rail_width, 18);
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

    #[test]
    fn resolve_theme_applies_shell_theme_overrides() {
        let layout = TuiLayout {
            theme_overrides: ThemeOverrides {
                nav_bg: Some(ThemeColor::Rgb { r: 1, g: 2, b: 3 }),
                tab_active_bg: Some(ThemeColor::Rgb { r: 4, g: 5, b: 6 }),
                header_fg: Some(ThemeColor::Rgb { r: 7, g: 8, b: 9 }),
                ..Default::default()
            },
            ..Default::default()
        };

        let theme = layout.resolve_theme();
        assert_eq!(
            theme.nav_bg.to_color(),
            ThemeColor::Rgb { r: 1, g: 2, b: 3 }.to_color()
        );
        assert_eq!(
            theme.tab_active_bg.to_color(),
            ThemeColor::Rgb { r: 4, g: 5, b: 6 }.to_color()
        );
        assert_eq!(
            theme.header_fg.to_color(),
            ThemeColor::Rgb { r: 7, g: 8, b: 9 }.to_color()
        );
    }

    #[test]
    fn load_from_file_reads_shell_chrome_options() {
        let path = unique_test_layout_path("shell-options");
        std::fs::write(
            &path,
            r#"{
                "shell": {
                    "top_tab_bar_mode": "hidden",
                    "nav_badges": false,
                    "nav_hints": false,
                    "surface_header_height": 5
                }
            }"#,
        )
        .expect("write layout");

        let layout = TuiLayout::load_from_file(&path);

        assert_eq!(layout.shell.top_tab_bar_mode, TopTabBarMode::Hidden);
        assert!(!layout.shell.nav_badges);
        assert!(!layout.shell.nav_hints);
        assert_eq!(layout.shell.surface_header_height, 5);
    }
}
