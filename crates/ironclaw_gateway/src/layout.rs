//! Layout configuration types for frontend customization.
//!
//! A [`LayoutConfig`] is stored as `.system/gateway/layout.json` in the
//! workspace. It controls branding, tab visibility/order, chat features, and
//! per-widget configuration. All fields are optional with sensible defaults.

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

    /// Opt in to converting inline JSON-shaped fragments in assistant
    /// messages into styled data cards (`upgradeInlineJson` in `app.js`).
    ///
    /// Disabled by default because the heuristic pattern-matches any
    /// balanced `{...}` in rendered markdown — prose containing JSON-like
    /// text (`"yes, set the value to {x: 1, y: 2}"`) gets false-positive
    /// rewritten into a card. Operators that drive structured data
    /// through chat (e.g., a workflow that emits real JSON in every
    /// reply) can flip this on; everyone else gets prose left alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upgrade_inline_json: Option<bool>,
}

/// Per-widget instance configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WidgetInstanceConfig {
    /// Whether this widget is enabled. Defaults to `true` so a layout entry
    /// that only customizes a widget's `config` (and omits `enabled`) does
    /// not silently disable the widget.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Arbitrary widget-specific configuration passed to `widget.init()`.
    #[serde(default)]
    pub config: serde_json::Value,
}

impl Default for WidgetInstanceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            config: serde_json::Value::Null,
        }
    }
}

fn default_true() -> bool {
    true
}

impl BrandingConfig {
    /// Generate CSS custom property overrides for injection into `:root`.
    ///
    /// Color values are run through [`is_safe_css_color`] before
    /// interpolation so a hostile `layout.json` cannot break out of the
    /// `:root {}` block (e.g.
    /// `red; } .chat-input[value^="s"] { background: url(...) }`) or close
    /// the surrounding `<style>` tag. Invalid values are silently dropped
    /// so the rest of the branding config still applies.
    pub fn to_css_vars(&self) -> String {
        let mut vars = Vec::new();
        if let Some(ref colors) = self.colors {
            if let Some(ref primary) = colors.primary
                && is_safe_css_color(primary)
            {
                vars.push(format!("--color-primary: {};", primary));
            }
            if let Some(ref accent) = colors.accent
                && is_safe_css_color(accent)
            {
                vars.push(format!("--color-accent: {};", accent));
            }
        }
        if vars.is_empty() {
            String::new()
        } else {
            format!(":root {{ {} }}", vars.join(" "))
        }
    }

    /// Return [`Self::logo_url`] if it passes [`is_safe_url`], otherwise
    /// `None`.
    ///
    /// `logo_url` is currently a passthrough field — no consumer in the
    /// browser runtime reads it yet — but it is exposed via
    /// `GET /api/frontend/layout` and lands inside the
    /// `window.__IRONCLAW_LAYOUT__` JSON island. The first consumer that
    /// renders it (most likely as `<img src="…">` or
    /// `<link rel="icon" href="…">`) would inherit a footgun if a
    /// `layout.json` could ship `javascript:`/`data:` URIs unfiltered.
    /// Routing every consumer through this getter — mirroring the
    /// `to_css_vars` precedent for branding colors — keeps the validation
    /// at the type layer so a future caller can't accidentally bypass it
    /// by reading the field directly.
    pub fn safe_logo_url(&self) -> Option<&str> {
        self.logo_url.as_deref().filter(|v| is_safe_url(v))
    }

    /// Return [`Self::favicon_url`] if it passes [`is_safe_url`], otherwise
    /// `None`. Same rationale as [`Self::safe_logo_url`].
    pub fn safe_favicon_url(&self) -> Option<&str> {
        self.favicon_url.as_deref().filter(|v| is_safe_url(v))
    }
}

