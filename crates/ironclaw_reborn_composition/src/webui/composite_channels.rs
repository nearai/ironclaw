//! Generic composition of per-channel-host WebUI facades.
//!
//! Each channel host (Slack, Telegram, …) contributes its own
//! `ConnectableChannelsProductFacade` / `ChannelConnectionFacade`; the serve
//! layer composes them into the single facade pair
//! `build_webui_services_with_connectable_channels` accepts. Vendor-agnostic
//! by construction: nothing here may key on a concrete channel id.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_product_workflow::{
    ChannelConnectionFacade, ConnectableChannelsProductFacade,
    RebornConnectableChannelListResponse, RebornServicesError, WebUiAuthenticatedCaller,
};

/// Concatenates every inner facade's channel list, preserving order.
pub struct CompositeConnectableChannelsFacade {
    inner: Vec<Arc<dyn ConnectableChannelsProductFacade>>,
}

impl CompositeConnectableChannelsFacade {
    pub fn new(inner: Vec<Arc<dyn ConnectableChannelsProductFacade>>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl ConnectableChannelsProductFacade for CompositeConnectableChannelsFacade {
    async fn list_connectable_channels(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<RebornConnectableChannelListResponse, RebornServicesError> {
        let mut channels = Vec::new();
        for facade in &self.inner {
            channels.extend(
                facade
                    .list_connectable_channels(caller.clone())
                    .await?
                    .channels,
            );
        }
        Ok(RebornConnectableChannelListResponse { channels })
    }
}

/// Merges per-channel connection maps; disconnect routes to the first inner
/// facade that reports a connection concept for the channel.
pub struct CompositeChannelConnectionFacade {
    inner: Vec<Arc<dyn ChannelConnectionFacade>>,
}

impl CompositeChannelConnectionFacade {
    pub fn new(inner: Vec<Arc<dyn ChannelConnectionFacade>>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl ChannelConnectionFacade for CompositeChannelConnectionFacade {
    async fn caller_channel_connections(
        &self,
        caller: WebUiAuthenticatedCaller,
    ) -> Result<HashMap<String, bool>, RebornServicesError> {
        let mut merged = HashMap::new();
        for facade in &self.inner {
            merged.extend(facade.caller_channel_connections(caller.clone()).await?);
        }
        Ok(merged)
    }

    async fn disconnect_channel_for_caller(
        &self,
        caller: WebUiAuthenticatedCaller,
        channel: &str,
    ) -> Result<(), RebornServicesError> {
        for facade in &self.inner {
            let connections = facade.caller_channel_connections(caller.clone()).await?;
            if connections.contains_key(channel) {
                return facade.disconnect_channel_for_caller(caller, channel).await;
            }
        }
        Err(RebornServicesError::internal())
    }
}
