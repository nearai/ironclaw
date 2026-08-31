//! The Slack Web API endpoints this package calls — one inventory.
//!
//! Every vendor call the channel halves make names its endpoint here, so the
//! manifest's `[[channel.egress]]` allowlist is checked against the code in
//! lockstep (`tests/agent_app_manifest_lockstep.rs`) instead of by reading
//! format strings, and a new call cannot ship without its egress declaration.

use ironclaw_host_api::action::NetworkMethod;

use crate::payload::SLACK_API_HOST;

/// One `slack.com/api/<method>` endpoint the package calls with the bot
/// token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SlackWebApiMethod {
    ChatPostMessage,
    ChatPostEphemeral,
    ChatDelete,
    ConversationsOpen,
    ReactionsAdd,
    ReactionsRemove,
    FilesGetUploadUrlExternal,
    FilesCompleteUploadExternal,
    FilesInfo,
    ConversationsHistory,
    ConversationsReplies,
    /// Native Agent reply surface: opens the streaming message.
    ChatStartStream,
    /// Native Agent reply surface: appends deltas and task updates.
    ChatAppendStream,
    /// Native Agent reply surface: closes the stream and sets the session
    /// status in one call.
    ChatStopStream,
    /// Native Agent reply surface: session lifecycle (`processing`,
    /// `suspended`, `active`).
    AgentsSessionsSetStatus,
}

impl SlackWebApiMethod {
    /// Every endpoint the package calls, in one place.
    pub const ALL: &'static [Self] = &[
        Self::ChatPostMessage,
        Self::ChatPostEphemeral,
        Self::ChatDelete,
        Self::ConversationsOpen,
        Self::ReactionsAdd,
        Self::ReactionsRemove,
        Self::FilesGetUploadUrlExternal,
        Self::FilesCompleteUploadExternal,
        Self::FilesInfo,
        Self::ConversationsHistory,
        Self::ConversationsReplies,
        Self::ChatStartStream,
        Self::ChatAppendStream,
        Self::ChatStopStream,
        Self::AgentsSessionsSetStatus,
    ];

    /// The Slack method name (`chat.postMessage`).
    pub fn name(self) -> &'static str {
        match self {
            Self::ChatPostMessage => "chat.postMessage",
            Self::ChatPostEphemeral => "chat.postEphemeral",
            Self::ChatDelete => "chat.delete",
            Self::ConversationsOpen => "conversations.open",
            Self::ReactionsAdd => "reactions.add",
            Self::ReactionsRemove => "reactions.remove",
            Self::FilesGetUploadUrlExternal => "files.getUploadURLExternal",
            Self::FilesCompleteUploadExternal => "files.completeUploadExternal",
            Self::FilesInfo => "files.info",
            Self::ConversationsHistory => "conversations.history",
            Self::ConversationsReplies => "conversations.replies",
            Self::ChatStartStream => "chat.startStream",
            Self::ChatAppendStream => "chat.appendStream",
            Self::ChatStopStream => "chat.stopStream",
            Self::AgentsSessionsSetStatus => "agents.sessions.setStatus",
        }
    }

    /// The request path under `slack.com` (`/api/chat.postMessage`), the
    /// exact string the manifest's `[[channel.egress]] paths` must list.
    pub fn path(self) -> &'static str {
        match self {
            Self::ChatPostMessage => "/api/chat.postMessage",
            Self::ChatPostEphemeral => "/api/chat.postEphemeral",
            Self::ChatDelete => "/api/chat.delete",
            Self::ConversationsOpen => "/api/conversations.open",
            Self::ReactionsAdd => "/api/reactions.add",
            Self::ReactionsRemove => "/api/reactions.remove",
            Self::FilesGetUploadUrlExternal => "/api/files.getUploadURLExternal",
            Self::FilesCompleteUploadExternal => "/api/files.completeUploadExternal",
            Self::FilesInfo => "/api/files.info",
            Self::ConversationsHistory => "/api/conversations.history",
            Self::ConversationsReplies => "/api/conversations.replies",
            Self::ChatStartStream => "/api/chat.startStream",
            Self::ChatAppendStream => "/api/chat.appendStream",
            Self::ChatStopStream => "/api/chat.stopStream",
            Self::AgentsSessionsSetStatus => "/api/agents.sessions.setStatus",
        }
    }

    /// The HTTP method the package uses for this endpoint.
    pub fn http_method(self) -> NetworkMethod {
        match self {
            Self::FilesGetUploadUrlExternal
            | Self::FilesInfo
            | Self::ConversationsHistory
            | Self::ConversationsReplies => NetworkMethod::Get,
            Self::ChatPostMessage
            | Self::ChatPostEphemeral
            | Self::ChatDelete
            | Self::ConversationsOpen
            | Self::ReactionsAdd
            | Self::ReactionsRemove
            | Self::FilesCompleteUploadExternal
            | Self::ChatStartStream
            | Self::ChatAppendStream
            | Self::ChatStopStream
            | Self::AgentsSessionsSetStatus => NetworkMethod::Post,
        }
    }

    /// The absolute URL without a query string.
    pub fn url(self) -> String {
        format!("https://{SLACK_API_HOST}{}", self.path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_endpoint_names_a_distinct_exact_api_path() {
        let mut paths: Vec<&str> = SlackWebApiMethod::ALL
            .iter()
            .map(|method| method.path())
            .collect();
        paths.sort_unstable();
        paths.dedup();
        assert_eq!(paths.len(), SlackWebApiMethod::ALL.len());
        for method in SlackWebApiMethod::ALL {
            assert_eq!(method.path(), format!("/api/{}", method.name()));
            assert_eq!(method.url(), format!("https://slack.com{}", method.path()));
        }
    }
}
