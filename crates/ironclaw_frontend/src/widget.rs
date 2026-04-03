//! Widget system types and utilities.
//!
//! Widgets are self-contained frontend components that plug into named
//! [`WidgetSlot`]s in the UI. Each widget has a manifest (`widget.json`)
//! and implementation files (`index.js`, optional `style.css`).

use serde::{Deserialize, Serialize};

/// Widget manifest — metadata about a widget component.
///
/// Stored as `frontend/widgets/{id}/manifest.json` in the workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WidgetManifest {
    /// Unique widget identifier (must be a valid HTML attribute value).
    pub id: String,

    /// Human-readable widget name.
    pub name: String,

    /// Where this widget is rendered in the UI.
    pub slot: WidgetSlot,

    /// Optional icon identifier (CSS class or emoji).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,

    /// Positioning hint (e.g., `"after:memory"`, `"before:jobs"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<String>,
}

/// Named insertion points in the UI where widgets can be rendered.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WidgetSlot {
    /// Full tab panel (adds a new tab to the tab bar).
    Tab,
    /// Banner area above the chat message list.
    ChatHeader,
    /// Area below the chat input.
    ChatFooter,
    /// Extra action buttons next to the send button.
    ChatActions,
    /// Right sidebar panel.
    Sidebar,
    /// Left side of the status bar.
    StatusLeft,
    /// Right side of the status bar.
    StatusRight,
    /// Additional section in the Settings tab.
    SettingsSection,
    /// Custom inline renderer for structured data in chat messages.
    /// Registered via `IronClaw.registerChatRenderer()` on the browser side.
    ChatRenderer,
}

/// Prefix every CSS selector with `[data-widget="{widget_id}"]` for style isolation.
///
/// This prevents widget styles from bleeding into the main app or other widgets.
/// The widget container element gets `data-widget="{id}"` set by the runtime.
///
/// # Limitations
///
/// This is a simple single-pass parser that does not handle nested at-rules
/// (e.g., `@media { .foo { ... } }`). At-rules with a `@` prefix are passed
/// through unmodified, but their inner selectors are not scoped. For complex
/// widget CSS with media queries, use the `data-widget` attribute selector
/// manually in the widget's CSS.
///
/// # Example
///
/// ```
/// use ironclaw_frontend::scope_css;
///
/// let scoped = scope_css(".title { color: red; }", "my-widget");
/// assert!(scoped.contains("[data-widget=\"my-widget\"] .title"));
/// ```
pub fn scope_css(css: &str, widget_id: &str) -> String {
    let prefix = format!("[data-widget=\"{}\"]", widget_id);
    let mut result = String::with_capacity(css.len() + css.len() / 4);
    let mut in_block = false;
    let mut current_selector = String::new();

    for ch in css.chars() {
        match ch {
            '{' if !in_block => {
                // Scope each comma-separated selector
                let selectors: Vec<&str> = current_selector.split(',').collect();
                let scoped: Vec<String> = selectors
                    .iter()
                    .map(|s| {
                        let s = s.trim();
                        if s.is_empty() || s.starts_with('@') {
                            s.to_string()
                        } else {
                            format!("{} {}", prefix, s)
                        }
                    })
                    .collect();
                result.push_str(&scoped.join(", "));
                result.push_str(" {");
                current_selector.clear();
                in_block = true;
            }
            '}' if in_block => {
                result.push('}');
                in_block = false;
            }
            _ if in_block => {
                result.push(ch);
            }
            _ => {
                current_selector.push(ch);
            }
        }
    }

    // Append any trailing content
    if !current_selector.is_empty() {
        result.push_str(&current_selector);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_widget_manifest_roundtrip() {
        let json = serde_json::json!({
            "id": "dashboard",
            "name": "Analytics Dashboard",
            "slot": "tab",
            "icon": "chart-bar",
            "position": "after:memory"
        });
        let manifest: WidgetManifest = serde_json::from_value(json).unwrap();
        assert_eq!(manifest.id, "dashboard");
        assert_eq!(manifest.slot, WidgetSlot::Tab);
        assert_eq!(manifest.icon.as_deref(), Some("chart-bar"));
    }

    #[test]
    fn test_widget_slot_serialization() {
        assert_eq!(
            serde_json::to_string(&WidgetSlot::ChatHeader).unwrap(),
            "\"chat_header\""
        );
        assert_eq!(
            serde_json::to_string(&WidgetSlot::SettingsSection).unwrap(),
            "\"settings_section\""
        );
    }

    #[test]
    fn test_scope_css_basic() {
        let input = ".title { color: red; }";
        let result = scope_css(input, "my-widget");
        assert!(result.contains("[data-widget=\"my-widget\"] .title"));
        assert!(result.contains("color: red;"));
    }

    #[test]
    fn test_scope_css_multiple_selectors() {
        let input = ".a, .b { margin: 0; }";
        let result = scope_css(input, "w");
        assert!(result.contains("[data-widget=\"w\"] .a"));
        assert!(result.contains("[data-widget=\"w\"] .b"));
    }

    #[test]
    fn test_scope_css_multiple_rules() {
        let input = ".a { color: red; } .b { color: blue; }";
        let result = scope_css(input, "w");
        assert!(result.contains("[data-widget=\"w\"] .a"));
        assert!(result.contains("[data-widget=\"w\"] .b"));
    }

    #[test]
    fn test_scope_css_empty() {
        assert_eq!(scope_css("", "w"), "");
    }

    #[test]
    fn test_scope_css_at_rule_not_prefixed() {
        // At-rules should not get the widget prefix
        let input = "@media (max-width: 768px) { .mobile { display: block; } }";
        let result = scope_css(input, "w");
        // The @media rule itself should not be prefixed
        assert!(!result.contains("[data-widget=\"w\"] @media"));
    }

    #[test]
    fn test_scope_css_preserves_declarations() {
        let input = ".box { padding: 10px; margin: 5px; }";
        let result = scope_css(input, "w");
        assert!(result.contains("padding: 10px;"));
        assert!(result.contains("margin: 5px;"));
    }

    #[test]
    fn test_scope_css_widget_id_with_special_chars() {
        let result = scope_css(".x { color: red; }", "my-widget_v2");
        assert!(result.contains("[data-widget=\"my-widget_v2\"] .x"));
    }

    #[test]
    fn test_widget_slot_all_variants_serialize() {
        // Ensure all slot variants round-trip through serde
        let slots = vec![
            WidgetSlot::Tab,
            WidgetSlot::ChatHeader,
            WidgetSlot::ChatFooter,
            WidgetSlot::ChatActions,
            WidgetSlot::Sidebar,
            WidgetSlot::StatusLeft,
            WidgetSlot::StatusRight,
            WidgetSlot::SettingsSection,
            WidgetSlot::ChatRenderer,
        ];
        for slot in slots {
            let json = serde_json::to_string(&slot).unwrap();
            let back: WidgetSlot = serde_json::from_str(&json).unwrap();
            assert_eq!(slot, back);
        }
    }

    #[test]
    fn test_widget_manifest_minimal() {
        // Manifest with only required fields
        let json = serde_json::json!({
            "id": "test",
            "name": "Test Widget",
            "slot": "tab"
        });
        let manifest: WidgetManifest = serde_json::from_value(json).unwrap();
        assert_eq!(manifest.id, "test");
        assert!(manifest.icon.is_none());
        assert!(manifest.position.is_none());
    }
}
