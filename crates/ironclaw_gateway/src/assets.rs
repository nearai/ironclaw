//! Embedded static assets for the IronClaw web gateway.
//!
//! All frontend files are compiled into the binary via `include_str!()` /
//! `include_bytes!()`. The web gateway serves these as the default baseline;
//! workspace-stored customizations (layout config, widgets, CSS overrides)
//! are layered on top at runtime.

// ==================== Core Files ====================

/// Main HTML page (SPA shell).
pub const INDEX_HTML: &str = include_str!("../static/index.html");

/// Main application JavaScript.
pub const APP_JS: &str = include_str!("../static/app.js");

/// Base stylesheet.
pub const STYLE_CSS: &str = include_str!("../static/style.css");

/// Theme initialization script (runs synchronously in `<head>` to prevent FOUC).
pub const THEME_INIT_JS: &str = include_str!("../static/theme-init.js");

/// Favicon.
pub const FAVICON_ICO: &[u8] = include_bytes!("../static/favicon.ico");

// ==================== Internationalization ====================

/// i18n core library.
pub const I18N_INDEX_JS: &str = include_str!("../static/i18n/index.js");

/// English translations.
pub const I18N_EN_JS: &str = include_str!("../static/i18n/en.js");

/// Chinese (Simplified) translations.
pub const I18N_ZH_CN_JS: &str = include_str!("../static/i18n/zh-CN.js");

/// Korean translations.
pub const I18N_KO_JS: &str = include_str!("../static/i18n/ko.js");

/// i18n integration with the app.
pub const I18N_APP_JS: &str = include_str!("../static/i18n-app.js");

// ==================== Debug Panel ====================

/// Debug panel JavaScript.
pub const DEBUG_PANEL_JS: &str = include_str!("../static/debug-panel.js");

/// Debug panel stylesheet.
pub const DEBUG_PANEL_CSS: &str = include_str!("../static/debug-panel.css");

/// Gateway initialization script.
pub const INIT_JS: &str = include_str!("../static/init.js");

// ==================== Admin Panel ====================

/// Shared theme tokens (CSS custom properties).
pub const THEME_CSS: &str = include_str!("../static/theme.css");

/// Admin panel HTML shell.
pub const ADMIN_HTML: &str = include_str!("../static/admin.html");

/// Admin panel stylesheet.
pub const ADMIN_CSS: &str = include_str!("../static/admin.css");

/// Admin panel JavaScript.
pub const ADMIN_JS: &str = include_str!("../static/admin.js");
