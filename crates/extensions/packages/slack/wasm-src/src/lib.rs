//! Slack personal (user-token) WASM Tool for IronClaw Reborn.
//!
//! Unlike the bot-token `slack` tool, this tool authenticates with a Slack
//! **user token** (`xoxp-`) stored under the `slack_user_token` secret, so it
//! acts as the user. That lets it search all of the user's messages, list and
//! read their DMs and group DMs, read channel history, and post as them.
//!
//! # Capabilities Required
//!
//! - HTTP: `slack.com/api/*` (GET, POST)
//! - Secrets: `slack_user_token` (injected automatically as a bearer token)
//!
//! # Supported Actions
//!
//! All 16 core operations of the standardized messaging framework are bound:
//!
//! - `search_messages`: Search all messages the user can see
//! - `list_conversations`: List channels, DMs, and group DMs the user is in
//! - `get_conversation_info`: Retrieve one exact conversation by ID
//! - `get_conversation_history`: Read history of any channel or DM
//! - `get_thread_replies`: Read one thread's replies (not part of history)
//! - `get_message`: Read one message by reference (history/thread lookup)
//! - `get_user_info`: Get information about a Slack user
//! - `resolve_user`: Search the workspace directory for people
//! - `list_members`: List the members of a conversation
//! - `whoami`: Resolve who the connected account is (auth.test)
//! - `send_message`: Post a message as the user
//! - `edit_message`: Replace the text of one of the user's messages
//! - `delete_message`: Delete one of the user's messages
//! - `add_reaction` / `remove_reaction`: React as the user
//! - `open_dm`: Open (or fetch) the DM with one person
//!
//! # Example Usage
//!
//! The host selects the operation from the capability id (e.g.
//! `slack.search_messages`); params carry only the operation's fields and
//! must NOT include an `action` key:
//!
//! ```json
//! {"query": "from:me project plan", "limit": 20}
//! ```

mod api;
mod types;

use types::{SlackUserAction, ToolContext};

// Generate bindings from the WIT interface.
wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../../../lanes/ironclaw_wasm/wit/tool.wit",
});

use exports::near::agent::tool::{ErrorKind, GuestFailure, Response};

/// Implementation of the tool interface.
struct SlackUserTool;

impl exports::near::agent::tool::Guest for SlackUserTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match execute_inner(&req.params, req.context.as_deref()) {
            Ok(result) => Response::Success(result),
            Err(failure) => Response::Failure(failure),
        }
    }

    fn schema() -> String {
        // Derived from `SlackUserAction` via `schemars::JsonSchema` so the
        // advertised schema can never drift from the serde contract.
        let schema = schemars::schema_for!(types::SlackUserAction);
        serde_json::to_string(&schema).unwrap_or_else(|_| "{}".to_string())
    }

    fn description() -> String {
        "Slack personal tool that acts as you via a user token (xoxp-): search all your \
         messages, list and read your DMs and group DMs, read channel history, look up and \
         search for people, list conversation members, and act as you — post, edit, delete, \
         react, and open DMs. Requires a Slack user token with scopes such as search:read, \
         channels:history, groups:history, im:history, mpim:history, users:read, chat:write \
         (post/edit/delete), reactions:read and reactions:write (reactions), and im:write \
         (opening DMs)."
            .to_string()
    }
}

/// Build an `input`-kind guest failure with a stable code and no free-text
/// message (the code alone is the actionable signal for these host-plumbing
/// failures).
fn input_failure(code: &'static str) -> GuestFailure {
    GuestFailure {
        kind: ErrorKind::Input,
        code: Some(code.to_string()),
        message: None,
    }
}

