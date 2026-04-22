//! Embedded static assets for the IronClaw web gateway.
//!
//! All frontend files are compiled into the binary via `include_str!()` /
//! `include_bytes!()`. The web gateway serves these as the default baseline;
//! workspace-stored customizations (layout config, widgets, CSS overrides)
//! are layered on top at runtime.

// ==================== Core Files ====================

/// Main HTML page (SPA shell).
pub const INDEX_HTML: &str = include_str!("../static/index.html");

/// Main application JavaScript — compile-time concatenation of the split
/// modules under `static/js/`. Each file owns one surface, one slice of
/// lifecycle, or one shared helper; merge-conflict churn used to
/// dominate `app.js` when it was a single 11 k-line file.
///
/// Execution order MUST match the original top-to-bottom order of the
/// pre-split monolith — code has forward references (closures, event
/// bindings) that are order-dependent. A newline between each module
/// prevents the last token of one file running into the first token of
/// the next when a file ends without a trailing newline.
pub const APP_JS: &str = concat!(
    include_str!("../static/js/core/bootstrap.js"),
    "\n",
    include_str!("../static/js/core/activity-store.js"),
    "\n",
    include_str!("../static/js/core/routing.js"),
    "\n",
    include_str!("../static/js/core/init-auth.js"),
    "\n",
    include_str!("../static/js/core/sse.js"),
    "\n",
    include_str!("../static/js/surfaces/chat.js"),
    "\n",
    include_str!("../static/js/core/render.js"),
    "\n",
    include_str!("../static/js/core/tool-activity.js"),
    "\n",
    include_str!("../static/js/core/onboarding.js"),
    "\n",
    include_str!("../static/js/core/history.js"),
    "\n",
    include_str!("../static/js/surfaces/memory.js"),
    "\n",
    include_str!("../static/js/surfaces/logs.js"),
    "\n",
    include_str!("../static/js/surfaces/extensions.js"),
    "\n",
    include_str!("../static/js/surfaces/jobs.js"),
    "\n",
    include_str!("../static/js/surfaces/routines.js"),
    "\n",
    include_str!("../static/js/surfaces/projects.js"),
    "\n",
    include_str!("../static/js/surfaces/users.js"),
    "\n",
    include_str!("../static/js/core/gateway-tee.js"),
    "\n",
    include_str!("../static/js/surfaces/skills.js"),
    "\n",
    include_str!("../static/js/surfaces/tool-permissions.js"),
    "\n",
    include_str!("../static/js/surfaces/settings.js"),
    "\n",
    include_str!("../static/js/core/ui-helpers.js"),
    "\n",
    include_str!("../static/js/surfaces/config.js"),
    "\n",
    include_str!("../static/js/core/widgets.js"),
);

/// Base stylesheet — compile-time concatenation of the split modules
/// under `static/styles/`. Authoring happens in one module per surface,
/// component, or primitive (see `static/styles/`); the served blob is
/// the same text a monolithic `style.css` would be. Load order is
/// base → layout → components → primitives → surfaces.
///
/// A newline between each module guards against the last rule of one
/// file running into the first selector of the next when a file ends
/// without a trailing newline.
pub const STYLE_CSS: &str = concat!(
    include_str!("../static/styles/base.css"),
    "\n",
    include_str!("../static/styles/layout.css"),
    "\n",
    include_str!("../static/styles/components/topbar.css"),
    "\n",
    include_str!("../static/styles/components/markdown.css"),
    "\n",
    include_str!("../static/styles/components/share-modal.css"),
    "\n",
    include_str!("../static/styles/primitives/toast.css"),
    "\n",
    include_str!("../static/styles/surfaces/auth.css"),
    "\n",
    include_str!("../static/styles/surfaces/chat.css"),
    "\n",
    include_str!("../static/styles/surfaces/memory.css"),
    "\n",
    include_str!("../static/styles/surfaces/jobs.css"),
    "\n",
    include_str!("../static/styles/surfaces/missions.css"),
    "\n",
    include_str!("../static/styles/surfaces/routines.css"),
    "\n",
    include_str!("../static/styles/surfaces/logs.css"),
    "\n",
    include_str!("../static/styles/surfaces/extensions.css"),
    "\n",
    include_str!("../static/styles/surfaces/activity.css"),
    "\n",
    include_str!("../static/styles/surfaces/skills.css"),
    "\n",
    include_str!("../static/styles/surfaces/settings.css"),
    "\n",
    include_str!("../static/styles/surfaces/config.css"),
    "\n",
    include_str!("../static/styles/surfaces/users.css"),
    "\n",
    include_str!("../static/styles/surfaces/tool-permissions.css"),
    "\n",
    include_str!("../static/styles/surfaces/projects.css"),
);

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

/// Debug-mode bootstrap script — runs in `<head>` so `window.isDebugMode`
/// is set before `connectSSE()` builds its URL. Tiny on purpose; keeps
/// theme bootstrap and debug bootstrap as separate concerns.
pub const DEBUG_INIT_JS: &str = include_str!("../static/debug-init.js");

/// Debug panel JavaScript.
pub const DEBUG_PANEL_JS: &str = include_str!("../static/debug-panel.js");

/// Debug panel stylesheet.
pub const DEBUG_PANEL_CSS: &str = include_str!("../static/debug-panel.css");

// ==================== Admin Panel ====================

/// Shared theme tokens (CSS custom properties).
pub const THEME_CSS: &str = include_str!("../static/theme.css");

/// Admin panel HTML shell.
pub const ADMIN_HTML: &str = include_str!("../static/admin.html");

/// Admin panel stylesheet.
pub const ADMIN_CSS: &str = include_str!("../static/admin/admin.css");

/// Admin panel JavaScript.
pub const ADMIN_JS: &str = include_str!("../static/admin/admin.js");
