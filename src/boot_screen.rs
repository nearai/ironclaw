//! Boot screen displayed after all initialization completes.
//!
//! Shows a polished ANSI-styled status panel summarizing the agent's runtime
//! state: model, database, tool count, enabled features, active channels,
//! and the gateway URL.

/// All displayable fields for the boot screen.
pub struct BootInfo {
    pub version: String,
    pub agent_name: String,
    pub llm_backend: String,
    pub llm_model: String,
    pub cheap_model: Option<String>,
    pub db_backend: String,
    pub db_connected: bool,
    pub tool_count: usize,
    pub gateway_url: Option<String>,
    pub embeddings_enabled: bool,
    pub embeddings_provider: Option<String>,
    pub heartbeat_enabled: bool,
    pub heartbeat_interval_secs: u64,
    pub sandbox_enabled: bool,
    pub claude_code_enabled: bool,
    pub routines_enabled: bool,
    pub channels: Vec<String>,
}

/// Print the boot screen to stdout.
pub fn print_boot_screen(info: &BootInfo) {
    // ANSI codes matching existing REPL palette
    let bold = "\x1b[1m";
    let cyan = "\x1b[36m";
    let dim = "\x1b[90m";
    let yellow_underline = "\x1b[33;4m";
    let reset = "\x1b[0m";

    let border = format!("  {dim}{}{reset}", "\u{2576}".repeat(58));

    println!();
    println!("{border}");
    println!();
    println!("  {bold}{}{reset} v{}", info.agent_name, info.version);
    println!();

    // Model line
    let model_display = if let Some(ref cheap) = info.cheap_model {
        format!(
            "{cyan}{}{reset}  {dim}cheap{reset} {cyan}{}{reset}",
            info.llm_model, cheap
        )
    } else {
        format!("{cyan}{}{reset}", info.llm_model)
    };
    println!(
        "  {dim}model{reset}     {model_display}  {dim}via {}{reset}",
        info.llm_backend
    );

    // Database line
    let db_status = if info.db_connected {
        "connected"
    } else {
        "none"
    };
    println!(
        "  {dim}database{reset}  {cyan}{}{reset} {dim}({db_status}){reset}",
        info.db_backend
    );

    // Tools line
    println!(
        "  {dim}tools{reset}     {cyan}{}{reset} {dim}registered{reset}",
        info.tool_count
    );

    // Features line
    let mut features = Vec::new();
    if info.embeddings_enabled {
        if let Some(ref provider) = info.embeddings_provider {
            features.push(format!("embeddings ({provider})"));
        } else {
            features.push("embeddings".to_string());
        }
    }
    if info.heartbeat_enabled {
        let mins = info.heartbeat_interval_secs / 60;
        features.push(format!("heartbeat ({mins}m)"));
    }
    if info.sandbox_enabled {
        features.push("sandbox".to_string());
    }
    if info.claude_code_enabled {
        features.push("claude-code".to_string());
    }
    if info.routines_enabled {
        features.push("routines".to_string());
    }
    if !features.is_empty() {
        println!(
            "  {dim}features{reset}  {cyan}{}{reset}",
            features.join("  ")
        );
    }

    // Channels line
    if !info.channels.is_empty() {
        println!(
            "  {dim}channels{reset}  {cyan}{}{reset}",
            info.channels.join("  ")
        );
    }

    // Gateway URL (highlighted)
    if let Some(ref url) = info.gateway_url {
        println!();
        println!("  {dim}gateway{reset}   {yellow_underline}{url}{reset}");
    }

    println!();
    println!("{border}");
    println!();
    println!("  /help for commands, /quit to exit");
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_print_boot_screen_full() {
        let info = BootInfo {
            version: "0.2.0".to_string(),
            agent_name: "ironclaw".to_string(),
            llm_backend: "nearai".to_string(),
            llm_model: "claude-3-5-sonnet-20241022".to_string(),
            cheap_model: Some("gpt-4o-mini".to_string()),
            db_backend: "libsql".to_string(),
            db_connected: true,
            tool_count: 24,
            gateway_url: Some("http://127.0.0.1:3001/?token=abc123".to_string()),
            embeddings_enabled: true,
            embeddings_provider: Some("openai".to_string()),
            heartbeat_enabled: true,
            heartbeat_interval_secs: 1800,
            sandbox_enabled: true,
            claude_code_enabled: false,
            routines_enabled: true,
            channels: vec![
                "repl".to_string(),
                "gateway".to_string(),
                "telegram".to_string(),
            ],
        };
        // Should not panic
        print_boot_screen(&info);
    }

    #[test]
    fn test_print_boot_screen_minimal() {
        let info = BootInfo {
            version: "0.2.0".to_string(),
            agent_name: "ironclaw".to_string(),
            llm_backend: "nearai".to_string(),
            llm_model: "gpt-4o".to_string(),
            cheap_model: None,
            db_backend: "none".to_string(),
            db_connected: false,
            tool_count: 5,
            gateway_url: None,
            embeddings_enabled: false,
            embeddings_provider: None,
            heartbeat_enabled: false,
            heartbeat_interval_secs: 0,
            sandbox_enabled: false,
            claude_code_enabled: false,
            routines_enabled: false,
            channels: vec![],
        };
        // Should not panic
        print_boot_screen(&info);
    }

    #[test]
    fn test_print_boot_screen_no_features() {
        let info = BootInfo {
            version: "0.1.0".to_string(),
            agent_name: "test".to_string(),
            llm_backend: "openai".to_string(),
            llm_model: "gpt-4o".to_string(),
            cheap_model: None,
            db_backend: "postgres".to_string(),
            db_connected: true,
            tool_count: 10,
            gateway_url: None,
            embeddings_enabled: false,
            embeddings_provider: None,
            heartbeat_enabled: false,
            heartbeat_interval_secs: 0,
            sandbox_enabled: false,
            claude_code_enabled: false,
            routines_enabled: false,
            channels: vec!["repl".to_string()],
        };
        // Should not panic
        print_boot_screen(&info);
    }
}
