pub const CONTRACT_VERSION: &str = "2026-02-18";
pub const DEFAULT_BROWSERLESS_URL: &str = "http://127.0.0.1:9222";

// Size limits
pub const MAX_PARAMS_BYTES: usize = 32 * 1024;
pub const MAX_ERROR_MESSAGE_BYTES: usize = 512;
pub const MAX_SELECTOR_BYTES: usize = 2 * 1024;
pub const MAX_SCRIPT_BYTES: usize = 16 * 1024;
pub const MAX_ACTION_TIMEOUT_MS: u32 = 60_000;
pub const MAX_ATTEMPTS: u8 = 3;
pub const MAX_SNAPSHOT_NODES: usize = 1200;
pub const MAX_SNAPSHOT_BYTES: usize = 256 * 1024;
pub const MAX_SNAPSHOT_TEXT_CHARS: usize = 120;

// CDP WebSocket constants
pub const CDP_TIMEOUT_MS: u32 = 30_000;
pub const CDP_WS_PATH: &str = "/";

// Error taxonomy
pub const ERR_INVALID_ACTION: &str = "invalid_action";
pub const ERR_INVALID_PARAMS: &str = "invalid_params";
pub const ERR_INVALID_SELECTOR: &str = "invalid_selector";
pub const ERR_INVALID_REF: &str = "invalid_ref";
pub const ERR_NETWORK_FAILURE: &str = "network_failure";
pub const ERR_TIMEOUT: &str = "timeout";
pub const ERR_RETRY_EXHAUSTED: &str = "retry_exhausted";
pub const ERR_SESSION_NOT_FOUND: &str = "session_not_found";
pub const ERR_SESSION_RESTORE_FAILED: &str = "session_restore_failed";
pub const ERR_POLICY_BLOCKED: &str = "policy_blocked";
pub const ERR_ARTIFACT_NOT_FOUND: &str = "artifact_not_found";
pub const ERR_NOT_IMPLEMENTED: &str = "not_implemented";
pub const ERR_SNAPSHOT_TOO_LARGE: &str = "snapshot_too_large";

// Canonical action surface
pub const CANONICAL_ACTIONS: &[&str] = &[
    // Navigation
    "open",
    "back",
    "forward",
    "reload",
    // Snapshot/refs
    "snapshot",
    // Interactions
    "click",
    "dblclick",
    "focus",
    "fill",
    "type",
    "press",
    "keydown",
    "keyup",
    "hover",
    "check",
    "uncheck",
    "select",
    "scroll",
    "scroll_into_view",
    "drag",
    "upload",
    // Wait/sync
    "wait",
    // Retrieval
    "get_text",
    "get_html",
    "get_value",
    "get_attr",
    "get_title",
    "get_url",
    "get_count",
    "get_box",
    // Artifacts
    "screenshot",
    // Sessions/state
    "session_create",
    "session_list",
    "session_resume",
    "session_close",
    "state_save",
    "state_load",
    // Browser state
    "cookies_list",
    "cookies_get",
    "cookies_set",
    "cookies_set_batch",
    "cookies_delete",
    "local_storage_list",
    "local_storage_get",
    "local_storage_set",
    "local_storage_delete",
    "session_storage_list",
    "session_storage_get",
    "session_storage_set",
    "session_storage_delete",
    // Eval
    "eval",
    // PDF
    "pdf",
];
