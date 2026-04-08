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

/// Rewrite any occurrence of `needle` (ASCII, case-insensitive) in `s` by
/// inserting a backslash between the leading `<` and `/`, turning `</tag`
/// into `<\/tag`. Used to neutralize `</script` / `</style` sequences so
/// embedded content cannot break out of an inline `<script>` or `<style>`
/// block. The HTML parser treats `</script ` and `</SCRIPT>` as end tags
/// just like `</script>`, so a plain literal replace is insufficient.
fn escape_tag_close(s: &str, needle: &str) -> String {
    debug_assert!(needle.starts_with("</") && needle.is_ascii());
    let bytes = s.as_bytes();
    let needle_bytes = needle.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes.len() - i >= needle_bytes.len()
            && bytes[i..i + needle_bytes.len()].eq_ignore_ascii_case(needle_bytes)
        {
            // Preserve original casing, just inject the backslash.
            out.push('<');
            out.push('\\');
            out.push_str(&s[i + 1..i + needle_bytes.len()]);
            i += needle_bytes.len();
        } else {
            // Advance one full UTF-8 char. `is_char_boundary` guarantees we
            // never split a multi-byte sequence.
            let mut next = i + 1;
            while next < bytes.len() && !s.is_char_boundary(next) {
                next += 1;
            }
            out.push_str(&s[i..next]);
            i = next;
        }
    }
    out
}

