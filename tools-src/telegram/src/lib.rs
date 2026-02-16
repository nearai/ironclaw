//! Telegram User-Mode WASM Tool for IronClaw.
//!
//! Provides Telegram integration operating from the **user's personal account**,
//! not a bot. This tool sends encrypted MTProto messages directly to Telegram's
//! data centers via HTTPS POST, using the grammers crate for the Sans-IO
//! protocol implementation.
//!
//! # Architecture
//!
//! ```text
//! WASM Tool ──MTProto/HTTPS──► Telegram DC (*.web.telegram.org/apiw)
//! ```
//!
//! No Docker container, no middleware. The tool performs the DH key exchange,
//! encrypts requests with the auth key, and POSTs raw ciphertext to Telegram's
//! web transport endpoint.
//!
//! # Session Persistence
//!
//! Session state (auth key, salt, DC, login status) is stored in the workspace
//! at `telegram/session.json`. The agent should save updated session data after
//! auth actions using `memory_write`.
//!
//! # Prerequisites
//!
//! 1. Get Telegram API credentials from https://my.telegram.org/apps
//! 2. Store them: `ironclaw secret set telegram_api_id <id>`
//!    `ironclaw secret set telegram_api_hash <hash>`
//! 3. Use the `login` action with your phone number
//!
//! # Authentication Flow
//!
//! 1. Call `login` with your phone number
//!    - Generates an auth key (DH exchange with Telegram DC)
//!    - Sends verification code to your phone
//!    - Returns session data and phone_code_hash
//! 2. Call `submit_auth_code` with the verification code
//! 3. Call `submit_2fa_password` if you have 2FA enabled
//! 4. After each auth step, save the returned `session` JSON to
//!    `telegram/session.json` via `memory_write`
//!
//! # Privacy
//!
//! - `get_messages` does NOT mark messages as read
//! - Messages are read via `messages.getHistory`, not `getUpdates`

mod api;
mod auth;
mod session;
mod transport;
mod types;

use session::Session;
use types::TelegramAction;

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

struct TelegramTool;

impl exports::near::agent::tool::Guest for TelegramTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match execute_inner(&req.params) {
            Ok(result) => exports::near::agent::tool::Response {
                output: Some(result),
                error: None,
            },
            Err(e) => exports::near::agent::tool::Response {
                output: None,
                error: Some(e),
            },
        }
    }

    fn schema() -> String {
        SCHEMA.to_string()
    }

    fn description() -> String {
        "Telegram user-mode integration for reading and sending messages from the user's \
         personal account. Supports contacts, chat history, message search, sending, \
         forwarding, and deletion. Communicates directly with Telegram's servers via \
         encrypted MTProto over HTTPS (no Docker/TDLight needed). Does NOT mark messages \
         as read when reading history. Use the 'login' action to authenticate with your \
         phone number. Session state is persisted in the workspace at telegram/session.json."
            .to_string()
    }
}

fn execute_inner(params: &str) -> Result<String, String> {
    let action: TelegramAction =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;

    near::agent::host::log(
        near::agent::host::LogLevel::Info,
        &format!("Executing Telegram action: {action:?}"),
    );

    match action {
        TelegramAction::Login { phone_number } => execute_login(&phone_number),
        TelegramAction::SubmitAuthCode { code } => execute_submit_code(&code),
        TelegramAction::Submit2faPassword { password } => execute_submit_2fa(&password),
        TelegramAction::GetMe => with_session(api::get_me),
        TelegramAction::GetContacts => with_session(api::get_contacts),
        TelegramAction::GetChats { limit } => with_session(|s| api::get_chats(s, limit)),
        TelegramAction::GetMessages {
            chat_id,
            limit,
            from_message_id,
        } => with_session(|s| api::get_messages(s, chat_id, limit, from_message_id)),
        TelegramAction::SendMessage { chat_id, text } => {
            with_session(|s| api::send_message(s, chat_id, &text))
        }
        TelegramAction::ForwardMessage {
            from_chat_id,
            to_chat_id,
            message_ids,
        } => with_session(|s| api::forward_message(s, from_chat_id, to_chat_id, message_ids)),
        TelegramAction::DeleteMessage {
            message_ids,
            revoke,
        } => with_session(|s| api::delete_messages(s, message_ids, revoke)),
        TelegramAction::SearchMessages {
            query,
            chat_id,
            limit,
        } => with_session(|s| api::search_messages(s, &query, chat_id, limit)),
        TelegramAction::GetUpdates => with_session(api::get_updates),
    }
}

/// Load session from workspace, verify it's initialized and logged in, then run the action.
fn with_session(f: impl FnOnce(&Session) -> Result<String, String>) -> Result<String, String> {
    let session = session::load_session().ok_or(
        "No session found. Use the 'login' action first, then save the returned session \
         to telegram/session.json via memory_write."
            .to_string(),
    )?;

    if !session.initialized {
        return Err("Session exists but auth key not generated. Run 'login' again.".into());
    }
    if !session.logged_in {
        return Err("Session exists but not logged in. Complete the login flow \
             (submit_auth_code / submit_2fa_password)."
            .into());
    }

    f(&session)
}

