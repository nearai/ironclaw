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

// ==================== Admin Panel ====================

/// Shared theme tokens (CSS custom properties).
pub const THEME_CSS: &str = include_str!("../static/theme.css");

/// Admin panel HTML shell.
pub const ADMIN_HTML: &str = include_str!("../static/admin.html");

/// Admin panel stylesheet.
pub const ADMIN_CSS: &str = include_str!("../static/admin.css");

/// Admin panel JavaScript.
pub const ADMIN_JS: &str = include_str!("../static/admin.js");

// ==================== Application Modules ====================
//
// The monolithic app.js has been split into focused modules. Each is
// embedded and served from `/modules/{name}.js`.

pub const MODULE_STATE_JS: &str = include_str!("../static/modules/state.js");
pub const MODULE_UI_UTILS_JS: &str = include_str!("../static/modules/ui-utils.js");
pub const MODULE_RENDERING_JS: &str = include_str!("../static/modules/rendering.js");
pub const MODULE_THEME_JS: &str = include_str!("../static/modules/theme.js");
pub const MODULE_REASONING_JS: &str = include_str!("../static/modules/reasoning.js");
pub const MODULE_HASH_NAV_JS: &str = include_str!("../static/modules/hash-nav.js");
pub const MODULE_API_JS: &str = include_str!("../static/modules/api.js");
pub const MODULE_AUTH_JS: &str = include_str!("../static/modules/auth.js");
pub const MODULE_RESTART_JS: &str = include_str!("../static/modules/restart.js");
pub const MODULE_TOOL_ACTIVITY_JS: &str = include_str!("../static/modules/tool-activity.js");
pub const MODULE_APPROVAL_JS: &str = include_str!("../static/modules/approval.js");
pub const MODULE_IMAGES_JS: &str = include_str!("../static/modules/images.js");
pub const MODULE_SLASH_JS: &str = include_str!("../static/modules/slash.js");
pub const MODULE_CHAT_JS: &str = include_str!("../static/modules/chat.js");
pub const MODULE_GATES_JS: &str = include_str!("../static/modules/gates.js");
pub const MODULE_THREADS_JS: &str = include_str!("../static/modules/threads.js");
pub const MODULE_TABS_JS: &str = include_str!("../static/modules/tabs.js");
pub const MODULE_SSE_JS: &str = include_str!("../static/modules/sse.js");
pub const MODULE_MEMORY_JS: &str = include_str!("../static/modules/memory.js");
pub const MODULE_LOGS_JS: &str = include_str!("../static/modules/logs.js");
pub const MODULE_EXTENSIONS_JS: &str = include_str!("../static/modules/extensions.js");
pub const MODULE_PAIRING_JS: &str = include_str!("../static/modules/pairing.js");
pub const MODULE_JOBS_JS: &str = include_str!("../static/modules/jobs.js");
pub const MODULE_ROUTINES_JS: &str = include_str!("../static/modules/routines.js");
pub const MODULE_PROJECTS_JS: &str = include_str!("../static/modules/projects.js");
pub const MODULE_USERS_JS: &str = include_str!("../static/modules/users.js");
pub const MODULE_GATEWAY_STATUS_JS: &str = include_str!("../static/modules/gateway-status.js");
pub const MODULE_TEE_JS: &str = include_str!("../static/modules/tee.js");
pub const MODULE_SKILLS_JS: &str = include_str!("../static/modules/skills.js");
pub const MODULE_TOOLS_PERMISSIONS_JS: &str =
    include_str!("../static/modules/tools-permissions.js");
pub const MODULE_KEYBOARD_JS: &str = include_str!("../static/modules/keyboard.js");
pub const MODULE_SETTINGS_JS: &str = include_str!("../static/modules/settings.js");
pub const MODULE_CONFIG_JS: &str = include_str!("../static/modules/config.js");
pub const MODULE_INIT_JS: &str = include_str!("../static/modules/init.js");
