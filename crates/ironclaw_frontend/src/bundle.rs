//! Frontend bundle assembly.
//!
//! Combines the embedded base HTML with workspace customizations (layout
//! config, widgets, CSS overrides) into the final served page.

use crate::layout::LayoutConfig;
use crate::widget::{WidgetManifest, scope_css};

/// Escape HTML special characters to prevent XSS in text content.
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Escape HTML attribute value (includes quotes).
fn escape_html_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A resolved frontend bundle ready for serving.
///
/// Contains the layout configuration, resolved widgets (with their JS/CSS
/// content loaded), and any custom CSS overrides.
#[derive(Debug, Clone, Default)]
pub struct FrontendBundle {
    /// Layout configuration (branding, tabs, chat settings).
    pub layout: LayoutConfig,

    /// Resolved widgets with their source code loaded.
    pub widgets: Vec<ResolvedWidget>,

    /// Custom CSS to append after the base stylesheet.
    pub custom_css: Option<String>,
}

/// A widget with its manifest and source files loaded.
#[derive(Debug, Clone)]
pub struct ResolvedWidget {
    /// Widget metadata.
    pub manifest: WidgetManifest,

    /// JavaScript source code (`index.js`).
    pub js: String,

    /// Optional CSS source code (`style.css`), auto-scoped.
    pub css: Option<String>,
}

