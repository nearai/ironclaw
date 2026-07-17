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
    ChannelConnectionFacade, RebornServicesError, WebUiAuthenticatedCaller,
};
#[cfg(all(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
use ironclaw_product_workflow::{
    ConnectableChannelsProductFacade, RebornConnectableChannelListResponse,
};

/// Concatenates every inner facade's channel list, preserving order.
#[cfg(all(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
pub(crate) struct CompositeConnectableChannelsFacade {
    inner: Vec<Arc<dyn ConnectableChannelsProductFacade>>,
}

#[cfg(all(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
impl CompositeConnectableChannelsFacade {
    pub(crate) fn new(inner: Vec<Arc<dyn ConnectableChannelsProductFacade>>) -> Self {
        Self { inner }
    }
}

#[async_trait]
#[cfg(all(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
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
pub(crate) struct CompositeChannelConnectionFacade {
    inner: Vec<Arc<dyn ChannelConnectionFacade>>,
}

impl CompositeChannelConnectionFacade {
    pub(crate) fn new(inner: Vec<Arc<dyn ChannelConnectionFacade>>) -> Self {
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
        // No mounted facade knows this channel identifier: invalid client
        // input (404), never an internal host fault (500).
        Err(RebornServicesError::not_found())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use ironclaw_host_api::{TenantId, UserId};
    use ironclaw_product_workflow::RebornServicesErrorCode;

    use super::*;

    #[derive(Debug)]
    struct FakeConnectionFacade {
        channel: &'static str,
        disconnects: AtomicUsize,
    }

    impl FakeConnectionFacade {
        fn new(channel: &'static str) -> Self {
            Self {
                channel,
                disconnects: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl ChannelConnectionFacade for FakeConnectionFacade {
        async fn caller_channel_connections(
            &self,
            _caller: WebUiAuthenticatedCaller,
        ) -> Result<HashMap<String, bool>, RebornServicesError> {
            Ok(HashMap::from([(self.channel.to_string(), true)]))
        }

        async fn disconnect_channel_for_caller(
            &self,
            _caller: WebUiAuthenticatedCaller,
            _channel: &str,
        ) -> Result<(), RebornServicesError> {
            self.disconnects.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn caller() -> WebUiAuthenticatedCaller {
        WebUiAuthenticatedCaller::new(
            TenantId::new("tenant-a").expect("tenant"),
            UserId::new("user-a").expect("user"),
            None,
            None,
        )
    }

    #[tokio::test]
    async fn disconnect_routes_to_the_owning_facade_and_unknown_channel_is_not_found() {
        let first = Arc::new(FakeConnectionFacade::new("alpha"));
        let second = Arc::new(FakeConnectionFacade::new("beta"));
        let composite = CompositeChannelConnectionFacade::new(vec![
            Arc::clone(&first) as Arc<dyn ChannelConnectionFacade>,
            Arc::clone(&second) as Arc<dyn ChannelConnectionFacade>,
        ]);

        composite
            .disconnect_channel_for_caller(caller(), "beta")
            .await
            .expect("known channel disconnects");
        assert_eq!(first.disconnects.load(Ordering::SeqCst), 0);
        assert_eq!(second.disconnects.load(Ordering::SeqCst), 1);

        let error = composite
            .disconnect_channel_for_caller(caller(), "nope")
            .await
            .expect_err("unknown channel is rejected");
        assert_eq!(
            error.code,
            RebornServicesErrorCode::NotFound,
            "an unregistered channel identifier is client input, not a host fault"
        );
        assert_eq!(error.status_code, 404);
    }
}