/// Login flow: create session, generate auth key, send verification code.
fn execute_login(phone_number: &str) -> Result<String, String> {
    let api_id = get_api_id()?;
    let api_hash = get_api_hash()?;

    // Default to DC2 (Venus) as it's commonly assigned to new sessions
    let dc_id = 2u8;

    let mut session = Session::new(api_id, api_hash, dc_id);
    session.phone_number = Some(phone_number.to_string());

    // Step 1: DH auth key exchange
    near::agent::host::log(
        near::agent::host::LogLevel::Info,
        "Starting DH auth key exchange with Telegram DC...",
    );
    auth::generate_auth_key(&mut session)?;

    // Step 2: send verification code
    near::agent::host::log(
        near::agent::host::LogLevel::Info,
        "Auth key generated. Sending verification code...",
    );
    let result = api::send_code(&mut session)?;

    // Return session + result so agent can persist it
    let session_json = session::session_to_json(&session)?;
    Ok(format!(
        "{{\"result\":{result},\"session\":{session_json},\"instructions\":\
         \"Save the 'session' object to telegram/session.json using memory_write.\"}}"
    ))
}

/// Submit auth code, return updated session.
fn execute_submit_code(code: &str) -> Result<String, String> {
    let mut session =
        session::load_session().ok_or("No session found. Use 'login' first.".to_string())?;

    let result = api::sign_in(&mut session, code)?;
    let session_json = session::session_to_json(&session)?;

    Ok(format!(
        "{{\"result\":{result},\"session\":{session_json},\"instructions\":\
         \"Save the 'session' object to telegram/session.json using memory_write.\"}}"
    ))
}

/// Submit 2FA password, return updated session.
fn execute_submit_2fa(password: &str) -> Result<String, String> {
    let mut session =
        session::load_session().ok_or("No session found. Use 'login' first.".to_string())?;

    let result = api::check_password(&mut session, password)?;
    let session_json = session::session_to_json(&session)?;

    Ok(format!(
        "{{\"result\":{result},\"session\":{session_json},\"instructions\":\
         \"Save the 'session' object to telegram/session.json using memory_write.\"}}"
    ))
}

/// Read api_id from params or check secret existence.
fn get_api_id() -> Result<i32, String> {
    // The secret store holds the value but WASM can't read it directly.
    // The api_id is injected via env or must be in capabilities.
    // For now, read from workspace config if available.
    if let Some(val) = near::agent::host::workspace_read("telegram/api_id") {
        return val
            .trim()
            .parse::<i32>()
            .map_err(|e| format!("invalid api_id in workspace: {e}"));
    }
    Err(
        "Telegram API ID not found. Store it in workspace at telegram/api_id \
         (just the numeric value) using memory_write."
            .into(),
    )
}

fn get_api_hash() -> Result<String, String> {
    if let Some(val) = near::agent::host::workspace_read("telegram/api_hash") {
        let trimmed = val.trim().to_string();
        if trimmed.is_empty() {
            return Err("telegram/api_hash is empty".into());
        }
        return Ok(trimmed);
    }
    Err(
        "Telegram API hash not found. Store it in workspace at telegram/api_hash \
         using memory_write."
            .into(),
    )
}

const SCHEMA: &str = r#"{
    "type": "object",
    "required": ["action"],
    "properties": {
        "action": {
            "type": "string",
            "enum": ["login", "submit_auth_code", "submit_2fa_password", "get_me", "get_contacts", "get_chats", "get_messages", "send_message", "forward_message", "delete_message", "search_messages", "get_updates"],
            "description": "The Telegram operation to perform"
        },
        "phone_number": {
            "type": "string",
            "description": "Phone number in international format (e.g., '+1234567890'). Required for: login"
        },
        "code": {
            "type": "string",
            "description": "Verification code received via SMS or Telegram. Required for: submit_auth_code"
        },
        "password": {
            "type": "string",
            "description": "Two-factor authentication password. Required for: submit_2fa_password"
        },
        "chat_id": {
            "type": "integer",
            "description": "Chat ID (negative for groups/channels). Required for: get_messages, send_message. Optional for: search_messages"
        },
        "limit": {
            "type": "integer",
            "description": "Maximum number of results (default: 20). Used by: get_chats, get_messages, search_messages",
            "default": 20
        },
        "from_message_id": {
            "type": "integer",
            "description": "Start from this message ID for pagination. Used by: get_messages"
        },
        "text": {
            "type": "string",
            "description": "Message text. Required for: send_message"
        },
        "from_chat_id": {
            "type": "integer",
            "description": "Source chat ID. Required for: forward_message"
        },
        "to_chat_id": {
            "type": "integer",
            "description": "Destination chat ID. Required for: forward_message"
        },
        "message_ids": {
            "type": "array",
            "items": { "type": "integer" },
            "description": "Message IDs. Required for: forward_message, delete_message"
        },
        "revoke": {
            "type": "boolean",
            "description": "Also delete for other participants (default: false). Used by: delete_message",
            "default": false
        },
        "query": {
            "type": "string",
            "description": "Search query. Required for: search_messages"
        }
    }
}"#;

export!(TelegramTool);