/// Inner execution logic. The host selects the operation via the capability id
/// in the invocation context; params carry only the operation's fields (no
/// `action` key). The Slack user token is injected by the host as a bearer
/// credential — a missing credential surfaces as an auth gate, not here.
fn execute_inner(params: &str, context: Option<&str>) -> Result<String, GuestFailure> {
    let action_name = action_from_context(context)?;
    let params = params_with_action(params, action_name)?;
    let action: SlackUserAction = serde_json::from_value(params).map_err(|e| GuestFailure {
        kind: ErrorKind::Input,
        code: Some("invalid_parameters".to_string()),
        message: Some(api::bounded_message(&e.to_string())),
    })?;

    crate::near::agent::host::log(
        crate::near::agent::host::LogLevel::Debug,
        &format!("Executing Slack user action: {action_name}"),
    );

    let result = match action {
        SlackUserAction::SearchMessages {
            query,
            sort,
            limit,
            cursor,
        } => {
            let result = api::search_messages(&query, sort, limit, cursor.as_deref())?;
            serde_json::to_string(&result).map_err(|e| api::serialization_failure(&e))?
        }

        SlackUserAction::ListConversations {
            kinds,
            limit,
            cursor,
        } => {
            let result = api::list_conversations(kinds.as_deref(), limit, cursor.as_deref())?;
            serde_json::to_string(&result).map_err(|e| api::serialization_failure(&e))?
        }

        SlackUserAction::GetConversationInfo { conversation } => {
            let result = api::get_conversation_info(&conversation)?;
            serde_json::to_string(&result).map_err(|e| api::serialization_failure(&e))?
        }

        SlackUserAction::GetConversationHistory {
            conversation,
            limit,
            cursor,
        } => {
            let result = api::get_conversation_history(&conversation, limit, cursor.as_deref())?;
            serde_json::to_string(&result).map_err(|e| api::serialization_failure(&e))?
        }

        SlackUserAction::GetThreadReplies {
            conversation,
            thread,
            limit,
            cursor,
        } => {
            let result = api::get_thread_replies(&conversation, &thread, limit, cursor.as_deref())?;
            serde_json::to_string(&result).map_err(|e| api::serialization_failure(&e))?
        }

        SlackUserAction::GetUserInfo { user_ref } => {
            let result = api::get_user_info(&user_ref)?;
            serde_json::to_string(&result).map_err(|e| api::serialization_failure(&e))?
        }

        SlackUserAction::Whoami => {
            let result = api::whoami()?;
            serde_json::to_string(&result).map_err(|e| api::serialization_failure(&e))?
        }

        SlackUserAction::SendMessage {
            conversation,
            text,
            thread,
            reply_to,
        } => {
            let result =
                api::send_message(&conversation, &text, thread.as_deref(), reply_to.as_ref())?;
            serde_json::to_string(&result).map_err(|e| api::serialization_failure(&e))?
        }

        SlackUserAction::EditMessage { message_ref, text } => {
            let result = api::edit_message(&message_ref, &text)?;
            serde_json::to_string(&result).map_err(|e| api::serialization_failure(&e))?
        }

        SlackUserAction::DeleteMessage { message_ref } => {
            let result = api::delete_message(&message_ref)?;
            serde_json::to_string(&result).map_err(|e| api::serialization_failure(&e))?
        }

        SlackUserAction::AddReaction { message_ref, emoji } => {
            let result = api::add_reaction(&message_ref, &emoji)?;
            serde_json::to_string(&result).map_err(|e| api::serialization_failure(&e))?
        }

        SlackUserAction::RemoveReaction { message_ref, emoji } => {
            let result = api::remove_reaction(&message_ref, emoji.as_deref())?;
            serde_json::to_string(&result).map_err(|e| api::serialization_failure(&e))?
        }

        SlackUserAction::OpenDm { user_ref } => {
            let result = api::open_dm(&user_ref)?;
            serde_json::to_string(&result).map_err(|e| api::serialization_failure(&e))?
        }

        SlackUserAction::GetMessage { message_ref } => {
            let result = api::get_message(&message_ref)?;
            serde_json::to_string(&result).map_err(|e| api::serialization_failure(&e))?
        }

        SlackUserAction::ResolveUser {
            query,
            limit,
            cursor,
        } => {
            let result = api::resolve_user(&query, limit, cursor.as_deref())?;
            serde_json::to_string(&result).map_err(|e| api::serialization_failure(&e))?
        }

        SlackUserAction::ListMembers {
            conversation,
            limit,
            cursor,
        } => {
            let result = api::list_members(&conversation, limit, cursor.as_deref())?;
            serde_json::to_string(&result).map_err(|e| api::serialization_failure(&e))?
        }
    };

    Ok(result)
}

/// Map a capability id (e.g. `slack.search_messages`) to the serde action
/// tag the params enum expects.
fn action_from_context(context: Option<&str>) -> Result<&'static str, GuestFailure> {
    let context = context.ok_or_else(|| input_failure("missing_invocation_context"))?;
    let context: ToolContext = serde_json::from_str(context).map_err(|e| GuestFailure {
        kind: ErrorKind::Input,
        code: Some("invalid_invocation_context".to_string()),
        message: Some(api::bounded_message(&e.to_string())),
    })?;
    match context.capability_id.as_str() {
        "slack.search_messages" => Ok("search_messages"),
        "slack.list_conversations" => Ok("list_conversations"),
        "slack.get_conversation_info" => Ok("get_conversation_info"),
        "slack.get_conversation_history" => Ok("get_conversation_history"),
        "slack.get_thread_replies" => Ok("get_thread_replies"),
        "slack.get_user_info" => Ok("get_user_info"),
        "slack.whoami" => Ok("whoami"),
        "slack.send_message" => Ok("send_message"),
        "slack.edit_message" => Ok("edit_message"),
        "slack.delete_message" => Ok("delete_message"),
        "slack.add_reaction" => Ok("add_reaction"),
        "slack.remove_reaction" => Ok("remove_reaction"),
        "slack.open_dm" => Ok("open_dm"),
        "slack.get_message" => Ok("get_message"),
        "slack.resolve_user" => Ok("resolve_user"),
        "slack.list_members" => Ok("list_members"),
        _ => Err(input_failure("unsupported_slack_user_capability")),
    }
}

