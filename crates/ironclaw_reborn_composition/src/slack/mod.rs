#[cfg(feature = "slack-v2-host-beta")]
pub(crate) mod slack_actor_identity;
#[cfg(feature = "slack-v2-host-beta")]
pub(crate) mod slack_channel_connection;
#[cfg(feature = "slack-v2-host-beta")]
pub(crate) mod slack_channel_routes;
#[cfg(feature = "slack-v2-host-beta")]
pub(crate) mod slack_connectable_channel;
// Compiled for the Telegram host too: the final-reply delivery observer and
// its services bundle are adapter-generic machinery (adapter/egress/sink are
// injected), reused by the Telegram host pending a vendor-neutral rename in
// the #6116 fold.
#[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
pub(crate) mod slack_delivery;
#[cfg(feature = "slack-v2-host-beta")]
pub(crate) mod slack_dm_open;
#[cfg(feature = "slack-v2-host-beta")]
pub(crate) mod slack_egress;
#[cfg(feature = "slack-v2-host-beta")]
pub(crate) mod slack_host_beta;
#[cfg(feature = "slack-v2-host-beta")]
pub(crate) mod slack_host_state;
#[cfg(feature = "slack-v2-host-beta")]
pub(crate) mod slack_outbound_targets;
#[cfg(feature = "slack-v2-host-beta")]
pub(crate) mod slack_personal_binding;
#[cfg(feature = "slack-v2-host-beta")]
pub(crate) mod slack_personal_oauth;
#[cfg(feature = "slack-v2-host-beta")]
pub mod slack_serve;
#[cfg(feature = "slack-v2-host-beta")]
pub(crate) mod slack_setup;
