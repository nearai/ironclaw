//! Layout configuration types for frontend customization.
//!
//! A [`LayoutConfig`] is stored as `frontend/layout.json` in the workspace.
//! It controls branding, tab visibility/order, chat features, and per-widget
//! configuration. All fields are optional with sensible defaults.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Top-level layout configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LayoutConfig {
    /// Branding overrides (title, logo, colors).
    #[serde(default)]
    pub branding: BrandingConfig,

    /// Tab bar configuration.
    #[serde(default)]
    pub tabs: TabConfig,

    /// Chat panel configuration.
    #[serde(default)]
    pub chat: ChatConfig,

    /// Per-widget instance configuration (keyed by widget ID).
    #[serde(default)]
    pub widgets: HashMap<String, WidgetInstanceConfig>,
}

/// Branding overrides for the gateway UI.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BrandingConfig {
    /// Page title (replaces default "IronClaw").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    /// Subtitle shown below the title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,

    /// URL to a logo image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo_url: Option<String>,

    /// URL to a custom favicon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub favicon_url: Option<String>,

    /// Color overrides (injected as CSS custom properties on `:root`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colors: Option<BrandingColors>,
}

/// Color overrides for the UI theme.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BrandingColors {
    /// Primary brand color (e.g., `"#0066cc"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary: Option<String>,

    /// Accent color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
}

/// Tab bar layout configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TabConfig {
    /// Ordered list of tab IDs to display (built-in + widget tabs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<Vec<String>>,

    /// Tab IDs to hide from the tab bar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<Vec<String>>,

    /// Default tab to show on load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_tab: Option<String>,
}

/// Chat panel feature flags.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatConfig {
    /// Show suggestion chips below the input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestions: Option<bool>,

    /// Enable image upload in the chat input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_upload: Option<bool>,
}

/// Per-widget instance configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WidgetInstanceConfig {
    /// Whether this widget is enabled.
    #[serde(default)]
    pub enabled: bool,

    /// Arbitrary widget-specific configuration passed to `widget.init()`.
    #[serde(default)]
    pub config: serde_json::Value,
}

impl BrandingConfig {
    /// Generate CSS custom property overrides for injection into `:root`.
    pub fn to_css_vars(&self) -> String {
        let mut vars = Vec::new();
        if let Some(ref colors) = self.colors {
            if let Some(ref primary) = colors.primary {
                vars.push(format!("--color-primary: {};", primary));
            }
            if let Some(ref accent) = colors.accent {
                vars.push(format!("--color-accent: {};", accent));
            }
        }
        if vars.is_empty() {
            String::new()
        } else {
            format!(":root {{ {} }}", vars.join(" "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layout_config_default_is_empty() {
        let config = LayoutConfig::default();
        assert!(config.branding.title.is_none());
        assert!(config.tabs.order.is_none());
        assert!(config.widgets.is_empty());
    }

    #[test]
    fn test_layout_config_roundtrip() {
        let json = serde_json::json!({
            "branding": { "title": "Acme AI", "colors": { "primary": "#0066cc" } },
            "tabs": { "order": ["chat", "memory"], "hidden": ["routines"] },
            "widgets": { "dashboard": { "enabled": true, "config": { "refresh": 30 } } }
        });
        let config: LayoutConfig = serde_json::from_value(json).unwrap();
        assert_eq!(config.branding.title.as_deref(), Some("Acme AI"));
        assert_eq!(config.tabs.hidden.as_ref().map(|h| h.len()), Some(1));
        assert!(config.widgets.get("dashboard").is_some_and(|w| w.enabled));
    }

    #[test]
    fn test_branding_css_vars_empty() {
        let branding = BrandingConfig::default();
        assert!(branding.to_css_vars().is_empty());
    }

    #[test]
    fn test_branding_css_vars_with_colors() {
        let branding = BrandingConfig {
            colors: Some(BrandingColors {
                primary: Some("#0066cc".to_string()),
                accent: Some("#ff6b00".to_string()),
            }),
            ..Default::default()
        };
        let css = branding.to_css_vars();
        assert!(css.contains("--color-primary: #0066cc;"));
        assert!(css.contains("--color-accent: #ff6b00;"));
    }

    #[test]
    fn test_partial_deserialization() {
        let json = serde_json::json!({"branding": {"title": "Test"}});
        let config: LayoutConfig = serde_json::from_value(json).unwrap();
        assert_eq!(config.branding.title.as_deref(), Some("Test"));
        assert!(config.chat.suggestions.is_none());
    }
}
