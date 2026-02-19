pub fn normalize_action(raw_action: &str) -> Option<&'static str> {
    let action = raw_action.trim().to_ascii_lowercase();

    let canonical = match action.as_str() {
        // Navigation
        "open" | "goto" | "navigate" => "open",
        "back" => "back",
        "forward" => "forward",
        "reload" => "reload",

        // Snapshot/refs
        "snapshot" | "dom_snapshot" => "snapshot",

        // Interactions
        "click" => "click",
        "dblclick" | "double_click" => "dblclick",
        "focus" => "focus",
        "fill" => "fill",
        "type" => "type",
        "press" | "press_key" | "key_press" => "press",
        "keydown" | "key_down" => "keydown",
        "keyup" | "key_up" => "keyup",
        "hover" => "hover",
        "check" => "check",
        "uncheck" => "uncheck",
        "select" => "select",
        "scroll" => "scroll",
        "scroll_into_view" | "scrollintoview" => "scroll_into_view",
        "drag" | "drag_drop" => "drag",
        "upload" | "file_upload" => "upload",

        // Wait/sync
        "wait" | "wait_for" => "wait",

        // Retrieval
        "get_text" | "text" => "get_text",
        "get_html" | "html" => "get_html",
        "get_value" | "value" => "get_value",
        "get_attr" | "get_attribute" => "get_attr",
        "get_title" | "title" => "get_title",
        "get_url" | "url" => "get_url",
        "get_count" | "count" => "get_count",
        "get_box" | "box" | "bounding_box" => "get_box",

        // Artifacts
        "screenshot" | "take_screenshot" | "capture_screenshot" => "screenshot",

        // Sessions/state
        "session_create" | "create_session" => "session_create",
        "session_list" | "list_sessions" => "session_list",
        "session_resume" | "resume_session" => "session_resume",
        "session_close" | "close_session" => "session_close",
        "state_save" | "save_state" => "state_save",
        "state_load" | "load_state" => "state_load",

        // Browser state
        "cookies_list" | "list_cookies" | "cookie_list" => "cookies_list",
        "cookies_get" | "get_cookie" | "cookie_get" => "cookies_get",
        "cookies_set" | "set_cookie" | "cookie_set" => "cookies_set",
        "cookies_set_batch" | "set_cookies" | "cookies_batch" => "cookies_set_batch",
        "cookies_delete" | "delete_cookie" | "cookie_delete" => "cookies_delete",
        "local_storage_list" => "local_storage_list",
        "local_storage_get" => "local_storage_get",
        "local_storage_set" => "local_storage_set",
        "local_storage_delete" => "local_storage_delete",
        "session_storage_list" => "session_storage_list",
        "session_storage_get" => "session_storage_get",
        "session_storage_set" => "session_storage_set",
        "session_storage_delete" => "session_storage_delete",

        // Eval
        "eval" | "evaluate" | "js_eval" | "run_script" => "eval",

        _ => return None,
    };

    Some(canonical)
}

pub fn alias_note(raw_action: &str, canonical_action: &str) -> Option<String> {
    let trimmed = raw_action.trim();
    if trimmed.eq_ignore_ascii_case(canonical_action) {
        None
    } else {
        Some(format!(
            "normalized action alias '{}' to canonical '{}'",
            trimmed, canonical_action
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action_alias_normalization_map() {
        assert_eq!(normalize_action("goto"), Some("open"));
        assert_eq!(normalize_action("navigate"), Some("open"));
        assert_eq!(normalize_action("double_click"), Some("dblclick"));
        assert_eq!(normalize_action("save_state"), Some("state_save"));
        assert_eq!(normalize_action("cookie_set"), Some("cookies_set"));
        assert_eq!(normalize_action("set_cookies"), Some("cookies_set_batch"));
        assert_eq!(normalize_action("cookies_batch"), Some("cookies_set_batch"));
        assert_eq!(normalize_action("js_eval"), Some("eval"));
        assert_eq!(normalize_action("get_attribute"), Some("get_attr"));
    }
}