/// Return `true` if `value` is a syntactically-safe URL for use in `<img
/// src>` / `<link href>` / similar HTML attribute contexts.
///
/// Accepts a conservative subset:
///
/// * **HTTPS / HTTP absolute URLs**: `https://example.com/logo.png`,
///   `http://intranet.local/icon.svg`. HTTP is allowed (not just HTTPS) so
///   intranet and dev deployments aren't gratuitously broken; the gateway
///   itself enforces TLS at the network layer where appropriate.
/// * **Site-relative paths**: `/static/logo.png`, `/foo/bar.svg`. Must
///   start with a single `/`, NOT `//` (protocol-relative — those can be
///   hijacked into a different scheme by the browser's URL parser).
///
/// Rejects:
///
/// * `javascript:`, `data:`, `vbscript:`, `file:`, `blob:`, and any other
///   non-HTTP(S) absolute scheme. These are the classic
///   `<img src="javascript:…">` / data-URI tracking-pixel vectors.
/// * Strings containing characters that could break out of an HTML
///   attribute (`<`, `>`, `"`, `'`, backtick, backslash) or terminate it
///   prematurely (NUL, newline, carriage return, tab).
/// * Empty / whitespace-only values, and anything > 2048 bytes (matches
///   the de-facto Chrome / Apache URL length cap; longer values are
///   either pathological or an exfil vector).
///
/// Strict URL parsing is **not** the goal — this validator favors
/// rejecting suspicious shapes over preserving every legal RFC 3986 form.
/// A user who needs an exotic URL can pre-encode it; the gateway's job is
/// to refuse anything an attacker could weaponize against an unsuspecting
/// future consumer.
pub(crate) fn is_safe_url(value: &str) -> bool {
    let v = value.trim();
    if v.is_empty() || v.len() > 2048 {
        return false;
    }
    // Reject HTML-attribute breakout vectors and any control character
    // that could be smuggled through copy-paste from a hostile source.
    if v.bytes().any(|b| {
        matches!(
            b,
            b'<' | b'>' | b'"' | b'\'' | b'`' | b'\\' | b'\0' | b'\n' | b'\r' | b'\t'
        )
    }) {
        return false;
    }
    // Site-relative path. Must start with a single `/`, NOT `//`
    // (protocol-relative URLs are scheme-flippable in the browser URL
    // parser and historically have been a source of CSP-bypass tricks).
    if let Some(rest) = v.strip_prefix('/') {
        return !rest.starts_with('/');
    }
    // Otherwise must be an absolute http(s) URL. Lowercase the scheme
    // prefix for the comparison only — the rest of the URL is left as-is
    // because path/query case can be semantically meaningful.
    let lower = v.to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

/// Return `true` if `value` is a syntactically-safe CSS color literal.
///
/// Accepts a conservative subset of CSS color syntax:
///
/// * Hex literals: `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`.
/// * Functional notation: `rgb(...)`, `rgba(...)`, `hsl(...)`, `hsla(...)`,
///   `hwb(...)`, `lab(...)`, `lch(...)`, `oklab(...)`, `oklch(...)`,
///   `color(...)`.
/// * CSS named colors (alphabetic identifiers only).
///
/// Anything containing characters that could break out of a CSS property
/// value (`;`, `{`, `}`, `<`, `>`, backslash, newline, quotes) or the
/// `url(` prefix is rejected regardless of surface syntax. The primary
/// goal is to keep attacker-controlled values from escaping the
/// `:root { … }` block or the enclosing `<style>` tag — strict CSS Color
/// Module conformance is *not* a goal, so this will reject some valid but
/// unusual inputs (e.g. `color-mix(...)`) by design.
pub(crate) fn is_safe_css_color(value: &str) -> bool {
    let v = value.trim();
    if v.is_empty() || v.len() > 128 {
        return false;
    }
    // Reject any character that could terminate the declaration, close the
    // surrounding block, break out of the `<style>` tag, or start a CSS
    // comment (`*` handles both `/*` and `*/` because both require the
    // asterisk; the bare `/` used in `rgb(0 0 0 / 50%)` stays legal).
    if v.bytes().any(|b| {
        matches!(
            b,
            b';' | b'{' | b'}' | b'<' | b'>' | b'"' | b'\'' | b'\\' | b'*' | b'\n' | b'\r' | b'\t'
        )
    }) {
        return false;
    }
    let lower = v.to_ascii_lowercase();
    // `url(...)` references — even inside function args — can point at
    // arbitrary origins and leak request metadata. Never allow them in a
    // branding color value.
    if lower.contains("url(") {
        return false;
    }
    // Hex literal: `#` followed by 3/4/6/8 hex digits.
    if let Some(hex) = v.strip_prefix('#') {
        let len_ok = matches!(hex.len(), 3 | 4 | 6 | 8);
        let chars_ok = hex.chars().all(|c| c.is_ascii_hexdigit());
        return len_ok && chars_ok;
    }
    // Functional notation: `ident(...)` where `ident` is a recognized
    // color function and the body contains only digits, letters, spaces,
    // commas, dots, percent signs, and parentheses. The outer parens must
    // balance and the value must end with `)`.
    if let Some(open) = v.find('(') {
        let ident = &lower[..open];
        let func_ok = matches!(
            ident,
            "rgb" | "rgba" | "hsl" | "hsla" | "hwb" | "lab" | "lch" | "oklab" | "oklch" | "color"
        );
        if !func_ok {
            return false;
        }
        if !v.ends_with(')') {
            return false;
        }
        let body = &v[open + 1..v.len() - 1];
        let body_ok = body.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, ' ' | ',' | '.' | '%' | '+' | '-' | '/')
        });
        // `/` is allowed inside functional color syntax (e.g.
        // `rgb(0 0 0 / 50%)`), but we already rejected the outer `url(`
        // form above and the enclosing parens are required — a bare `/`
        // inside the function body cannot escape the declaration.
        return body_ok;
    }
    // Named color or CSS-wide keyword: alphabetic identifier only.
    v.chars().all(|c| c.is_ascii_alphabetic())
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
    fn test_widget_instance_enabled_defaults_to_true() {
        // A layout entry that customizes config but omits `enabled` must
        // NOT silently disable the widget — that was the old bug.
        let json = serde_json::json!({ "config": { "refresh": 30 } });
        let cfg: WidgetInstanceConfig = serde_json::from_value(json).unwrap();
        assert!(cfg.enabled, "enabled should default to true");
    }

    #[test]
    fn test_widget_instance_default_impl_is_enabled() {
        // The programmatic default must match the deserialized default.
        assert!(WidgetInstanceConfig::default().enabled);
    }

    #[test]
    fn test_widget_instance_explicit_false_respected() {
        let json = serde_json::json!({ "enabled": false });
        let cfg: WidgetInstanceConfig = serde_json::from_value(json).unwrap();
        assert!(!cfg.enabled);
    }

    #[test]
    fn test_partial_deserialization() {
        let json = serde_json::json!({"branding": {"title": "Test"}});
        let config: LayoutConfig = serde_json::from_value(json).unwrap();
        assert_eq!(config.branding.title.as_deref(), Some("Test"));
        assert!(config.chat.suggestions.is_none());
    }

    #[test]
    fn test_is_safe_css_color_accepts_common_forms() {
        // Hex literals of every supported length.
        assert!(is_safe_css_color("#fff"));
        assert!(is_safe_css_color("#fff0"));
        assert!(is_safe_css_color("#0066cc"));
        assert!(is_safe_css_color("#0066ccaa"));
        // Functional notation, including modern `rgb(... / alpha)` syntax.
        assert!(is_safe_css_color("rgb(0, 0, 0)"));
        assert!(is_safe_css_color("rgba(10, 20, 30, 0.5)"));
        assert!(is_safe_css_color("rgb(0 0 0 / 50%)"));
        assert!(is_safe_css_color("hsl(200, 50%, 50%)"));
        assert!(is_safe_css_color("oklch(0.7 0.15 200)"));
        // Named colors / keywords.
        assert!(is_safe_css_color("red"));
        assert!(is_safe_css_color("transparent"));
        // Leading/trailing whitespace is tolerated.
        assert!(is_safe_css_color("  #fff  "));
    }

    #[test]
    fn test_is_safe_css_color_rejects_injection_vectors() {
        // Declaration termination would let an attacker add a new
        // declaration or close the `:root {}` block.
        assert!(!is_safe_css_color("red;"));
        assert!(!is_safe_css_color("red; } .chat-input { background: red }"));
        // `<style>` tag breakout.
        assert!(!is_safe_css_color("red</style><script>alert(1)</script>"));
        assert!(!is_safe_css_color("#fff</STYLE>"));
        // `url(...)` can pull from arbitrary origins.
        assert!(!is_safe_css_color("url(https://attacker.example/leak)"));
        assert!(!is_safe_css_color("rgb(url(x), 0, 0)"));
        // CSS comments could hide payload from casual readers.
        assert!(!is_safe_css_color("red /* ok */ "));
        assert!(!is_safe_css_color("red*/"));
        // Quotes / backslash / newline are never legal in a color value.
        assert!(!is_safe_css_color("\"#fff\""));
        assert!(!is_safe_css_color("#fff\\"));
        assert!(!is_safe_css_color("#fff\nbad"));
        // Unknown functions are rejected even with balanced parens.
        assert!(!is_safe_css_color("expression(1)"));
        // Empty or absurdly long values are rejected.
        assert!(!is_safe_css_color(""));
        assert!(!is_safe_css_color("   "));
        assert!(!is_safe_css_color(&"#".repeat(200)));
    }

    #[test]
    fn test_is_safe_url_accepts_common_forms() {
        // Absolute HTTPS — the default operator path.
        assert!(is_safe_url("https://example.com/logo.png"));
        assert!(is_safe_url("https://cdn.example.com/path/to/icon.svg?v=2"));
        // Absolute HTTP — intranet/dev usability. The gateway enforces
        // TLS at the network layer where appropriate, so blocking plain
        // HTTP here would be punishing for development setups.
        assert!(is_safe_url("http://intranet.local/x.png"));
        // Site-relative — for assets served by the gateway itself.
        assert!(is_safe_url("/static/logo.png"));
        assert!(is_safe_url("/foo/bar/baz.svg"));
        // Tolerated whitespace from sloppy hand-edits.
        assert!(is_safe_url("  https://example.com/logo.png  "));
    }

    #[test]
    fn test_is_safe_url_rejects_injection_vectors() {
        // The classic `<img src=javascript:>` vector. Case-insensitive
        // check covers `JavaScript:`, `JAVASCRIPT:`, etc.
        assert!(!is_safe_url("javascript:alert(1)"));
        assert!(!is_safe_url("JavaScript:alert(1)"));
        assert!(!is_safe_url("JAVASCRIPT:alert(1)"));
        // `data:` is the tracking-pixel / payload-stash vector.
        assert!(!is_safe_url("data:text/html,<script>alert(1)</script>"));
        assert!(!is_safe_url("data:image/svg+xml;base64,PHN2Zy8+"));
        // Other historically-abused schemes.
        assert!(!is_safe_url("vbscript:msgbox(1)"));
        assert!(!is_safe_url("file:///etc/passwd"));
        assert!(!is_safe_url("blob:https://attacker.example/x"));
        // Protocol-relative URLs are scheme-flippable in the browser
        // URL parser and have historically been a CSP-bypass source.
        assert!(!is_safe_url("//attacker.example/logo.png"));
        // HTML-attribute breakout vectors.
        assert!(!is_safe_url(
            "https://x.example/\"><script>alert(1)</script>"
        ));
        assert!(!is_safe_url("https://x.example/<img>"));
        assert!(!is_safe_url("https://x.example/'onerror='alert(1)"));
        assert!(!is_safe_url("https://x.example/`backtick`"));
        // Control characters that could be smuggled through copy-paste.
        assert!(!is_safe_url("https://x.example/\nhost"));
        assert!(!is_safe_url("https://x.example/\rhost"));
        assert!(!is_safe_url("https://x.example/\tpath"));
        assert!(!is_safe_url("https://x.example/\0null"));
        // Empty / whitespace-only.
        assert!(!is_safe_url(""));
        assert!(!is_safe_url("   "));
        // Length cap. 2049 is one byte over the 2048-byte limit.
        // `"https://example.com/"` is 20 chars, so 2029 trailing chars
        // brings the total to 2049 — one over.
        let too_long = format!("https://example.com/{}", "a".repeat(2029));
        assert_eq!(too_long.len(), 2049);
        assert!(!is_safe_url(&too_long));
        // And exactly 2048 must still pass.
        let at_limit = format!("https://example.com/{}", "a".repeat(2028));
        assert_eq!(at_limit.len(), 2048);
        assert!(is_safe_url(&at_limit));
        // No scheme at all (not relative either).
        assert!(!is_safe_url("example.com/logo.png"));
        // Single `/` is technically a valid root path, but the
        // contract is "site-relative path"; bare `/` is fine.
        assert!(is_safe_url("/"));
    }

    #[test]
    fn test_branding_safe_logo_url_filters_invalid() {
        // safe_logo_url is the contract any future consumer must use.
        // It must return None when the underlying field is missing,
        // empty, whitespace-only, or a hostile scheme — and the original
        // string when it's a legal HTTPS / HTTP / site-relative URL.
        let safe = BrandingConfig {
            logo_url: Some("https://example.com/logo.png".to_string()),
            ..Default::default()
        };
        assert_eq!(safe.safe_logo_url(), Some("https://example.com/logo.png"));

        let hostile = BrandingConfig {
            logo_url: Some("javascript:alert(1)".to_string()),
            ..Default::default()
        };
        assert!(
            hostile.safe_logo_url().is_none(),
            "javascript: scheme must be dropped by safe_logo_url"
        );

        let relative = BrandingConfig {
            logo_url: Some("/static/logo.png".to_string()),
            ..Default::default()
        };
        assert_eq!(relative.safe_logo_url(), Some("/static/logo.png"));

        let absent = BrandingConfig {
            logo_url: None,
            ..Default::default()
        };
        assert!(absent.safe_logo_url().is_none());
    }

    #[test]
    fn test_branding_safe_favicon_url_filters_invalid() {
        // Same contract as safe_logo_url; covers the parallel field so a
        // future consumer can never accidentally route favicon through a
        // bypass while logo is correctly validated.
        let safe = BrandingConfig {
            favicon_url: Some("/favicon.ico".to_string()),
            ..Default::default()
        };
        assert_eq!(safe.safe_favicon_url(), Some("/favicon.ico"));

        let hostile = BrandingConfig {
            favicon_url: Some("data:image/x-icon;base64,AA==".to_string()),
            ..Default::default()
        };
        assert!(
            hostile.safe_favicon_url().is_none(),
            "data: scheme must be dropped by safe_favicon_url"
        );
    }

    #[test]
    fn test_chat_upgrade_inline_json_defaults_to_none() {
        // The opt-in flag must default to `None` (== not set, treated as
        // off) so an existing layout.json without the field doesn't
        // suddenly start rewriting prose into JSON cards after upgrade.
        let cfg: ChatConfig = serde_json::from_str("{}").unwrap();
        assert!(cfg.upgrade_inline_json.is_none());
    }

    #[test]
    fn test_chat_upgrade_inline_json_roundtrips_explicit_true() {
        let cfg: ChatConfig = serde_json::from_str(r#"{"upgrade_inline_json": true}"#).unwrap();
        assert_eq!(cfg.upgrade_inline_json, Some(true));
        // Round-trip through serialize so the JS-visible JSON shape is
        // pinned: explicit `true` survives, default `None` is omitted.
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(json.contains("\"upgrade_inline_json\":true"));
        let omitted: ChatConfig = ChatConfig::default();
        let json = serde_json::to_string(&omitted).unwrap();
        assert!(!json.contains("upgrade_inline_json"));
    }

    #[test]
    fn test_branding_css_vars_drops_unsafe_colors() {
        // A hostile `layout.json` must not be able to slip a `;`-terminated
        // or tag-breakout color past `to_css_vars`. Both fields are set;
        // only the safe one should appear.
        let branding = BrandingConfig {
            colors: Some(BrandingColors {
                primary: Some("red; } .chat-input { background: red".to_string()),
                accent: Some("#ff6b00".to_string()),
            }),
            ..Default::default()
        };
        let css = branding.to_css_vars();
        assert!(
            !css.contains("chat-input"),
            "primary injection leaked: {css}"
        );
        assert!(!css.contains("--color-primary"), "primary must be dropped");
        assert!(css.contains("--color-accent: #ff6b00;"));
    }
}
