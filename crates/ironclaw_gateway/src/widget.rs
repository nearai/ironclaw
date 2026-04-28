//! Widget system types and utilities.
//!
//! Widgets are self-contained frontend components that plug into named
//! [`WidgetSlot`]s in the UI. Each widget has a manifest (`manifest.json`)
//! and implementation files (`index.js`, optional `style.css`).

use serde::{Deserialize, Serialize};

/// Widget manifest — metadata about a widget component.
///
/// Stored as `.system/gateway/widgets/{id}/manifest.json` in the workspace.
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
    /// Area inside a project drill-in view, above the missions/activity grid.
    /// Good for: metrics dashboards, KPI cards, project health summaries.
    ProjectHeader,
    /// Additional section alongside missions and activity in the project
    /// drill-in body. Good for: custom reports, data tables, embeds.
    ProjectSection,
}

/// Prefix every CSS selector with `[data-widget="{widget_id}"]` for style isolation.
///
/// This prevents widget styles from bleeding into the main app or other widgets.
/// The widget container element gets `data-widget="{id}"` set by the runtime.
///
/// The parser tracks brace depth and handles nested grouping at-rules like
/// `@media`, `@supports`, `@container`, `@layer`, `@document`, and `@scope`:
/// selectors inside those blocks are scoped the same way as top-level selectors.
/// Other at-rules (`@keyframes`, `@font-face`, `@page`, etc.) are passed through
/// verbatim — their bodies contain declarations or keyframe selectors that
/// must not be prefixed with `[data-widget=…]`.
///
/// This is a brace-aware text transform, not a real CSS parser: it does not
/// understand CSS strings, comments, or the CSS Nesting spec. Widget CSS
/// should avoid putting `{`/`}` inside string literals or comments.
///
/// # Example
///
/// ```
/// use ironclaw_gateway::scope_css;
///
/// let scoped = scope_css(".title { color: red; }", "my-widget");
/// assert!(scoped.contains("[data-widget=\"my-widget\"] .title"));
/// ```
pub fn scope_css(css: &str, widget_id: &str) -> String {
    let prefix = format!("[data-widget=\"{}\"]", widget_id);
    let mut result = String::with_capacity(css.len() + css.len() / 4);

    // Stack entry kind — `true` means the enclosing block is a rule list
    // (top level, inside `@media`, etc.), so the next `{` opens a rule whose
    // selector should be scoped. `false` means the enclosing block holds
    // declarations (or opaque at-rule content like `@keyframes`), so we copy
    // everything verbatim while still balancing braces.
    let mut stack: Vec<bool> = Vec::new();
    let mut current_selector = String::new();

    for ch in css.chars() {
        // Top-of-stack — default to "rule list" when empty (top level).
        let in_rule_list = stack.last().copied().unwrap_or(true);

        if !in_rule_list {
            // Verbatim mode: copy chars through but track nesting so the
            // matching `}` pops the right frame. This keeps `@keyframes`
            // and other opaque at-rules intact.
            match ch {
                '{' => {
                    stack.push(false);
                    result.push('{');
                }
                '}' => {
                    stack.pop();
                    result.push('}');
                }
                _ => result.push(ch),
            }
            continue;
        }

        match ch {
            '{' => {
                let selector = current_selector.trim();
                if is_grouping_atrule(selector) {
                    // Emit the at-rule header as-is; its body holds more rules
                    // (selectors inside will be scoped on the next iteration).
                    result.push_str(selector);
                    result.push_str(" {");
                    stack.push(true);
                } else {
                    // Regular rule — scope each comma-separated selector. A
                    // non-grouping at-rule (like `@keyframes`) is passed through
                    // and its body is treated as opaque.
                    let parts: Vec<String> = selector
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(|s| {
                            if s.starts_with('@') {
                                s.to_string()
                            } else {
                                format!("{} {}", prefix, s)
                            }
                        })
                        .collect();
                    result.push_str(&parts.join(", "));
                    result.push_str(" {");
                    stack.push(false);
                }
                current_selector.clear();
            }
            '}' => {
                // Close an enclosing rule list (e.g., the outer `}` of `@media`).
                // A stray `}` at the very top level is malformed but we pass
                // it through rather than dropping it.
                stack.pop();
                result.push('}');
                current_selector.clear();
            }
            _ => current_selector.push(ch),
        }
    }

    // Any trailing unterminated content (malformed CSS) is passed through.
    if !current_selector.trim().is_empty() {
        result.push_str(&current_selector);
    }

    result
}

