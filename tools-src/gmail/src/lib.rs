//! Gmail WASM Tool for IronClaw.
//!
//! Provides Gmail integration for reading, searching, sending, drafting,
//! and replying to emails.
//!
//! # Capabilities Required
//!
//! - HTTP: `gmail.googleapis.com/gmail/v1/*` (GET, POST, DELETE)
//! - Secrets: `google_oauth_token` (shared OAuth 2.0 token, injected automatically)
//!
//! # Supported Actions
//!
//! - `list_messages`: List/search messages with Gmail query syntax
//! - `get_message`: Get a specific message with full content
//! - `send_message`: Send a new email
//! - `create_draft`: Create a draft email
//! - `reply_to_message`: Reply to an existing message (or reply-all)
//! - `trash_message`: Move a message to trash
//!
//! # Example Usage
//!
//! ```json
//! {"action": "list_messages", "query": "is:unread from:boss@company.com", "max_results": 5}
//! ```

mod api;
mod types;

use types::GmailAction;

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

struct GmailTool;

impl exports::near::agent::tool::Guest for GmailTool {
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
        // Derived from `GmailAction` via `schemars::JsonSchema` so the
        // advertised schema can never drift from the serde contract.
        let schema = schemars::schema_for!(types::GmailAction);
        serde_json::to_string(&schema).expect("schema serialization is infallible")
    }

    fn description() -> String {
        "Gmail integration for reading, searching, sending, drafting, and replying to emails. \
         Supports Gmail search query syntax (is:unread, from:, subject:, after:, etc.). \
         Requires a Google OAuth token with gmail.modify and gmail.compose scopes. \
         To discover all available API operations, use http GET to fetch \
         <https://www.googleapis.com/discovery/v1/apis/gmail/v1/rest> (public, no auth needed)."
            .to_string()
    }
}

fn execute_inner(params: &str) -> Result<String, String> {
    if !crate::near::agent::host::secret_exists("google_oauth_token") {
        return Err(
            "Google OAuth token not configured. Run `ironclaw tool auth gmail` to set up \
             OAuth, or set the GOOGLE_OAUTH_TOKEN environment variable."
                .to_string(),
        );
    }

    let action: GmailAction =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {}", e))?;

    crate::near::agent::host::log(
        crate::near::agent::host::LogLevel::Info,
        &format!("Executing Gmail action: {:?}", action),
    );

    let result = match action {
        GmailAction::ListMessages {
            query,
            max_results,
            label_ids,
        } => {
            let result = api::list_messages(query.as_deref(), max_results, &label_ids)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GmailAction::GetMessage { message_id } => {
            let result = api::get_message(&message_id)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GmailAction::SendMessage {
            to,
            subject,
            body,
            cc,
            bcc,
        } => {
            let result = api::send_message(&to, &subject, &body, cc.as_deref(), bcc.as_deref())?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GmailAction::CreateDraft {
            to,
            subject,
            body,
            cc,
            bcc,
        } => {
            let result = api::create_draft(&to, &subject, &body, cc.as_deref(), bcc.as_deref())?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GmailAction::ReplyToMessage {
            message_id,
            body,
            reply_all,
        } => {
            let result = api::reply_to_message(&message_id, &body, reply_all)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        GmailAction::TrashMessage { message_id } => {
            let result = api::trash_message(&message_id)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }
    };

    Ok(result)
}

export!(GmailTool);
