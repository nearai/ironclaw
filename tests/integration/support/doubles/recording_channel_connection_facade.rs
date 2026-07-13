/// Test double substituting the production `ChannelConnectionFacade` impl
/// (`SlackChannelConnectionFacade`,
/// `crates/ironclaw_reborn_composition/src/slack/slack_channel_connection.rs`).
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_product_workflow::{
    ChannelConnectionFacade, RebornServicesError, WebUiAuthenticatedCaller,
};

/// Records `(caller user, channel)` disconnect calls; reports a fixed
/// per-channel connection map so removal qualification sees the channel as
/// connected.
#[derive(Default)]
pub(crate) struct RecordingChannelConnectionFacade {
    connections: HashMap<String, bool>,
    disconnects: Arc<Mutex<Vec<(String, String)>>>,
}

impl RecordingChannelConnectionFacade {
    pub(crate) fn with_connections(entries: &[(&str, bool)]) -> Self {
        Self {
            connections: entries
                .iter()
                .map(|(channel, connected)| ((*channel).to_string(), *connected))
                .collect(),
            disconnects: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Recorded disconnect calls as `(user_id, channel)`.
    pub(crate) fn disconnects(&self) -> Vec<(String, String)> {
        self.disconnects.lock().unwrap().clone()
    }
}

#[async_trait]
impl ChannelConnectionFacade for RecordingChannelConnectionFacade {
    async fn caller_channel_connections(
        &self,
        _caller: WebUiAuthenticatedCaller,
    ) -> Result<HashMap<String, bool>, RebornServicesError> {
        Ok(self.connections.clone())
    }

    async fn disconnect_channel_for_caller(
        &self,
        caller: WebUiAuthenticatedCaller,
        channel: &str,
    ) -> Result<(), RebornServicesError> {
        self.disconnects
            .lock()
            .unwrap()
            .push((caller.user_id.as_str().to_string(), channel.to_string()));
        Ok(())
    }
}
