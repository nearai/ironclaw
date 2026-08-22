//! The linked account's [`ToolAdapter`]: one adapter instance for the whole
//! extension, routing by capability id.
//!
//! What this file is responsible for is deliberately small — resolve the
//! caller's linked account, get its live session, route, and bound the call. It
//! does not validate input (the host does, against the canonical schema), does
//! not decide permissions (the manifest and the origin gate matrix do), does
//! not open sockets ([`super::transport`] does), and does not know what a
//! Telegram message looks like ([`super::mapping`] does).
//!
//! **`ToolPorts` is unused, and that is the manifest being honest.** These
//! tools declare no `network` effect, because `network` in this workspace means
//! *host-mediated* egress and the linked account is reached over a raw MTProto
//! socket the host does not mediate (PROPOSAL §3.4, §6.7). No declared egress
//! means no egress port is derived, so there is nothing here to use — an
//! adapter cannot reach authority its manifest never named.

use std::sync::Arc;

use ironclaw_extension_contracts::linked_session::LinkedAccountResolver;
use ironclaw_extension_contracts::tool_adapter::{
    ToolAdapter, ToolCall, ToolError, ToolPorts, ToolResult,
};
use ironclaw_host_api::{
    dispatch::RuntimeDispatchErrorKind,
    messaging::{StandardMessagingErrorCode, StandardMessagingOp},
};
use serde_json::Value;

use super::{
    TOOL_CALL_TIMEOUT, mapping, ops,
    pool::{PooledSession, SessionPool},
};

/// The capability-id prefix every bound tool carries. The manifest binding
/// validation already requires `telegram.<op_name>`; this is the same fact on
/// the execution side, where a mismatch means "not ours" rather than a parse
/// error.
const CAPABILITY_PREFIX: &str = "telegram.";

/// Routes model-callable Telegram operations onto the caller's linked account.
pub struct TelegramLinkedToolAdapter {
    accounts: Arc<dyn LinkedAccountResolver>,
    pool: Arc<SessionPool>,
}

impl TelegramLinkedToolAdapter {
    /// Build the adapter. Performs no I/O: the pool connects lazily on the
    /// first call that needs a session, which is what keeps `bind`
    /// contractually I/O-free.
    pub fn new(accounts: Arc<dyn LinkedAccountResolver>, pool: Arc<SessionPool>) -> Self {
        Self { accounts, pool }
    }

    /// Resolve the caller's account and get its live session.
    ///
    /// The account comes from the host through [`LinkedAccountResolver`], never
    /// from a value this adapter derives itself: a bare user id taken off the
    /// call is exactly the axis PROPOSAL §3.3/§5.1 establish is not trustworthy
    /// for isolation.
    async fn session(&self, call: &ToolCall) -> Result<Arc<PooledSession>, ToolError> {
        let grant = self
            .accounts
            .resolve(&call.scope)
            .await
            .map_err(ops::resolution_error)?;
        let session = self.pool.acquire(&grant).await?;
        Ok(session)
    }
}

#[async_trait::async_trait]
impl ToolAdapter for TelegramLinkedToolAdapter {
    async fn invoke(
        &self,
        call: ToolCall,
        _ports: &ToolPorts<'_>,
    ) -> Result<ToolResult, ToolError> {
        let Some(op) = bound_op(call.capability_id.as_str()) else {
            return Err(undeclared_capability(call.capability_id.as_str()));
        };

        // `open_dm` is a pure re-encode with no vendor call, so it must not
        // require — or pay for — a live connection. Every other op does.
        if op == StandardMessagingOp::OpenDm {
            return ops::writes::open_dm(&call.input).map(tool_result);
        }

        let session = self.session(&call).await?;
        // The wall-clock bound belongs here rather than inside each op: a call
        // that has stopped making progress must end as a failure the run can
        // recover from, not hold a pooled session open indefinitely. Unlink
        // does NOT wait for this — the generation fence fails in-flight work at
        // its next RPC.
        let output = tokio::time::timeout(TOOL_CALL_TIMEOUT, run(op, &session, &call.input))
            .await
            .unwrap_or_else(|_elapsed| {
                Err(mapping::failed_because(
                    StandardMessagingErrorCode::VendorError,
                    "the telegram request did not complete in time".to_string(),
                ))
            })?;
        Ok(tool_result(output))
    }
}