/// Inject frontend customizations into the base HTML template.
///
/// Modifications:
///
/// **Before `</head>`:**
/// - Branding CSS custom property overrides
/// - Title override (replaces `<title>` content)
///
/// **Before `</body>`:**
/// - Layout config as `window.__IRONCLAW_LAYOUT__`
/// - Scoped widget `<style>` blocks
/// - Widget `<script type="module">` tags
/// - Custom CSS `<style>` block
pub fn assemble_index(base_html: &str, bundle: &FrontendBundle) -> String {
    let mut head_injections = Vec::new();
    let mut body_injections = Vec::new();

    // --- Head injections ---

    // Branding CSS variables
    let css_vars = bundle.layout.branding.to_css_vars();
    if !css_vars.is_empty() {
        head_injections.push(format!("<style>{}</style>", css_vars));
    }

    // --- Body injections ---

    // Layout config as global variable
    if let Ok(layout_json) = serde_json::to_string(&bundle.layout) {
        body_injections.push(format!(
            "<script>window.__IRONCLAW_LAYOUT__ = {};</script>",
            layout_json
        ));
    }

    // Widget CSS (scoped) and JS
    for widget in &bundle.widgets {
        if let Some(ref css) = widget.css {
            let scoped = scope_css(css, &widget.manifest.id);
            if !scoped.trim().is_empty() {
                body_injections.push(format!(
                    "<style data-widget=\"{}\">{}</style>",
                    escape_html_attr(&widget.manifest.id),
                    scoped
                ));
            }
        }

        // Widget JS inlined (avoids auth issues with <script src> on protected endpoints).
        // Escape </script> in widget JS to prevent script tag breakout (XSS).
        let safe_js = widget.js.replace("</script>", "<\\/script>");
        body_injections.push(format!(
            "<script type=\"module\" data-widget=\"{}\">\n{}\n</script>",
            escape_html_attr(&widget.manifest.id),
            safe_js
        ));
    }

    // Custom CSS
    if let Some(ref custom_css) = bundle.custom_css
        && !custom_css.trim().is_empty()
    {
        body_injections.push(format!("<style data-custom-css>{}</style>", custom_css));
    }

    // --- Assemble ---

    let mut result = base_html.to_string();

    // Inject before </head>
    if !head_injections.is_empty() {
        let head_block = head_injections.join("\n");
        if let Some(pos) = result.rfind("</head>") {
            result.insert_str(pos, &format!("\n{}\n", head_block));
        }
    }

    // Override <title> if branding title is set (HTML-escaped to prevent XSS)
    if let Some(ref title) = bundle.layout.branding.title
        && let Some(start) = result.find("<title>")
        && let Some(end) = result[start..].find("</title>")
    {
        let end = start + end + "</title>".len();
        result.replace_range(
            start..end,
            &format!("<title>{}</title>", escape_html(title)),
        );
    }

    // Inject before </body>
    if !body_injections.is_empty() {
        let body_block = body_injections.join("\n");
        if let Some(pos) = result.rfind("</body>") {
            result.insert_str(pos, &format!("\n{}\n", body_block));
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::*;
    use crate::widget::*;

    const MINIMAL_HTML: &str =
        "<!DOCTYPE html><html><head><title>IronClaw</title></head><body></body></html>";

    #[test]
    fn test_assemble_index_no_customizations() {
        let bundle = FrontendBundle::default();
        let result = assemble_index(MINIMAL_HTML, &bundle);
        // Layout config is always injected (even when default/empty)
        assert!(result.contains("window.__IRONCLAW_LAYOUT__"));
        // No branding overrides or custom CSS
        assert!(!result.contains("--color-primary"));
        assert!(!result.contains("data-custom-css"));
    }

    #[test]
    fn test_assemble_index_branding_title() {
        let bundle = FrontendBundle {
            layout: LayoutConfig {
                branding: BrandingConfig {
                    title: Some("Acme AI".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let result = assemble_index(MINIMAL_HTML, &bundle);
        assert!(result.contains("<title>Acme AI</title>"));
        assert!(!result.contains("<title>IronClaw</title>"));
    }

    #[test]
    fn test_assemble_index_branding_colors() {
        let bundle = FrontendBundle {
            layout: LayoutConfig {
                branding: BrandingConfig {
                    colors: Some(BrandingColors {
                        primary: Some("#0066cc".to_string()),
                        accent: None,
                    }),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let result = assemble_index(MINIMAL_HTML, &bundle);
        assert!(result.contains("--color-primary: #0066cc;"));
    }

    #[test]
    fn test_assemble_index_layout_config_injected() {
        let bundle = FrontendBundle {
            layout: LayoutConfig {
                tabs: TabConfig {
                    hidden: Some(vec!["routines".to_string()]),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let result = assemble_index(MINIMAL_HTML, &bundle);
        assert!(result.contains("window.__IRONCLAW_LAYOUT__"));
        assert!(result.contains("routines"));
    }

    #[test]
    fn test_assemble_index_widget_script() {
        let bundle = FrontendBundle {
            widgets: vec![ResolvedWidget {
                manifest: WidgetManifest {
                    id: "dashboard".to_string(),
                    name: "Dashboard".to_string(),
                    slot: WidgetSlot::Tab,
                    icon: None,
                    position: None,
                },
                js: "console.log('hello');".to_string(),
                css: Some(".panel { color: red; }".to_string()),
            }],
            ..Default::default()
        };
        let result = assemble_index(MINIMAL_HTML, &bundle);
        assert!(result.contains("data-widget=\"dashboard\""));
        assert!(result.contains("console.log('hello');"));
        assert!(result.contains("data-widget=\"dashboard\""));
        assert!(result.contains("[data-widget=\"dashboard\"] .panel"));
    }

    #[test]
    fn test_assemble_index_custom_css() {
        let bundle = FrontendBundle {
            custom_css: Some("body { background: #111; }".to_string()),
            ..Default::default()
        };
        let result = assemble_index(MINIMAL_HTML, &bundle);
        assert!(result.contains("data-custom-css"));
        assert!(result.contains("background: #111;"));
    }

    // ==================== Security Tests ====================

    #[test]
    fn test_assemble_index_title_xss_escaped() {
        let bundle = FrontendBundle {
            layout: LayoutConfig {
                branding: BrandingConfig {
                    title: Some("<script>alert(1)</script>".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let result = assemble_index(MINIMAL_HTML, &bundle);
        // Title should be HTML-escaped, not rendered as a script tag
        assert!(result.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(!result.contains("<title><script>"));
    }

    #[test]
    fn test_assemble_index_widget_js_script_breakout_escaped() {
        let bundle = FrontendBundle {
            widgets: vec![ResolvedWidget {
                manifest: WidgetManifest {
                    id: "evil".to_string(),
                    name: "Evil Widget".to_string(),
                    slot: WidgetSlot::Tab,
                    icon: None,
                    position: None,
                },
                js: "var x = '</script><script>alert(1)</script>';".to_string(),
                css: None,
            }],
            ..Default::default()
        };
        let result = assemble_index(MINIMAL_HTML, &bundle);
        // </script> in widget JS should be escaped to prevent tag breakout
        assert!(!result.contains("</script><script>alert(1)"));
        assert!(result.contains("<\\/script>"));
    }

    #[test]
    fn test_assemble_index_widget_id_xss_escaped() {
        let bundle = FrontendBundle {
            widgets: vec![ResolvedWidget {
                manifest: WidgetManifest {
                    id: "x\" onload=\"alert(1)".to_string(),
                    name: "XSS Widget".to_string(),
                    slot: WidgetSlot::Tab,
                    icon: None,
                    position: None,
                },
                js: "// safe".to_string(),
                css: None,
            }],
            ..Default::default()
        };
        let result = assemble_index(MINIMAL_HTML, &bundle);
        // Widget ID in attributes should be escaped
        assert!(result.contains("&quot;"));
        assert!(!result.contains("onload=\"alert(1)\""));
    }

    // ==================== Edge Case Tests ====================

    #[test]
    fn test_escape_html_basic() {
        assert_eq!(escape_html("<b>bold</b>"), "&lt;b&gt;bold&lt;/b&gt;");
        assert_eq!(escape_html("a & b"), "a &amp; b");
        assert_eq!(escape_html("safe text"), "safe text");
        assert_eq!(escape_html(""), "");
    }

    #[test]
    fn test_escape_html_attr_quotes() {
        assert_eq!(
            escape_html_attr("value\"with\"quotes"),
            "value&quot;with&quot;quotes"
        );
    }

    #[test]
    fn test_assemble_index_missing_head_body_tags() {
        // Gracefully handles malformed HTML (no </head> or </body>)
        let html = "<html><body>content</body></html>";
        let bundle = FrontendBundle {
            layout: LayoutConfig {
                branding: BrandingConfig {
                    title: Some("Test".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let result = assemble_index(html, &bundle);
        // Should still contain layout config (injected before </body>)
        assert!(result.contains("window.__IRONCLAW_LAYOUT__"));
    }

    #[test]
    fn test_assemble_index_empty_widget_js() {
        let bundle = FrontendBundle {
            widgets: vec![ResolvedWidget {
                manifest: WidgetManifest {
                    id: "empty".to_string(),
                    name: "Empty Widget".to_string(),
                    slot: WidgetSlot::Tab,
                    icon: None,
                    position: None,
                },
                js: String::new(),
                css: None,
            }],
            ..Default::default()
        };
        let result = assemble_index(MINIMAL_HTML, &bundle);
        // Empty JS should still produce a script tag (widget registers itself)
        assert!(result.contains("data-widget=\"empty\""));
    }

    #[test]
    fn test_assemble_index_empty_custom_css_skipped() {
        let bundle = FrontendBundle {
            custom_css: Some("   \n  ".to_string()),
            ..Default::default()
        };
        let result = assemble_index(MINIMAL_HTML, &bundle);
        // Whitespace-only custom CSS should be skipped
        assert!(!result.contains("data-custom-css"));
    }
}