/// Inject the host-selected `action` tag into the params object so the tagged
/// `SlackUserAction` enum can deserialize. Rejects params that already carry an
/// `action` key (the host owns operation selection).
fn params_with_action(params: &str, action: &str) -> Result<serde_json::Value, GuestFailure> {
    let mut params: serde_json::Value = if params.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(params).map_err(|_| input_failure("invalid_parameters"))?
    };
    let obj = params
        .as_object_mut()
        .ok_or_else(|| input_failure("invalid_parameters"))?;
    if obj.contains_key("action") {
        return Err(input_failure("invalid_parameters"));
    }
    obj.insert(
        "action".to_string(),
        serde_json::Value::String(action.to_string()),
    );
    Ok(params)
}

// Export the tool implementation.
export!(SlackUserTool);

#[cfg(test)]
mod tests {
    use super::*;

    /// Every capability id the manifest declares must resolve to the serde
    /// action tag its params enum expects. A manifest entry without a mapping
    /// arm here reaches the guest and dies as
    /// `unsupported_slack_user_capability` at call time — an install-time
    /// omission surfacing as a runtime failure, which this catches instead.
    ///
    /// The list is the manifest's 16 `[[tools]]` ids, which are also the 16
    /// core standard messaging operations (the binding rule fixes a bound
    /// tool's id at `<extension_id>.<op_name>`).
    const CAPABILITY_IDS: &[(&str, &str)] = &[
        ("slack.search_messages", "search_messages"),
        ("slack.list_conversations", "list_conversations"),
        ("slack.get_conversation_info", "get_conversation_info"),
        ("slack.get_conversation_history", "get_conversation_history"),
        ("slack.get_thread_replies", "get_thread_replies"),
        ("slack.get_message", "get_message"),
        ("slack.get_user_info", "get_user_info"),
        ("slack.resolve_user", "resolve_user"),
        ("slack.list_members", "list_members"),
        ("slack.whoami", "whoami"),
        ("slack.send_message", "send_message"),
        ("slack.edit_message", "edit_message"),
        ("slack.delete_message", "delete_message"),
        ("slack.add_reaction", "add_reaction"),
        ("slack.remove_reaction", "remove_reaction"),
        ("slack.open_dm", "open_dm"),
    ];

    fn context(capability_id: &str) -> String {
        serde_json::json!({ "capability_id": capability_id }).to_string()
    }

    #[test]
    fn every_manifest_capability_id_maps_to_an_action() {
        assert_eq!(
            CAPABILITY_IDS.len(),
            16,
            "Slack binds all 16 core standard messaging operations"
        );
        for (capability_id, expected_action) in CAPABILITY_IDS {
            let context = context(capability_id);
            assert_eq!(
                action_from_context(Some(&context)).expect(capability_id),
                *expected_action,
                "{capability_id}"
            );
        }
    }

    #[test]
    fn an_unknown_capability_id_is_rejected() {
        let context = context("slack.pin_message");
        assert!(action_from_context(Some(&context)).is_err());
        assert!(action_from_context(None).is_err());
    }

    /// The host owns operation selection: params carrying their own `action`
    /// key would let a caller invoke one operation through another's
    /// authorization.
    #[test]
    fn params_cannot_smuggle_their_own_action() {
        let smuggled = params_with_action(r#"{"action":"delete_message"}"#, "get_message");
        assert!(smuggled.is_err(), "params must not carry an action key");

        let injected = params_with_action(r#"{"message_ref":{}}"#, "get_message")
            .expect("host-selected action is injected");
        assert_eq!(injected["action"], serde_json::json!("get_message"));

        // A no-parameter operation still gets a well-formed params object.
        let empty = params_with_action("", "whoami").expect("empty params are an empty object");
        assert_eq!(empty["action"], serde_json::json!("whoami"));
    }
}