/// The binding map: capability id → standard operation.
///
/// `get_thread_replies` is the one core op deliberately absent — Telegram keeps
/// thread replies inside the chat history, so `get_conversation_history`
/// already returns them (PROPOSAL §6.1).
fn bound_op(capability_id: &str) -> Option<StandardMessagingOp> {
    let op_name = capability_id.strip_prefix(CAPABILITY_PREFIX)?;
    let op = StandardMessagingOp::ALL
        .iter()
        .copied()
        .find(|op| op.op_name() == op_name)?;
    matches!(
        op,
        StandardMessagingOp::SendMessage
            | StandardMessagingOp::EditMessage
            | StandardMessagingOp::DeleteMessage
            | StandardMessagingOp::AddReaction
            | StandardMessagingOp::RemoveReaction
            | StandardMessagingOp::OpenDm
            | StandardMessagingOp::ListConversations
            | StandardMessagingOp::GetConversationInfo
            | StandardMessagingOp::GetConversationHistory
            | StandardMessagingOp::GetMessage
            | StandardMessagingOp::SearchMessages
            | StandardMessagingOp::Whoami
            | StandardMessagingOp::GetUserInfo
            | StandardMessagingOp::ResolveUser
            | StandardMessagingOp::ListMembers
    )
    .then_some(op)
}

async fn run(
    op: StandardMessagingOp,
    session: &PooledSession,
    input: &Value,
) -> Result<Value, ToolError> {
    use StandardMessagingOp as Op;
    match op {
        Op::SendMessage => ops::writes::send_message(session, input).await,
        Op::EditMessage => ops::writes::edit_message(session, input).await,
        Op::DeleteMessage => ops::writes::delete_message(session, input).await,
        Op::AddReaction => ops::writes::add_reaction(session, input).await,
        Op::RemoveReaction => ops::writes::remove_reaction(session, input).await,
        Op::ListConversations => ops::reads::list_conversations(session, input).await,
        Op::GetConversationInfo => ops::reads::get_conversation_info(session, input).await,
        Op::GetConversationHistory => ops::reads::get_conversation_history(session, input).await,
        Op::GetMessage => ops::reads::get_message(session, input).await,
        Op::SearchMessages => ops::reads::search_messages(session, input).await,
        Op::Whoami => ops::people::whoami(session).await,
        Op::GetUserInfo => ops::people::get_user_info(session, input).await,
        Op::ResolveUser => ops::people::resolve_user(session, input).await,
        Op::ListMembers => ops::people::list_members(session, input).await,
        // `open_dm` never reaches here — it is answered before a session is
        // acquired — and every remaining variant is unbound.
        Op::OpenDm => Err(undeclared_capability(op.op_name())),
        _ => Err(undeclared_capability(op.op_name())),
    }
}

fn undeclared_capability(_id: &str) -> ToolError {
    ToolError::Rejected {
        kind: RuntimeDispatchErrorKind::UndeclaredCapability,
        diagnostic: None,
        detail: None,
    }
}

/// Projects one op's canonical output as the adapter's success result — the
/// package-side face of a `Completed` outcome. `pub(crate)` so the conformance
/// suite (`super::conformance`, test-only) can assert what an op's output
/// actually becomes rather than re-deriving it.
pub(crate) fn tool_result(output: Value) -> ToolResult {
    let output_bytes = serde_json::to_vec(&output)
        .map(|bytes| bytes.len() as u64)
        .unwrap_or_default();
    ToolResult {
        output,
        display_preview: None,
        output_bytes,
    }
}

#[cfg(test)]
mod tests;
