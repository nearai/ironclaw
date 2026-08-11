//! The per-extension channel workflow factory (§12.11 D-A).
//!
//! The generic channel host reconciles one inbound graph per channel-declaring
//! extension. Everything *before* and *after* the product cone in that graph is
//! host code — the evidence mint, the `[channel.config]` secret and
//! configuration ports, the sink, registration, drain, the pairing interceptor.
//! The product cone itself — the durable idempotency ledger and conversation
//! binding at extension-keyed storage roots, the inbound-turn service, the
//! product surface, the command admission policy, and the run-delivery
//! observer — is `ironclaw_assistant`'s to build, and the host sits *below*
//! product.
//!
//! So the host declares the shape and product supplies it: this factory is
//! injected into `GenericChannelHostDeps`, exactly as the storage-only
//! `ChannelWorkflowStateFactory` it replaces was. What crosses the port is
//! only [`ChannelWorkflowRequest`] in and [`ChannelWorkflowGraph`] out. The
//! durable conversation services the graph is built over — a concrete
//! `ironclaw_conversations` type this crate may not name — stay behind the
//! port entirely; the host never held them for their own sake, only to hand
//! them back to product.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extension_contracts::external::{ExternalConversationRef, ExternalEventId};
use ironclaw_host_api::path::VirtualPath;
use ironclaw_host_api::product_adapter::{AdapterInstallationId, ProductAdapterId};
use ironclaw_host_api::product_adapter_error::ProductAdapterError;

use crate::account_setup::ChannelConnectionNoticePolicy;
use crate::actor_identity::ProductActorUserResolver;
use crate::binding::ProductBindingResolver;
use crate::command::CommandActorRoleResolver;
use crate::inbound::{ProductInboundAck, ProductInboundEnvelope};
use crate::shared_admission::SharedConversationAdmission;
use crate::surface::{ChannelInboundProductSurface, ProductSurface};

/// Extension-keyed durable roots for the per-extension workflow state (the
/// idempotency ledger and the conversation-binding store). Parameterized so
/// a channel whose durable state predates the generic scheme can keep its
/// legacy roots until the one-time key migration folds them in.
///
/// `VirtualPath`-shaped on purpose: the *placement* is the host's policy (it
/// derives the generic scheme from the tenant and extension id, or takes an
/// extension's legacy override), while the *substrate* is product's. That
/// split is what lets this cross the port at all.
#[derive(Debug, Clone)]
pub struct ChannelWorkflowStorageRoots {
    pub idempotency: VirtualPath,
    pub conversations: VirtualPath,
}

/// Everything the host resolves per extension before product can build the
/// graph. Every field is either host-API vocabulary or a product-side port the
/// host implements — nothing here obliges the host to name a product type.
pub struct ChannelWorkflowRequest {
    /// The product adapter identity of this extension.
    pub adapter_id: ProductAdapterId,
    /// The installation the verified ingress route belongs to.
    pub installation_id: AdapterInstallationId,
    /// Durable roots for this extension's ledger and conversation store.
    pub storage_roots: ChannelWorkflowStorageRoots,
    /// Resolves a verified inbound actor to a bound IronClaw user. The host
    /// derives the per-extension identity policy (vendor OAuth, pairing, or
    /// the operator-actor default) and implements the port.
    pub actor_user_resolver: Arc<dyn ProductActorUserResolver>,
    /// Shared-conversation admission. Fail-closed either way: `None` rejects
    /// every shared conversation; `Some` admits exactly the conversations the
    /// resolver says are connected.
    pub shared_admission: Option<Arc<dyn SharedConversationAdmission>>,
    /// The manifest-declared channel commands, in declaration order. An
    /// undeclarable name fails the build, and the route stays unregistered.
    pub commands: Vec<String>,
    /// Resolves the admin-audience role of the actor a command arrived from.
    pub command_roles: Arc<dyn CommandActorRoleResolver>,
    /// The late-bound surface channel commands execute through.
    pub command_surface: Arc<dyn ProductSurface>,
    /// `[channel.presentation].command_prefix`, when the channel declares one.
    pub command_prefix: Option<String>,
    /// The connect/pair notice wording this extension posts.
    pub connection_notices: ChannelConnectionNoticePolicy,
}

/// The built product cone for one extension.
pub struct ChannelWorkflowGraph {
    /// The admission surface the generic inbound sink drives.
    pub surface: Arc<dyn ChannelInboundProductSurface>,
    /// The same binding service the surface resolves through — exposed so a
    /// caller can pre-resolve the binding admission will find.
    pub binding: Arc<dyn ProductBindingResolver>,
    /// The run-delivery watcher, when the composed runtime has a delivery
    /// coordinator. `None` leaves the registration ingress-only: turns run and
    /// nothing watches them for channel replies.
    pub observer: Option<Arc<dyn ChannelRunDeliveryObserver>>,
}

/// The generic run-delivery watcher, as the channel host consumes it.
///
/// The host adapts this onto its own post-admission and pairing-outcome
/// observer seams; the three calls below are the entire surface it uses.
#[async_trait]
pub trait ChannelRunDeliveryObserver: Send + Sync {
    /// Watch the run an accepted inbound message submitted and deliver its
    /// user-visible outputs back to the originating conversation.
    async fn observe_ack(&self, envelope: ProductInboundEnvelope, ack: ProductInboundAck);

    /// The error-path mirror of [`Self::observe_ack`].
    async fn observe_error(&self, envelope: ProductInboundEnvelope, error: ProductAdapterError);

    /// Post a connection-status notice on an external conversation (the
    /// pairing-outcome path).
    async fn post_connection_status_notice(
        &self,
        conversation: &ExternalConversationRef,
        event_id: &ExternalEventId,
        text: &str,
    );
}

/// Builds one extension's product cone over whatever substrate the composed
/// runtime owns.
#[async_trait]
pub trait ChannelWorkflowFactory: Send + Sync {
    /// `Err` is the host's fail-closed reason, logged verbatim by the
    /// reconcile loop and never rendered to a caller — deliberately a plain
    /// string rather than the boundary error, because no membrane image of it
    /// exists: the route is simply not registered.
    async fn build_channel_workflow(
        &self,
        request: ChannelWorkflowRequest,
    ) -> Result<ChannelWorkflowGraph, String>;
}
