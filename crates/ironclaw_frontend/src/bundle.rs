//! Frontend bundle assembly.
//!
//! Combines the embedded base HTML with workspace customizations (layout
//! config, widgets, CSS overrides) into the final served page.

use crate::layout::LayoutConfig;
use crate::widget::{WidgetManifest, scope_css};

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
                    widget.manifest.id, scoped
                ));
            }
        }

        // Widget JS as module script served from API
        body_injections.push(format!(
            "<script type=\"module\" src=\"/api/frontend/widget/{}/index.js\"></script>",
            widget.manifest.id
        ));
    }

    // Custom CSS
    if let Some(ref custom_css) = bundle.custom_css {
        if !custom_css.trim().is_empty() {
            body_injections.push(format!(
                "<style data-custom-css>{}</style>",
                custom_css
            ));
        }
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

    // Override <title> if branding title is set
    if let Some(ref title) = bundle.layout.branding.title {
        if let Some(start) = result.find("<title>") {
            if let Some(end) = result[start..].find("</title>") {
                let end = start + end + "</title>".len();
                result.replace_range(start..end, &format!("<title>{}</title>", title));
            }
        }
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
        assert!(result.contains("src=\"/api/frontend/widget/dashboard/index.js\""));
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
}
