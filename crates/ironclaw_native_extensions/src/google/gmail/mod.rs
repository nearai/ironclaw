//! Gmail native extension package.
//!
//! [`manifest`] declares the HostBundled `ExtensionPackage` (six capability
//! descriptors); [`handlers`] holds the six first-party capability handler
//! implementations. [`register_gmail`] populates a [`RegistrationOutput`] with
//! the package and the keyed handlers.

pub mod handlers;
pub mod manifest;

use std::sync::Arc;

use ironclaw_host_api::CapabilityId;
use ironclaw_host_runtime::FirstPartyCapabilityHandler;
use ironclaw_oauth::OAuthProvider;

use crate::google::credential::GoogleCredentialResolver;
use crate::google::scopes;
use crate::{NativeExtensionError, RegistrationOutput};

use handlers::{
    CreateDraftHandler, GetMessageHandler, GmailHandlerDeps, ListMessagesHandler,
    ReplyToMessageHandler, SendMessageHandler, TrashMessageHandler,
};
use manifest::{capability_id, gmail_package};

/// Build a keyed `(CapabilityId, handler)` pair, mapping id-construction
/// failure to a [`NativeExtensionError`].
fn keyed(
    short_name: &str,
    handler: Arc<dyn FirstPartyCapabilityHandler>,
) -> Result<(CapabilityId, Arc<dyn FirstPartyCapabilityHandler>), NativeExtensionError> {
    let id = CapabilityId::new(capability_id(short_name))?;
    Ok((id, handler))
}

/// Register the Gmail package and its six capability handlers into `output`.
///
/// `resolver` is the shared credential resolver (used by handlers for the
/// scope-mismatch preflight); `provider` is the shared Google `OAuthProvider`.
/// Both are shared with the Google Calendar package — Gmail and Calendar
/// resolve the same `google_oauth_token` credential. Handlers do not own an
/// HTTP transport — they issue calls through the per-invocation
/// `runtime_http_egress` the host supplies in `InvocationServices`.
///
/// Scope requirements (per capability):
/// - `list_messages`, `get_message`: `gmail.readonly`
/// - `send_message`: `gmail.send`
/// - `create_draft`, `trash_message`: `gmail.modify`
/// - `reply_to_message`: `gmail.send` + `gmail.modify`
pub fn register_gmail(
    resolver: Arc<GoogleCredentialResolver>,
    provider: Arc<dyn OAuthProvider>,
    output: &mut RegistrationOutput,
) -> Result<(), NativeExtensionError> {
    output.packages.push(gmail_package()?);

    let readonly_scopes = vec![scopes::GMAIL_READONLY.to_string()];
    let send_scopes = vec![scopes::GMAIL_SEND.to_string()];
    let modify_scopes = vec![scopes::GMAIL_MODIFY.to_string()];
    let reply_scopes = vec![
        scopes::GMAIL_SEND.to_string(),
        scopes::GMAIL_MODIFY.to_string(),
    ];

    let read_deps = GmailHandlerDeps::new(resolver.clone(), provider.clone(), readonly_scopes);
    let send_deps = GmailHandlerDeps::new(resolver.clone(), provider.clone(), send_scopes);
    let modify_deps = GmailHandlerDeps::new(resolver.clone(), provider.clone(), modify_scopes);
    let reply_deps = GmailHandlerDeps::new(resolver, provider, reply_scopes);

    let handlers: Vec<(&str, Arc<dyn FirstPartyCapabilityHandler>)> = vec![
        (
            "list_messages",
            Arc::new(ListMessagesHandler::new(read_deps.clone())),
        ),
        ("get_message", Arc::new(GetMessageHandler::new(read_deps))),
        ("send_message", Arc::new(SendMessageHandler::new(send_deps))),
        (
            "create_draft",
            Arc::new(CreateDraftHandler::new(modify_deps.clone())),
        ),
        (
            "reply_to_message",
            Arc::new(ReplyToMessageHandler::new(reply_deps)),
        ),
        (
            "trash_message",
            Arc::new(TrashMessageHandler::new(modify_deps)),
        ),
    ];

    for (short_name, handler) in handlers {
        output.handlers.push(keyed(short_name, handler)?);
    }
    Ok(())
}