/// Sentinel inserted into the cached HTML wherever a CSP script nonce should
/// appear. The gateway substitutes this with a fresh per-response nonce
/// before serving so the cached HTML can be reused across requests while the
/// browser still sees a unique nonce on every page load.
///
/// Kept ASCII-only and unlikely to collide with anything an author might
/// reasonably write into widget JS or layout JSON.
pub const NONCE_PLACEHOLDER: &str = "__IRONCLAW_CSP_NONCE__";

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
/// All injected `<script>` tags carry a `nonce` attribute set to
/// [`NONCE_PLACEHOLDER`]. Callers (the gateway's `index_handler`) substitute
/// the placeholder with a fresh per-response nonce and emit a matching
/// `Content-Security-Policy: script-src 'nonce-…'` header. This lets us cache
/// the assembled HTML across requests while still rotating nonces.
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

    // Branding CSS variables. Every other inline injection point in this
    // function runs its content through `escape_tag_close` to neutralize
    // tag-breakout vectors; the branding path used to format directly into
    // `<style>…</style>`, which let a `</style>` sequence inside a color
    // value close the tag early and inject arbitrary HTML. Apply the same
    // escape here so branding stays in lock-step with the other paths.
    let css_vars = bundle.layout.branding.to_css_vars();
    if !css_vars.is_empty() {
        let safe_vars = escape_tag_close(&css_vars, "</style");
        head_injections.push(format!("<style>{}</style>", safe_vars));
    }

    // --- Body injections ---

    // Layout config as global variable. JSON strings can contain `</script>`
    // (serde_json does not escape `<` or `/` by default), so neutralize any
    // `</script…` sequence before dropping it into an inline <script> tag.
    //
    // The `nonce` attribute carries the [`NONCE_PLACEHOLDER`] sentinel — the
    // gateway swaps it for a fresh per-response nonce that matches the
    // response's `Content-Security-Policy` header. Without the nonce the
    // browser blocks this inline script under the gateway's CSP.
    match serde_json::to_string(&bundle.layout) {
        Ok(layout_json) => {
            let safe_layout = escape_tag_close(&layout_json, "</script");
            body_injections.push(format!(
                "<script nonce=\"{NONCE_PLACEHOLDER}\">window.__IRONCLAW_LAYOUT__ = {safe_layout};</script>"
            ));
        }
        Err(e) => {
            // `LayoutConfig` and every nested type derive `Serialize` cleanly,
            // so this branch is unreachable on well-typed input. Surface it
            // anyway — a silent drop here would mean the customized HTML
            // ships without `window.__IRONCLAW_LAYOUT__`, and the IIFE in
            // `app.js` would no-op all branding/tab/chat customizations
            // without leaving a trace. A loud warn at the failure site is
            // cheap insurance against a future refactor that introduces a
            // serialization-fallible field.
            tracing::warn!(
                error = %e,
                "failed to serialize LayoutConfig for window.__IRONCLAW_LAYOUT__ injection — \
                 customizations will not apply"
            );
        }
    }

    // Widget CSS (scoped) and JS
    for widget in &bundle.widgets {
        if let Some(ref css) = widget.css {
            let scoped = scope_css(css, &widget.manifest.id);
            if !scoped.trim().is_empty() {
                // Neutralize any `</style…` sequence in scoped CSS — scope_css
                // only rewrites selectors, so a widget CSS string literal
                // containing `</style>` would otherwise break out of the tag.
                //
                // No nonce needed: the gateway's CSP allows `'unsafe-inline'`
                // for `style-src`. Scripts are the only nonce-gated tags.
                let safe_css = escape_tag_close(&scoped, "</style");
                body_injections.push(format!(
                    "<style data-widget=\"{}\">{}</style>",
                    escape_html_attr(&widget.manifest.id),
                    safe_css
                ));
            }
        }

        // Widget JS inlined (avoids auth issues with <script src> on protected
        // endpoints). Escape `</script>` to prevent tag breakout (XSS) and
        // stamp the CSP nonce placeholder so the gateway can authorize this
        // script under its `script-src 'nonce-…'` policy.
        let safe_js = escape_tag_close(&widget.js, "</script");
        body_injections.push(format!(
            "<script type=\"module\" nonce=\"{}\" data-widget=\"{}\">\n{}\n</script>",
            NONCE_PLACEHOLDER,
            escape_html_attr(&widget.manifest.id),
            safe_js
        ));
    }

    // Custom CSS
    if let Some(ref custom_css) = bundle.custom_css
        && !custom_css.trim().is_empty()
    {
        // Same reasoning as widget CSS — neutralize `</style…` breakouts.
        let safe_custom = escape_tag_close(custom_css, "</style");
        body_injections.push(format!("<style data-custom-css>{}</style>", safe_custom));
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
    fn test_assemble_index_layout_script_carries_nonce_placeholder() {
        // Every injected <script> must carry the nonce placeholder so the
        // gateway can rotate it per response. Without this attribute, the
        // browser blocks the script under the gateway's strict CSP.
        let bundle = FrontendBundle {
            layout: LayoutConfig {
                branding: BrandingConfig {
                    title: Some("Acme".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let result = assemble_index(MINIMAL_HTML, &bundle);
        assert!(
            result.contains(&format!("<script nonce=\"{NONCE_PLACEHOLDER}\">")),
            "layout JSON script must carry nonce placeholder, got: {result}"
        );
    }

    #[test]
    fn test_assemble_index_widget_script_carries_nonce_placeholder() {
        let bundle = FrontendBundle {
            widgets: vec![ResolvedWidget {
                manifest: WidgetManifest {
                    id: "dashboard".to_string(),
                    name: "Dashboard".to_string(),
                    slot: WidgetSlot::Tab,
                    icon: None,
                    position: None,
                },
                js: "console.log('hi');".to_string(),
                css: None,
            }],
            ..Default::default()
        };
        let result = assemble_index(MINIMAL_HTML, &bundle);
        // Widget script must have BOTH the type=module and the nonce attribute.
        assert!(
            result.contains(&format!(
                "<script type=\"module\" nonce=\"{NONCE_PLACEHOLDER}\" data-widget=\"dashboard\">"
            )),
            "widget script tag must carry nonce placeholder, got: {result}"
        );
        // The placeholder must be a substring the caller can substitute. The
        // sentinel string itself must NOT be a valid nonce — it should never
        // accidentally appear elsewhere in well-formed HTML.
        assert!(NONCE_PLACEHOLDER.starts_with("__"));
        assert!(!NONCE_PLACEHOLDER.contains(' '));
    }

    #[test]
    fn test_assemble_index_widget_style_has_no_nonce() {
        // Inline <style> blocks don't need a nonce — the gateway's CSP allows
        // 'unsafe-inline' for style-src. Adding nonce there would be dead
        // weight. This test pins that decision so a future change doesn't
        // accidentally start nonce-gating styles.
        let bundle = FrontendBundle {
            widgets: vec![ResolvedWidget {
                manifest: WidgetManifest {
                    id: "styled".to_string(),
                    name: "Styled".to_string(),
                    slot: WidgetSlot::Tab,
                    icon: None,
                    position: None,
                },
                js: "// noop".to_string(),
                css: Some(".panel { color: red; }".to_string()),
            }],
            ..Default::default()
        };
        let result = assemble_index(MINIMAL_HTML, &bundle);
        // Find the <style data-widget="styled"> tag and assert no nonce attr.
        let style_tag = "<style data-widget=\"styled\">";
        assert!(result.contains(style_tag));
        assert!(
            !result.contains("<style data-widget=\"styled\" nonce="),
            "<style> tags must not carry nonce attributes"
        );
    }

    #[test]
    fn test_assemble_index_layout_json_script_breakout_escaped() {
        // Layout branding title containing `</script>` must not break out
        // of the `window.__IRONCLAW_LAYOUT__` script injection.
        let bundle = FrontendBundle {
            layout: LayoutConfig {
                branding: BrandingConfig {
                    title: Some("evil</script><script>alert(1)</script>".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let result = assemble_index(MINIMAL_HTML, &bundle);
        // The layout <script> tag must not contain a raw `</script>` closer
        // from the title — it must be neutralized as `<\/script>`.
        // Title itself is injected into <title> HTML-escaped (a separate code path),
        // but it's also present inside the JSON string in window.__IRONCLAW_LAYOUT__.
        let layout_start = result.find("window.__IRONCLAW_LAYOUT__").unwrap();
        let layout_end = result[layout_start..].find("</script>").unwrap() + layout_start;
        let layout_script = &result[layout_start..layout_end];
        // Between the opening `<script>` and the first real `</script>`, the
        // raw breakout payload must not appear.
        assert!(
            !layout_script.contains("</script>"),
            "raw </script> inside layout JSON broke out of the script tag"
        );
        assert!(layout_script.contains("<\\/script>"));
    }

    #[test]
    fn test_assemble_index_widget_css_style_breakout_escaped() {
        // Widget CSS containing `</style>` (e.g., via a content: "…" string)
        // must not break out of the <style> tag.
        let bundle = FrontendBundle {
            widgets: vec![ResolvedWidget {
                manifest: WidgetManifest {
                    id: "evil-css".to_string(),
                    name: "Evil CSS Widget".to_string(),
                    slot: WidgetSlot::Tab,
                    icon: None,
                    position: None,
                },
                js: "// safe".to_string(),
                css: Some(
                    ".x::before { content: \"</style><script>alert(1)</script>\"; }".to_string(),
                ),
            }],
            ..Default::default()
        };
        let result = assemble_index(MINIMAL_HTML, &bundle);
        // The widget <style> block must not contain a raw `</style>` from
        // the CSS string literal.
        let style_start = result.find("data-widget=\"evil-css\">").unwrap();
        // `style_start` points into the opening <style> tag's attribute; the
        // first `</style>` after this marker must be the tag's real closer.
        let rest = &result[style_start..];
        let first_close = rest.find("</style>").unwrap();
        let body = &rest[..first_close];
        assert!(
            !body.contains("</style>"),
            "raw </style> inside widget CSS broke out of the style tag"
        );
        assert!(body.contains("<\\/style>"));
    }

    #[test]
    fn test_assemble_index_branding_style_breakout_escaped() {
        // Defense in depth for the branding CSS-vars injection point.
        // The `BrandingConfig` color validator (in `layout.rs`) is the
        // primary defense and strips anything containing `</style>`
        // before it ever reaches `assemble_index`, so the head `<style>`
        // block for branding should never even be emitted in this case.
        //
        // This test locks in BOTH contracts:
        //
        // 1. A hostile color value is dropped before it lands in the
        //    head — no `--color-primary` declaration appears at all.
        // 2. The only place the raw breakout string appears in the final
        //    document is the layout-config `<script>` (which
        //    `escape_tag_close` already handled for the `</script>`
        //    sequence); it must NOT appear inside any head `<style>`
        //    block that would render the injection as HTML.
        //
        // If either the validator or the bundle-level escape regresses,
        // this test fails with a useful diagnostic.
        let bundle = FrontendBundle {
            layout: LayoutConfig {
                branding: BrandingConfig {
                    colors: Some(BrandingColors {
                        primary: Some("red</style><script>alert(1)</script>".to_string()),
                        accent: None,
                    }),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let result = assemble_index(MINIMAL_HTML, &bundle);

        // Contract 1: validator drops the hostile primary value.
        assert!(
            !result.contains("--color-primary"),
            "hostile branding color must be dropped by the validator; got: {result}"
        );

        // Contract 2: no `<style>` tag in the head section contains the
        // raw `</style>` breakout. The head runs from `<head>` up to
        // `</head>`; search that slice for any `<style>` block emitted
        // by branding and verify none contains the raw close-tag.
        if let Some(head_end) = result.find("</head>") {
            let head = &result[..head_end];
            let mut search = head;
            while let Some(style_open) = search.find("<style") {
                let after_open = &search[style_open..];
                let body_start = after_open
                    .find('>')
                    .map(|i| i + 1)
                    .unwrap_or(after_open.len());
                let body_rest = &after_open[body_start..];
                let body_end = body_rest.find("</style>").unwrap_or(body_rest.len());
                let body = &body_rest[..body_end];
                assert!(
                    !body.contains("</style>"),
                    "raw </style> inside head <style> block: {body}"
                );
                // Advance past this block for any subsequent matches.
                search = &body_rest[body_end.min(body_rest.len())..];
            }
        }
    }

    #[test]
    fn test_assemble_index_custom_css_style_breakout_escaped() {
        // Custom workspace CSS containing `</style>` must not break out.
        let bundle = FrontendBundle {
            custom_css: Some("body { color: red; } </style><script>alert(1)</script>".to_string()),
            ..Default::default()
        };
        let result = assemble_index(MINIMAL_HTML, &bundle);
        let style_start = result.find("data-custom-css>").unwrap();
        let rest = &result[style_start..];
        let first_close = rest.find("</style>").unwrap();
        let body = &rest[..first_close];
        assert!(
            !body.contains("</style>"),
            "raw </style> inside custom CSS broke out of the style tag"
        );
        assert!(body.contains("<\\/style>"));
    }

    #[test]
    fn test_escape_tag_close_case_insensitive() {
        // HTML parsers treat `</SCRIPT>` and `</script >` the same as
        // `</script>`, so the escape must be case-insensitive.
        assert_eq!(
            escape_tag_close("a </SCRIPT> b", "</script"),
            "a <\\/SCRIPT> b"
        );
        assert_eq!(
            escape_tag_close("a </Script\n> b", "</script"),
            "a <\\/Script\n> b"
        );
        // Unrelated `<` and `/` characters must be untouched.
        assert_eq!(escape_tag_close("<div>x</div>", "</script"), "<div>x</div>");
    }

    #[test]
    fn test_escape_tag_close_multibyte_safe() {
        // Must not panic on multi-byte UTF-8 characters adjacent to the needle.
        let input = "日本語</script>日本語";
        let out = escape_tag_close(input, "</script");
        assert!(out.contains("<\\/script>"));
        assert!(out.contains("日本語"));
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