/// `true` if the given CSS fragment is a grouping at-rule whose body contains
/// more rules (not declarations). Selectors inside these at-rules should be
/// scoped recursively.
fn is_grouping_atrule(selector: &str) -> bool {
    let s = selector.trim_start();
    if !s.starts_with('@') {
        return false;
    }
    // The at-rule name is everything after `@` up to the first whitespace,
    // `(`, or `{`. Compare case-insensitively.
    let name: String = s[1..]
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != '(' && *c != '{')
        .collect::<String>()
        .to_ascii_lowercase();
    matches!(
        name.as_str(),
        "media" | "supports" | "container" | "layer" | "document" | "scope"
    )
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

    /// Count `{` / `}` in a string and return `(open, close)`. A well-formed
    /// CSS output must have equal counts.
    fn brace_counts(s: &str) -> (usize, usize) {
        (s.matches('{').count(), s.matches('}').count())
    }

    #[test]
    fn test_scope_css_at_rule_not_prefixed() {
        // The @media rule itself should not be prefixed.
        let input = "@media (max-width: 768px) { .mobile { display: block; } }";
        let result = scope_css(input, "w");
        assert!(!result.contains("[data-widget=\"w\"] @media"));
    }

    #[test]
    fn test_scope_css_media_query_inner_selector_scoped() {
        // Selectors nested inside @media must still be scoped to the widget.
        let input = "@media (max-width: 768px) { .mobile { display: block; } }";
        let result = scope_css(input, "w");
        assert!(
            result.contains("[data-widget=\"w\"] .mobile"),
            "expected inner .mobile to be scoped, got: {result}"
        );
        // Declarations must be preserved.
        assert!(result.contains("display: block;"));
        // Braces must balance — this is the regression check for the old
        // single-bool parser that dropped the outer `}`.
        let (open, close) = brace_counts(&result);
        assert_eq!(open, close, "unbalanced braces in: {result}");
        assert_eq!(open, 2);
    }

    #[test]
    fn test_scope_css_nested_supports_and_media() {
        // @supports wrapping @media wrapping a rule — three levels of nesting.
        let input =
            "@supports (display: grid) { @media (min-width: 600px) { .grid { display: grid; } } }";
        let result = scope_css(input, "w");
        assert!(result.contains("[data-widget=\"w\"] .grid"));
        let (open, close) = brace_counts(&result);
        assert_eq!(open, close, "unbalanced braces in: {result}");
        assert_eq!(open, 3);
    }

    #[test]
    fn test_scope_css_keyframes_passthrough() {
        // @keyframes bodies hold keyframe-selectors (0%, 100%), NOT element
        // selectors — they must not be prefixed with [data-widget=…]. And
        // the nested { } inside must be correctly balanced.
        let input = "@keyframes fade { 0% { opacity: 0; } 100% { opacity: 1; } }";
        let result = scope_css(input, "w");
        assert!(!result.contains("[data-widget=\"w\"] 0%"));
        assert!(!result.contains("[data-widget=\"w\"] 100%"));
        assert!(result.contains("@keyframes fade"));
        assert!(result.contains("opacity: 0;"));
        assert!(result.contains("opacity: 1;"));
        let (open, close) = brace_counts(&result);
        assert_eq!(open, close, "unbalanced braces in: {result}");
    }

    #[test]
    fn test_scope_css_sibling_rules_inside_media() {
        // Two sibling rules inside a single @media block.
        let input = "@media (max-width: 768px) { .a { color: red; } .b { color: blue; } }";
        let result = scope_css(input, "w");
        assert!(result.contains("[data-widget=\"w\"] .a"));
        assert!(result.contains("[data-widget=\"w\"] .b"));
        let (open, close) = brace_counts(&result);
        assert_eq!(open, close);
    }

    #[test]
    fn test_scope_css_balanced_after_complex_input() {
        // Mix of top-level rules, @media, and @keyframes.
        let input = "
            .header { color: red; }
            @media (max-width: 768px) {
                .header { font-size: 14px; }
                .nav, .footer { padding: 0; }
            }
            @keyframes spin { from { transform: rotate(0); } to { transform: rotate(360deg); } }
        ";
        let result = scope_css(input, "w");
        let (open, close) = brace_counts(&result);
        assert_eq!(open, close, "unbalanced braces in complex input: {result}");
        assert!(result.contains("[data-widget=\"w\"] .header"));
        assert!(result.contains("[data-widget=\"w\"] .nav"));
        assert!(result.contains("[data-widget=\"w\"] .footer"));
        assert!(!result.contains("[data-widget=\"w\"] from"));
        assert!(!result.contains("[data-widget=\"w\"] to"));
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
            WidgetSlot::ProjectHeader,
            WidgetSlot::ProjectSection,
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
