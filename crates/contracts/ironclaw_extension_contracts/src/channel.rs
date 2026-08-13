//! Channel-surface declaration vocabulary (`[channel]` in a v3 manifest).
//!
//! One extension declares at most one channel surface. The host consumes the
//! descriptor everywhere — ingress routing, conversation
//! binding, presentation policy — so the vocabulary lives in the contracts
//! crate; adapters implement behavior only and are never asked for metadata.

use serde::{Deserialize, Serialize};

use ironclaw_host_api::{
    action::NetworkScheme,
    error::HostApiError,
    ids::{SecretHandle, VendorId},
};

use crate::recipe::{IngressVerificationRecipe, RecipeValidationError};

const MAX_CHANNEL_COMMANDS: usize = 32;
const MAX_CHANNEL_COMMAND_NAME_BYTES: usize = 64;
const MAX_CHANNEL_COMMAND_PREFIX_BYTES: usize = 32;

/// How external conversations map to IronClaw conversations
/// (`docs/internal/reborn/extension-runtime/overview.md` §3). The host WebUI's
/// internal channel uses the same enum, so the workflow reasons about every
/// channel one way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationModel {
    /// The protocol supplies conversation identity; each external
    /// conversation is one ongoing IronClaw conversation, bound per external
    /// conversation ref.
    Continuous,
    /// The client explicitly creates and switches isolated conversations.
    Isolated,
}

/// One URL-safe path segment appended to
/// `/webhooks/extensions/{extension_id}/` for a channel's ingress route.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RouteSuffix(String);

impl RouteSuffix {
    pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
        let value = value.into();
        Self::validate(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn validate(value: &str) -> Result<(), HostApiError> {
        let invalid = |reason: &str| HostApiError::InvalidId {
            kind: "route_suffix",
            value: value.to_string(),
            reason: reason.to_string(),
        };
        if value.is_empty() {
            return Err(invalid("must not be empty"));
        }
        if value.len() > 64 {
            return Err(invalid("must be at most 64 bytes"));
        }
        if !value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            return Err(invalid(
                "must be one URL-safe segment: lowercase ASCII letters, digits, '-', '_'",
            ));
        }
        Ok(())
    }
}

impl std::fmt::Display for RouteSuffix {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl serde::Serialize for RouteSuffix {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> serde::Deserialize<'de> for RouteSuffix {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// How a run's answer gets back to where the input came from — the **reply**
/// axis (`[channel.reply] transport`).
///
/// Deliberately a separate type from [`DeliveryTransport`]: `Stream` is
/// meaningless for delivery and `Push` is meaningless for reply, so two enums
/// make those nonsense combinations unrepresentable. `Message` appearing in
/// both is not duplication — it is the observation that for a conversational
/// vendor the two axes happen to share a mechanism, which is exactly why the
/// distinction stayed invisible until a streaming channel existed.
///
/// A third transport (a channel that streams but cannot subscribe itself, so
/// the host pushes chunks into it) would join here as a new variant rather
/// than forcing a reshape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyTransport {
    /// The answer streams to a subscribed client over the durable projection
    /// pipeline. The host publishes; the channel's adapter is never called for
    /// a reply, which is why such a channel implements no reply half at all.
    Stream,
    /// The answer is sent as one or more channel messages. The package owns
    /// provider-specific rendering and splitting because limits may use
    /// transport-specific units such as UTF-16 code units.
    Message,
}

/// How we reach someone out of band — the **delivery** axis
/// (`[channel.delivery] transport`). See [`ReplyTransport`] for why these are
/// two types rather than one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryTransport {
    /// A push notification to the user's enrolled clients.
    Push,
    /// A message in a resolved target conversation.
    Message,
}

/// The `[channel.reply]` section: how a run's answer gets back to its source.
///
/// **Absence means the channel has no reply half** — it cannot answer a run's
/// input at all (a notification-only channel), or its replies are published by
/// the host rather than sent by the adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelReplyDescriptor {
    pub transport: ReplyTransport,
}

/// The `[channel.delivery]` section: how we reach the user outside a run.
///
/// **Absence means the channel cannot be an out-of-band delivery target** —
/// it is reply-only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelDeliveryDescriptor {
    pub transport: DeliveryTransport,
    /// Whether a per-user registration must exist before this channel can
    /// deliver. The host owns those registrations and resolves a channel with
    /// zero of them to a "no target" outcome before any adapter call.
    #[serde(default)]
    pub requires_enrollment: bool,
}

/// The declared channel surface of one extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelDescriptor {
    /// Channel surface id within the extension (e.g. `messages`).
    pub id: String,
    pub display_name: String,
    /// Required: how external conversations bind (checklist MAN-10).
    pub conversation_model: ConversationModel,
    /// Exact product command tokens exposed by this channel, without a leading
    /// slash. Missing and empty declarations expose no product commands.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingress: Option<ChannelIngressDescriptor>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub egress: Vec<ChannelEgressDescriptor>,
    #[serde(default)]
    pub presentation: ChannelPresentation,
    /// The reply axis (`[channel.reply]`): how this run's answer gets back to
    /// where the input came from. Source-routed; never exists without a run.
    /// Absent means this channel has no reply half.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<ChannelReplyDescriptor>,
    /// The delivery axis (`[channel.delivery]`): how we reach the user out of
    /// band. Target-resolved; may exist with no run at all. Absent means this
    /// channel is not an out-of-band delivery target.
    ///
    /// The two are **orthogonal, not alternatives** — one run can stream an
    /// answer into an open tab (reply) *and* fire a push because the user is
    /// not looking (delivery).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<ChannelDeliveryDescriptor>,
    /// User-account connection behavior for this channel. This declaration is
    /// the only authority for pairing presentation and connection notices;
    /// hosts must not infer a recipe from an extension id or display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<ChannelConnectionDescriptor>,
}

/// Private read shape for the one in-place v3 channel evolution.
///
/// The public descriptor exposes only the current section-based axes. The
/// immediately preceding v3 shape used `inbound` / `outbound` /
/// `notifications` booleans and placed `max_message_chars` under
/// presentation; persisted resolved manifests may still contain those bytes.
/// Keeping the compatibility fields private prevents the retired vocabulary
/// from becoming authoring API again while allowing a rolling deployment to
/// read its own earlier records.
#[derive(Deserialize)]
struct ChannelDescriptorWire {
    id: String,
    display_name: String,
    conversation_model: ConversationModel,
    #[serde(default)]
    commands: Vec<String>,
    #[serde(default)]
    ingress: Option<ChannelIngressDescriptor>,
    #[serde(default)]
    egress: Vec<ChannelEgressDescriptor>,
    #[serde(default)]
    presentation: ChannelPresentationWire,
    #[serde(default)]
    reply: Option<ChannelReplyDescriptor>,
    #[serde(default)]
    delivery: Option<ChannelDeliveryDescriptor>,
    #[serde(default)]
    connection: Option<ChannelConnectionDescriptor>,
    #[serde(default)]
    inbound: Option<bool>,
    #[serde(default)]
    outbound: Option<bool>,
    #[serde(default)]
    notifications: Option<bool>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChannelPresentationWire {
    #[serde(default)]
    supports_markdown: bool,
    #[serde(default)]
    supports_threads: bool,
    #[serde(default)]
    can_reply_in_threads: bool,
    #[serde(default)]
    command_prefix: Option<String>,
    #[serde(default)]
    #[serde(rename = "max_message_chars")]
    _max_message_chars: Option<u32>,
}

impl<'de> Deserialize<'de> for ChannelDescriptor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ChannelDescriptorWire::deserialize(deserializer)?;
        if wire.inbound == Some(true) && wire.ingress.is_none() {
            return Err(serde::de::Error::custom(
                "legacy inbound = true requires [channel.ingress]",
            ));
        }

        let legacy_outbound = wire.outbound == Some(true);
        let legacy_notification = wire.notifications == Some(true);
        let mut reply = wire.reply;
        let mut delivery = wire.delivery;

        // This is deliberately the exact immediately preceding mapping, not a
        // general versioned migration engine. In that deployed shape,
        // conversational outbound channels were message reply + message
        // delivery, while the sole `notifications = true` shape was the
        // enrollment-backed push channel and had no reply half.
        if legacy_outbound && legacy_notification {
            delivery.get_or_insert(ChannelDeliveryDescriptor {
                transport: DeliveryTransport::Push,
                requires_enrollment: true,
            });
        } else if legacy_outbound {
            reply.get_or_insert(ChannelReplyDescriptor {
                transport: ReplyTransport::Message,
            });
            delivery.get_or_insert(ChannelDeliveryDescriptor {
                transport: DeliveryTransport::Message,
                requires_enrollment: false,
            });
        }
        Ok(Self {
            id: wire.id,
            display_name: wire.display_name,
            conversation_model: wire.conversation_model,
            commands: wire.commands,
            ingress: wire.ingress,
            egress: wire.egress,
            presentation: ChannelPresentation {
                supports_markdown: wire.presentation.supports_markdown,
                supports_threads: wire.presentation.supports_threads,
                can_reply_in_threads: wire.presentation.can_reply_in_threads,
                command_prefix: wire.presentation.command_prefix,
            },
            reply,
            delivery,
            connection: wire.connection,
        })
    }
}

impl ChannelDescriptor {
    /// Whether this channel accepts input. **Presence of `[channel.ingress]`
    /// is the declaration** — there is no separate `inbound` boolean saying
    /// *that* a channel does something without saying *how*.
    pub fn supports_inbound(&self) -> bool {
        self.ingress.is_some()
    }

    /// Whether this channel can answer a run's input (the reply axis).
    pub fn supports_reply(&self) -> bool {
        self.reply.is_some()
    }

    /// Whether this channel can be reached out of band (the delivery axis).
    pub fn supports_delivery(&self) -> bool {
        self.delivery.is_some()
    }

    /// Whether this channel emits anything at all. Kept as one predicate
    /// because a few host paths genuinely mean "either axis" (does this
    /// channel need egress wiring at all); anything deciding *how* to send
    /// must ask the axes separately.
    pub fn supports_outbound(&self) -> bool {
        self.supports_reply() || self.supports_delivery()
    }

    pub fn reply_transport(&self) -> Option<ReplyTransport> {
        self.reply.as_ref().map(|reply| reply.transport)
    }

    pub fn delivery_transport(&self) -> Option<DeliveryTransport> {
        self.delivery.as_ref().map(|delivery| delivery.transport)
    }

    /// Whether delivery needs a per-user registration first. False when the
    /// channel has no delivery half at all — nothing to enroll for.
    pub fn requires_enrollment(&self) -> bool {
        self.delivery
            .as_ref()
            .is_some_and(|delivery| delivery.requires_enrollment)
    }

    /// Structural validation beyond field-level deserialization.
    pub fn validate(&self) -> Result<(), ChannelDescriptorError> {
        if self.id.trim().is_empty() {
            return Err(ChannelDescriptorError::EmptyId);
        }
        if self.display_name.trim().is_empty() {
            return Err(ChannelDescriptorError::EmptyDisplayName);
        }
        if self.commands.len() > MAX_CHANNEL_COMMANDS {
            return Err(ChannelDescriptorError::InvalidCommands);
        }
        let mut seen_commands: Vec<&str> = Vec::with_capacity(self.commands.len());
        for command in &self.commands {
            if command.is_empty()
                || command.len() > MAX_CHANNEL_COMMAND_NAME_BYTES
                || !command
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
                || seen_commands.contains(&command.as_str())
            {
                return Err(ChannelDescriptorError::InvalidCommands);
            }
            seen_commands.push(command);
        }
        if let Some(prefix) = &self.presentation.command_prefix
            && (prefix.is_empty()
                || !prefix.starts_with('/')
                || prefix.len() > MAX_CHANNEL_COMMAND_PREFIX_BYTES
                || prefix.chars().any(char::is_control))
        {
            return Err(ChannelDescriptorError::InvalidCommandPrefix);
        }
        if self.connection.is_some() && self.ingress.is_none() {
            return Err(ChannelDescriptorError::ConnectionWithoutInbound);
        }
        // A `stream` reply forwards the durable projection stream over the
        // session transport. A webhook vendor has no such stream to consume,
        // and a channel with no ingress has no run whose answer to stream —
        // so the declared reply transport must pair with the entrypoint.
        // Checked outside the ingress block below so the no-ingress case is
        // rejected too, not silently skipped.
        if self.reply_transport() == Some(ReplyTransport::Stream)
            && !self
                .ingress
                .as_ref()
                .is_some_and(|ingress| ingress.verification.is_authenticated_session())
        {
            return Err(ChannelDescriptorError::StreamingReplyWithoutSessionIngress);
        }
        if let Some(connection) = &self.connection {
            connection.validate()?;
        }
        if let Some(ingress) = &self.ingress {
            ingress
                .verification
                .validate()
                .map_err(ChannelDescriptorError::Verification)?;
            // Pair the ingress trust class with the mount: authenticated_session
            // ingress is verified upstream by the host transport (T1) and mounts
            // no webhook route, so it must NOT carry a route_suffix; every
            // webhook recipe (T2) MUST, or there is no route to receive on.
            match (
                ingress.verification.is_authenticated_session(),
                ingress.route_suffix.is_some(),
            ) {
                (true, true) => {
                    return Err(ChannelDescriptorError::SessionIngressWithRouteSuffix);
                }
                (false, false) => {
                    return Err(ChannelDescriptorError::WebhookIngressWithoutRouteSuffix);
                }
                _ => {}
            }
        }
        for egress in &self.egress {
            if egress.host.trim().is_empty() || egress.host.contains('*') {
                return Err(ChannelDescriptorError::WildcardOrEmptyEgressHost {
                    host: egress.host.clone(),
                });
            }
            if let Some(injection) = &egress.injection {
                if egress.credential_handle.is_none() {
                    return Err(ChannelDescriptorError::EgressInjectionWithoutCredential {
                        host: egress.host.clone(),
                    });
                }
                if injection.validate_declaration().is_err() {
                    return Err(ChannelDescriptorError::InvalidEgressInjection {
                        host: egress.host.clone(),
                    });
                }
            }
            let mut seen_body_handles: Vec<&str> = Vec::new();
            for body_credential in &egress.body_credentials {
                if !body_credential.pointer.starts_with('/')
                    || seen_body_handles.contains(&body_credential.handle.as_str())
                {
                    return Err(ChannelDescriptorError::InvalidEgressInjection {
                        host: egress.host.clone(),
                    });
                }
                seen_body_handles.push(body_credential.handle.as_str());
            }
            for path in egress.paths.iter().chain(&egress.path_prefixes) {
                if !valid_egress_path_constraint(path, egress.injection.as_ref()) {
                    return Err(ChannelDescriptorError::InvalidEgressConstraint {
                        host: egress.host.clone(),
                    });
                }
            }
            // A prefix must end on a segment boundary. Without this, prefix
            // matching is a raw byte comparison, so a declaration like
            // `/file/bot{token}` would also authorize `/file/bot{token}Evil/…`
            // — a sibling path the manifest author never allowed on this
            // pinned host + credential.
            for prefix in &egress.path_prefixes {
                if !prefix.ends_with('/') {
                    return Err(ChannelDescriptorError::InvalidEgressConstraint {
                        host: egress.host.clone(),
                    });
                }
            }
            if egress
                .request_body_limit_bytes
                .is_some_and(|limit| limit > MAX_CHANNEL_EGRESS_TRANSFER_BYTES)
                || egress
                    .response_body_limit_bytes
                    .is_some_and(|limit| limit == 0 || limit > MAX_CHANNEL_EGRESS_TRANSFER_BYTES)
            {
                return Err(ChannelDescriptorError::InvalidEgressConstraint {
                    host: egress.host.clone(),
                });
            }
        }
        Ok(())
    }
}

/// Manifest-declared user connection strategy for an inbound channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelConnectionStrategy {
    AdminManagedChannels,
    WebGeneratedCode,
    #[serde(rename = "oauth", alias = "o_auth")]
    OAuth,
}

/// Manifest-owned copy shown while a user connects an inbound channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelConnectionDescriptor {
    /// Provider key used by the per-user connection gate and continuation
    /// fan-out. It is data, not an extension-id convention.
    pub provider: VendorId,
    pub strategy: ChannelConnectionStrategy,
    pub instructions: String,
    #[serde(default)]
    pub input_placeholder: String,
    pub submit_label: String,
    pub error_message: String,
    pub notices: ChannelConnectionNotices,
    #[serde(alias = "activation_success_message")]
    pub connection_success_message: String,
    /// Optional deep-link template. `{code}` is replaced with the host-minted
    /// proof and other placeholders resolve from non-secret administrator
    /// configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deep_link_template: Option<String>,
    /// Exact message prefixes the generic inbound pairing parser may strip
    /// before validating a proof code (for example, `/start`). Bare codes are
    /// always accepted; an empty list grants no command-shaped syntax.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inbound_code_prefixes: Vec<String>,
}

impl ChannelConnectionDescriptor {
    const MAX_INBOUND_CODE_PREFIXES: usize = 8;
    const MAX_INBOUND_CODE_PREFIX_BYTES: usize = 32;

    fn validate(&self) -> Result<(), ChannelDescriptorError> {
        for (field, value) in [
            ("instructions", self.instructions.as_str()),
            ("submit_label", self.submit_label.as_str()),
            ("error_message", self.error_message.as_str()),
            (
                "connection_success_message",
                self.connection_success_message.as_str(),
            ),
            (
                "notices.connect_required",
                self.notices.connect_required.as_str(),
            ),
            ("notices.paired", self.notices.paired.as_str()),
            (
                "notices.already_paired_same_user",
                self.notices.already_paired_same_user.as_str(),
            ),
            (
                "notices.already_bound_to_other_user",
                self.notices.already_bound_to_other_user.as_str(),
            ),
            (
                "notices.expired_or_unknown",
                self.notices.expired_or_unknown.as_str(),
            ),
        ] {
            if value.trim().is_empty() {
                return Err(ChannelDescriptorError::EmptyConnectionField { field });
            }
        }
        if let Some(template) = &self.deep_link_template
            && (!template.contains("{code}")
                || !matches!(self.strategy, ChannelConnectionStrategy::WebGeneratedCode))
        {
            return Err(ChannelDescriptorError::InvalidConnectionDeepLink);
        }
        if !self.inbound_code_prefixes.is_empty()
            && (!matches!(self.strategy, ChannelConnectionStrategy::WebGeneratedCode)
                || self.inbound_code_prefixes.len() > Self::MAX_INBOUND_CODE_PREFIXES)
        {
            return Err(ChannelDescriptorError::InvalidConnectionCodePrefixes);
        }
        let mut seen_prefixes: Vec<&str> = Vec::new();
        for prefix in &self.inbound_code_prefixes {
            if prefix.is_empty()
                || prefix.len() > Self::MAX_INBOUND_CODE_PREFIX_BYTES
                || prefix.trim() != prefix
                || prefix.chars().any(char::is_whitespace)
                || prefix.chars().any(char::is_control)
                || seen_prefixes.contains(&prefix.as_str())
            {
                return Err(ChannelDescriptorError::InvalidConnectionCodePrefixes);
            }
            seen_prefixes.push(prefix);
        }
        Ok(())
    }
}

/// Manifest-owned notices emitted by the generic pairing and ingress paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelConnectionNotices {
    pub connect_required: String,
    pub paired: String,
    pub already_paired_same_user: String,
    pub already_bound_to_other_user: String,
    pub expired_or_unknown: String,
}

/// One declarative vendor call the host runs on the channel's behalf.
///
/// This is the shape that replaced `ChannelAdapter::activate`/`cleanup` and
/// the attachment fetch: **per-channel data, generic execution**. The host
/// substitutes `{handle}` placeholders from the installation's non-secret
/// config, then runs the call through the same restricted egress and the same
/// host-side credential injection an adapter send uses — so a manifest field
/// cannot drift from an implementation, because there is no implementation.
///
/// Placeholders the host does **not** find in config are left in place for
/// the egress layer, which is how a credential-in-path vendor works: the
/// manifest's `injection = { type = "path_placeholder" }` substitutes the
/// secret host-side and the bytes never enter adapter scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelVendorCallRecipe {
    #[serde(default)]
    pub method: ChannelVendorCallMethod,
    /// URL path on the channel's declared egress host, `{handle}`-templated.
    pub path: String,
    /// JSON body template. String values are `{handle}`-substituted from
    /// non-secret config; secrets are never templated here — they ride
    /// `body_credentials` on the egress target, which inserts the resolved
    /// value host-side at a declared JSON pointer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
    /// Secret handles the host may inject into this call's body, each at the
    /// pointer its `[[channel.egress]] body_credentials` entry declares.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub body_credentials: Vec<SecretHandle>,
}

/// HTTP methods a declarative vendor call may use.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelVendorCallMethod {
    #[default]
    Post,
    Get,
}

/// Ingress declaration for an inbound channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelIngressDescriptor {
    /// Idempotent vendor-side wiring run at activation — telling the vendor
    /// where to POST. Every input is already known to the host (it owns the
    /// webhook route and therefore the URL), which is why this is a recipe
    /// and not a method. Absent means no registration is needed: a vendor
    /// whose events URL is configured in its own app console, or a channel
    /// with no webhook at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registration: Option<ChannelVendorCallRecipe>,
    /// Idempotent, best-effort vendor-side unwiring run at deactivation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deregistration: Option<ChannelVendorCallRecipe>,
    /// Present for webhook ingress (the mounted route's last path segment);
    /// absent for `authenticated_session` ingress, which mounts no webhook
    /// route. The pairing is enforced by [`ChannelDescriptor::validate`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_suffix: Option<RouteSuffix>,
    #[serde(default)]
    pub method: ChannelIngressMethod,
    #[serde(default = "default_body_limit_bytes")]
    pub body_limit_bytes: u64,
    /// Required and explicit — `kind = "none"` must be declared, never
    /// defaulted.
    pub verification: IngressVerificationRecipe,
}

/// Webhook ingress methods the generic router accepts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelIngressMethod {
    #[default]
    Post,
}

fn default_body_limit_bytes() -> u64 {
    1_048_576
}

/// One declared egress target for the channel adapter's vendor calls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelEgressDescriptor {
    #[serde(default = "default_https")]
    pub scheme: NetworkScheme,
    pub host: String,
    pub methods: Vec<ironclaw_host_api::action::NetworkMethod>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_handle: Option<SecretHandle>,
    /// How the host injects the declared credential into vendor requests.
    /// Absent means the default `Authorization: Bearer <secret>` header.
    /// `path_placeholder` covers vendors that carry the credential in the URL
    /// path (the adapter writes `{placeholder}` into the path; the host
    /// substitutes the secret — bytes never reach the adapter).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub injection: Option<ironclaw_host_api::http::RuntimeCredentialTarget>,
    /// Body credentials the host may inject for this target: each entry binds
    /// a secret handle to the RFC 6901 JSON pointer where its resolved value
    /// is inserted in the request's JSON body (e.g. a vendor
    /// webhook-registration call whose API takes the shared secret as a body
    /// field). The manifest is the sole authority for the placement; adapters
    /// opt in per request by naming the handle and never see bytes. Empty
    /// means no body credential may be injected for this target.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub body_credentials: Vec<ChannelBodyCredentialDescriptor>,
    /// Exact URL paths this target permits. Empty preserves the legacy
    /// host+method-only policy; first-party manifests should declare paths.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    /// URL path prefixes this target permits for provider-generated suffixes
    /// such as file download paths. Prefix matching is explicit and distinct
    /// from exact [`Self::paths`] matching.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path_prefixes: Vec<String>,
    /// Maximum request body size for this target. `None` preserves the
    /// pre-v3-declaration behavior; declared limits are enforced both before
    /// approval and after host-side credential injection, before I/O.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_body_limit_bytes: Option<u64>,
    /// Maximum response body size for this target. The channel host also
    /// clamps declarations to its global safety ceiling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_body_limit_bytes: Option<u64>,
}

/// Maximum request or response body bound a channel manifest may request.
/// This is an authority ceiling, not the default; each target may declare a
/// narrower value.
pub const MAX_CHANNEL_EGRESS_TRANSFER_BYTES: u64 = 10 * 1024 * 1024;

fn valid_egress_path_constraint(
    path: &str,
    injection: Option<&ironclaw_host_api::http::RuntimeCredentialTarget>,
) -> bool {
    if path.is_empty()
        || path.len() > 2_048
        || !path.starts_with('/')
        || path.starts_with("//")
        || path.contains("://")
        || path.contains(['?', '#', '\\', '%'])
        || path.chars().any(|character| character.is_control())
        || path.split('/').any(|segment| matches!(segment, "." | ".."))
        || !path.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, '/' | '.' | '_' | '-' | '{' | '}')
        })
    {
        return false;
    }
    match injection {
        Some(ironclaw_host_api::http::RuntimeCredentialTarget::PathPlaceholder { placeholder }) => {
            let marker = format!("{{{placeholder}}}");
            let without_marker = path.replace(&marker, "");
            !without_marker.contains(['{', '}'])
        }
        _ => !path.contains(['{', '}']),
    }
}

/// One declared body-credential binding on a channel egress target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelBodyCredentialDescriptor {
    pub handle: SecretHandle,
    /// RFC 6901 JSON pointer naming where the resolved secret value is
    /// inserted in the request's JSON body (must start with `/`).
    pub pointer: String,
}

fn default_https() -> NetworkScheme {
    NetworkScheme::Https
}

/// Presentation facts prompt construction consumes.
///
/// The `ironclaw_llm` policy type that used to derive from this went with the
/// v1 reasoning engine in the WS8 dead-surface sweep; these facts are the
/// surviving source of truth.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelPresentation {
    #[serde(default)]
    pub supports_markdown: bool,
    #[serde(default)]
    pub supports_threads: bool,
    /// Whether the channel replies to a shared-conversation message by
    /// creating/continuing a vendor-side thread anchored on it, as opposed
    /// to an inline anchored reply in the flat conversation. Declares the
    /// reply-placement contract; the per-vendor anchor mechanics (a thread
    /// root id vs. a reply-to-message id) live in each channel package.
    #[serde(default)]
    pub can_reply_in_threads: bool,
    /// Optional per-command display prefix a channel adapter renders before
    /// each declared command name in user-visible help text (e.g. a channel
    /// whose native command namespace requires an app-scoped dispatcher
    /// prefix: `"/ironclaw "` + `model` -> `/ironclaw model`). `None`
    /// renders the bare `/{name}` form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_prefix: Option<String>,
}

/// Structural channel-descriptor failures (path context added by the
/// manifest parser).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChannelDescriptorError {
    #[error("channel id must not be empty")]
    EmptyId,
    #[error("channel display_name must not be empty")]
    EmptyDisplayName,
    #[error(
        "channel commands must contain at most 32 unique tokens of at most 64 bytes using lowercase ASCII letters, digits, '-', or '_'"
    )]
    InvalidCommands,
    #[error(
        "channel presentation command_prefix must be non-empty, start with '/', contain no control characters, and be at most 32 bytes"
    )]
    InvalidCommandPrefix,
    // `InboundWithoutIngress` is gone: `[channel.ingress]` IS the inbound
    // declaration, so the contradiction it named — claiming inbound without
    // declaring how input arrives — is now unrepresentable.
    #[error("[channel.connection] requires an inbound channel ([channel.ingress])")]
    ConnectionWithoutInbound,
    #[error("channel connection field `{field}` must not be empty")]
    EmptyConnectionField { field: &'static str },
    #[error(
        "channel connection deep_link_template requires a generated-code strategy and a {{code}} placeholder"
    )]
    InvalidConnectionDeepLink,
    #[error(
        "channel connection inbound_code_prefixes requires web_generated_code and at most 8 unique non-whitespace prefixes of at most 32 bytes"
    )]
    InvalidConnectionCodePrefixes,
    #[error(transparent)]
    Verification(RecipeValidationError),
    #[error("egress target `{host}` declares an injection but no credential_handle")]
    EgressInjectionWithoutCredential { host: String },
    #[error("egress target `{host}` declares a malformed credential injection")]
    InvalidEgressInjection { host: String },
    #[error("egress host `{host}` must be a literal, non-empty host (no wildcards)")]
    WildcardOrEmptyEgressHost { host: String },
    #[error("egress target `{host}` declares an invalid path or transfer bound")]
    InvalidEgressConstraint { host: String },
    #[error(
        "authenticated_session ingress mounts no webhook route and must not declare a route_suffix"
    )]
    SessionIngressWithRouteSuffix,
    #[error("webhook ingress must declare a route_suffix to mount its receiving route")]
    WebhookIngressWithoutRouteSuffix,
    #[error(
        "streaming reply mode requires an authenticated-session entrypoint; webhook channels batch"
    )]
    StreamingReplyWithoutSessionIngress,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_reply_transport_requires_the_session_entrypoint() {
        let mut channel: ChannelDescriptor =
            toml::from_str(documented_channel_toml()).expect("documented channel deserializes");
        assert_eq!(channel.reply_transport(), Some(ReplyTransport::Message));
        channel
            .reply
            .as_mut()
            .expect("documented channel declares a reply half")
            .transport = ReplyTransport::Stream;
        assert!(
            matches!(
                channel.validate(),
                Err(ChannelDescriptorError::StreamingReplyWithoutSessionIngress)
            ),
            "a webhook channel must not declare a stream reply transport"
        );

        let mut session = channel.clone();
        if let Some(ingress) = session.ingress.as_mut() {
            ingress.route_suffix = None;
            ingress.verification = IngressVerificationRecipe::AuthenticatedSession;
        }
        session
            .validate()
            .expect("a stream reply over the session entrypoint validates");

        // A channel with no ingress has no run whose answer to stream. This
        // arm is why the check sits outside the `if let Some(ingress)` block:
        // with it inside, a no-ingress stream declaration validated silently.
        let mut ingressless = session.clone();
        ingressless.ingress = None;
        ingressless.connection = None;
        assert!(
            matches!(
                ingressless.validate(),
                Err(ChannelDescriptorError::StreamingReplyWithoutSessionIngress)
            ),
            "a stream reply with no entrypoint must fail closed"
        );
    }

    #[test]
    fn absent_sections_mean_the_axis_is_unsupported() {
        // The whole point of §2: presence of a section is the declaration.
        // A channel that declares neither reply nor delivery emits nothing,
        // and one that declares no ingress accepts nothing — all valid
        // shapes, none of them expressible as a boolean that says *that* a
        // channel does something without saying *how*.
        let channel: ChannelDescriptor = toml::from_str(
            r#"
id = "messages"
display_name = "Vendor messages"
conversation_model = "continuous"
"#,
        )
        .expect("a bare channel deserializes");
        channel.validate().expect("a bare channel is valid");

        assert!(!channel.supports_inbound());
        assert!(!channel.supports_reply());
        assert!(!channel.supports_delivery());
        assert!(!channel.supports_outbound());
        assert!(!channel.requires_enrollment());
        assert_eq!(channel.reply_transport(), None);
        assert_eq!(channel.delivery_transport(), None);
    }

    #[test]
    fn reply_axis_tolerates_retired_non_security_metadata() {
        let channel: ChannelDescriptor = toml::from_str(
            r#"
id = "messages"
display_name = "Vendor messages"
conversation_model = "continuous"

[reply]
transport = "message"
obsolete_split_hint = 4096
"#,
        )
        .expect("non-security reply metadata evolves in place");

        assert_eq!(channel.reply_transport(), Some(ReplyTransport::Message));
    }

    #[test]
    fn the_two_axes_are_orthogonal_not_alternatives() {
        // One channel can stream a reply into an open tab AND push out of
        // band. Expressing that was the reason for the split.
        let channel: ChannelDescriptor =
            toml::from_str(&session_channel_toml(false)).expect("parse session channel");
        channel.validate().expect("both axes together validate");

        assert_eq!(channel.reply_transport(), Some(ReplyTransport::Stream));
        assert_eq!(channel.delivery_transport(), Some(DeliveryTransport::Push));
        assert!(channel.requires_enrollment());
    }

    #[test]
    fn immediately_preceding_message_channel_shape_normalizes_to_both_message_axes() {
        let channel: ChannelDescriptor = toml::from_str(
            r#"
id = "messages"
display_name = "Legacy messages"
inbound = true
outbound = true
conversation_model = "continuous"

[ingress]
route_suffix = "events"

[ingress.verification]
kind = "shared_secret_header"
secret_handle = "legacy_secret"
header = "X-Legacy-Secret"

[presentation]
supports_markdown = true
max_message_chars = 4096
"#,
        )
        .expect("the immediately preceding v3 channel shape stays readable");

        channel
            .validate()
            .expect("legacy channel normalizes validly");
        assert_eq!(channel.reply_transport(), Some(ReplyTransport::Message));
        assert_eq!(
            channel.delivery_transport(),
            Some(DeliveryTransport::Message)
        );
    }

    #[test]
    fn immediately_preceding_notification_channel_shape_normalizes_to_push_delivery_only() {
        let channel: ChannelDescriptor = toml::from_str(
            r#"
id = "notifications"
display_name = "Legacy notifications"
inbound = false
outbound = true
notifications = true
conversation_model = "continuous"

[presentation]
supports_markdown = false
max_message_chars = 1500
"#,
        )
        .expect("the immediately preceding notification shape stays readable");

        channel
            .validate()
            .expect("legacy notification normalizes validly");
        assert_eq!(channel.reply_transport(), None);
        assert_eq!(channel.delivery_transport(), Some(DeliveryTransport::Push));
        assert!(channel.requires_enrollment());
    }

    fn documented_channel_toml() -> &'static str {
        r#"
id = "messages"
display_name = "Vendor messages"
conversation_model = "continuous"

[reply]
transport = "message"
max_message_chars = 40000

[delivery]
transport = "message"

[ingress]
route_suffix = "events"
method = "post"
body_limit_bytes = 1048576

[ingress.verification]
kind = "hmac_sha256"
secret_handle = "vendor_signing_secret"
signature_header = "X-Vendor-Signature"
signature_prefix = "v0="
signature_encoding = "hex"
timestamp_header = "X-Vendor-Request-Timestamp"
max_age_seconds = 300
signed_payload = [
  { literal = "v0:" },
  { header = "X-Vendor-Request-Timestamp" },
  { literal = ":" },
  { body = true },
]

[[egress]]
scheme = "https"
host = "vendor.example"
methods = ["post"]
credential_handle = "vendor_bot_token"

[presentation]
supports_markdown = true
supports_threads = true
can_reply_in_threads = true
"#
    }

    fn generated_code_connection_toml(prefixes: &str) -> String {
        format!(
            "{}\n\n[connection]\nprovider = \"vendor\"\nstrategy = \"web_generated_code\"\ninstructions = \"Send the displayed code.\"\nsubmit_label = \"Connect\"\nerror_message = \"Pairing failed.\"\nconnection_success_message = \"Connected.\"\ndeep_link_template = \"https://vendor.example/connect?code={{code}}\"\ninbound_code_prefixes = {prefixes}\n\n[connection.notices]\nconnect_required = \"Connect first.\"\npaired = \"Connected.\"\nalready_paired_same_user = \"Already connected.\"\nalready_bound_to_other_user = \"Connected elsewhere.\"\nexpired_or_unknown = \"Invalid code.\"\n",
            documented_channel_toml()
        )
    }

    fn channel_toml_with_commands(commands: &str) -> String {
        documented_channel_toml().replace(
            "conversation_model = \"continuous\"\n",
            &format!("conversation_model = \"continuous\"\ncommands = {commands}\n"),
        )
    }

    fn session_channel_toml(with_route_suffix: bool) -> String {
        let route_line = if with_route_suffix {
            "route_suffix = \"push\"\n"
        } else {
            ""
        };
        format!(
            r#"
id = "web-app"
display_name = "Web app"
conversation_model = "continuous"

[reply]
transport = "stream"

[delivery]
transport = "push"
requires_enrollment = true

[ingress]
{route_line}method = "post"

[ingress.verification]
kind = "authenticated_session"
"#
        )
    }

    #[test]
    fn authenticated_session_ingress_is_valid_without_a_route_suffix() {
        let channel: ChannelDescriptor =
            toml::from_str(&session_channel_toml(false)).expect("parse session channel");
        let ingress = channel.ingress.as_ref().expect("ingress declared");
        assert!(ingress.route_suffix.is_none());
        assert!(ingress.verification.is_authenticated_session());
        channel
            .validate()
            .expect("a session channel with no route_suffix is valid");
    }

    #[test]
    fn authenticated_session_ingress_rejects_a_route_suffix() {
        // A session channel is verified upstream (T1) and mounts no webhook
        // route, so declaring one is a contradiction — fail closed.
        let channel: ChannelDescriptor =
            toml::from_str(&session_channel_toml(true)).expect("parse session channel");
        assert_eq!(
            channel.validate().unwrap_err(),
            ChannelDescriptorError::SessionIngressWithRouteSuffix,
        );
    }

    #[test]
    fn webhook_ingress_requires_a_route_suffix() {
        // `documented_channel_toml` is a webhook (hmac) ingress; drop its
        // route_suffix and the mount has nowhere to receive.
        let without_suffix = documented_channel_toml().replace("route_suffix = \"events\"\n", "");
        let channel: ChannelDescriptor =
            toml::from_str(&without_suffix).expect("parse webhook channel without route_suffix");
        assert!(
            channel
                .ingress
                .as_ref()
                .expect("ingress")
                .route_suffix
                .is_none()
        );
        assert_eq!(
            channel.validate().unwrap_err(),
            ChannelDescriptorError::WebhookIngressWithoutRouteSuffix,
        );
    }

    #[test]
    fn channel_commands_are_exact_and_fail_closed_by_default() {
        let missing: ChannelDescriptor = toml::from_str(documented_channel_toml()).unwrap();
        assert!(missing.commands.is_empty());
        missing.validate().unwrap();

        let explicit_empty: ChannelDescriptor =
            toml::from_str(&channel_toml_with_commands("[]")).unwrap();
        assert!(explicit_empty.commands.is_empty());
        explicit_empty.validate().unwrap();

        let declared: ChannelDescriptor =
            toml::from_str(&channel_toml_with_commands("[\"status\"]")).unwrap();
        assert_eq!(declared.commands, ["status"]);
        declared.validate().unwrap();

        let json = serde_json::to_string(&declared).unwrap();
        let round_trip: ChannelDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(round_trip.commands, ["status"]);
    }

    #[test]
    fn channel_commands_validate_shape_bounds_and_uniqueness() {
        let excessive = (0..33)
            .map(|index| format!("command_{index}"))
            .collect::<Vec<_>>();
        let excessive = serde_json::to_string(&excessive).unwrap();
        let oversized = format!("[\"{}\"]", "a".repeat(65));

        for commands in [
            "[\"status\", \"status\"]",
            "[\"\"]",
            "[\"/status\"]",
            "[\"has space\"]",
            "[\"Status\"]",
            oversized.as_str(),
            excessive.as_str(),
        ] {
            let channel: ChannelDescriptor =
                toml::from_str(&channel_toml_with_commands(commands)).unwrap();
            assert_eq!(
                channel.validate().unwrap_err(),
                ChannelDescriptorError::InvalidCommands,
                "expected invalid commands: {commands}"
            );
        }
    }

    fn channel_toml_with_command_prefix(prefix_toml_value: &str) -> String {
        // Anchored on the last `[presentation]` key, not on `max_message_chars`
        // — that bound now lives in `[channel.reply]`, so anchoring there
        // would inject `command_prefix` into the wrong section and
        // `deny_unknown_fields` would reject it for the wrong reason.
        documented_channel_toml().replace(
            "can_reply_in_threads = true\n",
            &format!("can_reply_in_threads = true\ncommand_prefix = {prefix_toml_value}\n"),
        )
    }

    #[test]
    fn command_prefix_is_optional_and_round_trips() {
        let absent: ChannelDescriptor = toml::from_str(documented_channel_toml()).unwrap();
        assert_eq!(absent.presentation.command_prefix, None);
        absent.validate().unwrap();

        let declared: ChannelDescriptor =
            toml::from_str(&channel_toml_with_command_prefix("\"/ironclaw \"")).unwrap();
        assert_eq!(
            declared.presentation.command_prefix.as_deref(),
            Some("/ironclaw ")
        );
        declared.validate().unwrap();

        let json = serde_json::to_string(&declared).unwrap();
        let round_trip: ChannelDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(
            round_trip.presentation.command_prefix.as_deref(),
            Some("/ironclaw ")
        );

        // Unset stays unset on the wire too (skip_serializing_if).
        let absent_json = serde_json::to_string(&absent).unwrap();
        assert!(
            !absent_json.contains("command_prefix"),
            "unset command_prefix must not appear on the wire: {absent_json}"
        );
    }

    #[test]
    fn command_prefix_validation_rejects_malformed_shapes() {
        for bad in [
            "\"\"".to_string(),
            "\"ironclaw \"".to_string(),
            "\"/control\t\"".to_string(),
            format!("\"/{}\"", "a".repeat(32)),
        ] {
            let channel: ChannelDescriptor =
                toml::from_str(&channel_toml_with_command_prefix(&bad)).unwrap();
            assert_eq!(
                channel.validate().unwrap_err(),
                ChannelDescriptorError::InvalidCommandPrefix,
                "expected invalid command_prefix: {bad}"
            );
        }
    }

    #[test]
    fn channel_descriptor_parses_the_documented_shape() {
        let channel: ChannelDescriptor = toml::from_str(documented_channel_toml()).unwrap();
        channel.validate().unwrap();
        assert_eq!(channel.conversation_model, ConversationModel::Continuous);
        let ingress = channel.ingress.as_ref().unwrap();
        assert_eq!(
            ingress
                .route_suffix
                .as_ref()
                .expect("webhook ingress declares a route_suffix")
                .as_str(),
            "events"
        );
        assert_eq!(ingress.body_limit_bytes, 1_048_576);
        assert!(channel.presentation.supports_threads);
        // Reply-placement contract (#7377): declared in the documented shape,
        // absent elsewhere in this module's fixtures — the field defaults to
        // false (anchored inline replies) unless a channel opts in.
        assert!(channel.presentation.can_reply_in_threads);
    }

    #[test]
    fn generated_code_prefixes_are_manifest_declared_and_bounded() {
        let channel: ChannelDescriptor =
            toml::from_str(&generated_code_connection_toml("[\"/start\", \"connect\"]")).unwrap();
        channel.validate().unwrap();
        assert_eq!(
            channel.connection.unwrap().inbound_code_prefixes,
            ["/start", "connect"]
        );

        for prefixes in [
            "[\"\"]",
            "[\"/start\", \"/start\"]",
            "[\"has space\"]",
            "[\"/123456789012345678901234567890123\"]",
            "[\"a\", \"b\", \"c\", \"d\", \"e\", \"f\", \"g\", \"h\", \"i\"]",
        ] {
            let channel: ChannelDescriptor =
                toml::from_str(&generated_code_connection_toml(prefixes)).unwrap();
            assert_eq!(
                channel.validate().unwrap_err(),
                ChannelDescriptorError::InvalidConnectionCodePrefixes,
                "expected invalid prefixes: {prefixes}"
            );
        }
    }

    #[test]
    fn generated_code_prefixes_reject_other_connection_strategies() {
        let source = generated_code_connection_toml("[\"/start\"]")
            .replace("web_generated_code", "oauth")
            .replace(
                "deep_link_template = \"https://vendor.example/connect?code={code}\"\n",
                "",
            );
        let channel: ChannelDescriptor = toml::from_str(&source).unwrap();
        assert_eq!(
            channel.validate().unwrap_err(),
            ChannelDescriptorError::InvalidConnectionCodePrefixes
        );
    }

    #[test]
    fn unsupported_connection_strategies_are_rejected_during_manifest_parse() {
        for strategy in ["inbound_proof_code", "qr_code"] {
            let source =
                generated_code_connection_toml("[]").replace("web_generated_code", strategy);
            assert!(
                toml::from_str::<ChannelDescriptor>(&source).is_err(),
                "legacy strategy {strategy} must not deserialize"
            );
        }
    }

    #[test]
    fn conversation_model_is_required() {
        let toml = documented_channel_toml().replace("conversation_model = \"continuous\"\n", "");
        let error = toml::from_str::<ChannelDescriptor>(&toml).unwrap_err();
        assert!(error.to_string().contains("conversation_model"), "{error}");
    }

    #[test]
    fn route_suffix_must_be_one_url_safe_segment() {
        for bad in ["a/b", "a.b", "", "A", "a b", "événement"] {
            assert!(RouteSuffix::new(bad).is_err(), "expected rejection: {bad}");
        }
        assert!(RouteSuffix::new("events").is_ok());
        assert!(RouteSuffix::new("events-v2_beta").is_ok());
    }

    #[test]
    fn egress_injection_target_parses_and_validates() {
        // Path-placeholder injection (token-in-path vendor APIs).
        let toml = documented_channel_toml().replace(
            "credential_handle = \"vendor_bot_token\"",
            "credential_handle = \"vendor_bot_token\"\ninjection = { type = \"path_placeholder\", placeholder = \"token\" }",
        );
        let channel: ChannelDescriptor = toml::from_str(&toml).unwrap();
        channel.validate().unwrap();
        assert!(matches!(
            channel.egress[0].injection,
            Some(ironclaw_host_api::http::RuntimeCredentialTarget::PathPlaceholder { .. })
        ));

        // Header injection stays expressible explicitly too.
        let toml = documented_channel_toml().replace(
            "credential_handle = \"vendor_bot_token\"",
            "credential_handle = \"vendor_bot_token\"\ninjection = { type = \"header\", name = \"authorization\", prefix = \"Bearer \" }",
        );
        let channel: ChannelDescriptor = toml::from_str(&toml).unwrap();
        channel.validate().unwrap();
    }

    #[test]
    fn basic_egress_injection_parses_and_rejects_invalid_usernames() {
        let source = |username: &str| {
            documented_channel_toml().replace(
                "credential_handle = \"vendor_bot_token\"",
                &format!(
                    "credential_handle = \"vendor_bot_token\"\n\
                     injection = {{ type = \"basic\", username = \"{username}\" }}"
                ),
            )
        };

        let channel: ChannelDescriptor = toml::from_str(&source("api-user")).unwrap();
        channel.validate().unwrap();
        assert!(matches!(
            channel.egress[0].injection,
            Some(ironclaw_host_api::http::RuntimeCredentialTarget::Basic { .. })
        ));

        for invalid in [" ", "user:name", r"user\u000Aname"] {
            let channel: ChannelDescriptor = toml::from_str(&source(invalid)).unwrap();
            assert_eq!(
                channel.validate().unwrap_err(),
                ChannelDescriptorError::InvalidEgressInjection {
                    host: "vendor.example".to_string(),
                },
                "expected Basic username rejection for {invalid:?}"
            );
        }
    }

    #[test]
    fn egress_injection_without_a_credential_handle_fails_closed() {
        let toml = documented_channel_toml().replace(
            "credential_handle = \"vendor_bot_token\"",
            "injection = { type = \"path_placeholder\", placeholder = \"token\" }",
        );
        let channel: ChannelDescriptor = toml::from_str(&toml).unwrap();
        assert!(matches!(
            channel.validate().unwrap_err(),
            ChannelDescriptorError::EgressInjectionWithoutCredential { .. }
        ));
    }

    #[test]
    fn egress_injection_shapes_are_validated() {
        for bad in [
            "injection = { type = \"path_placeholder\", placeholder = \"\" }",
            "injection = { type = \"path_placeholder\", placeholder = \"has space\" }",
            "injection = { type = \"query_param\", name = \" \" }",
            "injection = { type = \"header\", name = \"bad header\" }",
            // `body_json_pointer` is an RFC 6901 pointer, so it must be rooted.
            // Without this case the arm that enforces it never executes.
            "injection = { type = \"body_json_pointer\", pointer = \"secret\" }",
            "injection = { type = \"body_json_pointer\", pointer = \"\" }",
        ] {
            let toml = documented_channel_toml().replace(
                "credential_handle = \"vendor_bot_token\"",
                &format!("credential_handle = \"vendor_bot_token\"\n{bad}"),
            );
            let channel: ChannelDescriptor = toml::from_str(&toml).unwrap();
            assert!(
                matches!(
                    channel.validate().unwrap_err(),
                    ChannelDescriptorError::InvalidEgressInjection { .. }
                ),
                "expected rejection for: {bad}"
            );
        }
    }

    #[test]
    fn rooted_body_json_pointer_injection_is_accepted() {
        let toml = documented_channel_toml().replace(
            "credential_handle = \"vendor_bot_token\"",
            "credential_handle = \"vendor_bot_token\"\n\
             injection = { type = \"body_json_pointer\", pointer = \"/secret\" }",
        );
        let channel: ChannelDescriptor = toml::from_str(&toml).unwrap();
        channel
            .validate()
            .expect("a rooted RFC 6901 pointer is a well-formed injection target");
    }

    #[test]
    fn egress_hosts_must_be_literal() {
        let toml = documented_channel_toml()
            .replace("host = \"vendor.example\"", "host = \"*.vendor.example\"");
        let channel: ChannelDescriptor = toml::from_str(&toml).unwrap();
        assert!(matches!(
            channel.validate().unwrap_err(),
            ChannelDescriptorError::WildcardOrEmptyEgressHost { .. }
        ));
    }

    #[test]
    fn egress_paths_and_transfer_bounds_parse_and_validate() {
        let toml = documented_channel_toml().replace(
            "credential_handle = \"vendor_bot_token\"",
            "credential_handle = \"vendor_bot_token\"\ninjection = { type = \"path_placeholder\", placeholder = \"token\" }\npaths = [\"/bot{token}/getFile\"]\npath_prefixes = [\"/file/bot{token}/\"]\nrequest_body_limit_bytes = 65536\nresponse_body_limit_bytes = 5242880",
        );
        let channel: ChannelDescriptor = toml::from_str(&toml).unwrap();
        channel.validate().unwrap();
        let target = &channel.egress[0];
        assert_eq!(target.paths, vec!["/bot{token}/getFile"]);
        assert_eq!(target.path_prefixes, vec!["/file/bot{token}/"]);
        assert_eq!(target.request_body_limit_bytes, Some(65_536));
        assert_eq!(target.response_body_limit_bytes, Some(5 * 1024 * 1024));
    }

    /// A prefix without a trailing `/` does not end on a segment boundary, so
    /// prefix matching would authorize any sibling sharing its bytes. Reject
    /// the declaration rather than rely on every matcher to compensate.
    #[test]
    fn egress_path_prefix_without_a_segment_boundary_is_rejected() {
        let toml = documented_channel_toml().replace(
            "credential_handle = \"vendor_bot_token\"",
            "credential_handle = \"vendor_bot_token\"\ninjection = { type = \"path_placeholder\", placeholder = \"token\" }\npath_prefixes = [\"/file/bot{token}\"]",
        );
        let channel: ChannelDescriptor = toml::from_str(&toml).unwrap();
        assert!(matches!(
            channel.validate(),
            Err(ChannelDescriptorError::InvalidEgressConstraint { .. })
        ));
    }

    #[test]
    fn malformed_egress_paths_and_transfer_bounds_fail_closed() {
        for declaration in [
            "paths = [\"https://evil.example/file\"]",
            "paths = [\"/a/../secret\"]",
            "path_prefixes = [\"//evil.example/\"]",
            "path_prefixes = [\"/file\\\\escape\"]",
            "request_body_limit_bytes = 10485761",
            "response_body_limit_bytes = 10485761",
            "response_body_limit_bytes = 0",
        ] {
            let toml = documented_channel_toml().replace(
                "credential_handle = \"vendor_bot_token\"",
                &format!("credential_handle = \"vendor_bot_token\"\n{declaration}"),
            );
            let channel: ChannelDescriptor = toml::from_str(&toml).unwrap();
            assert!(
                matches!(
                    channel.validate().unwrap_err(),
                    ChannelDescriptorError::InvalidEgressConstraint { .. }
                ),
                "expected rejection for {declaration}"
            );
        }
    }

    #[test]
    fn evolving_channel_section_tolerates_unknown_fields_without_weakening_nested_recipes() {
        let toml = documented_channel_toml().replace(
            "conversation_model = \"continuous\"\n",
            "conversation_model = \"continuous\"\nsurprise = 1\n",
        );
        assert!(toml::from_str::<ChannelDescriptor>(&toml).is_ok());

        let unknown_nested = documented_channel_toml().replace(
            "secret_handle = \"vendor_signing_secret\"\n",
            "secret_handle = \"vendor_signing_secret\"\nsurprise = 1\n",
        );
        assert!(
            toml::from_str::<ChannelDescriptor>(&unknown_nested).is_err(),
            "only the evolving channel subsection is tolerant; security-sensitive recipes stay strict"
        );
    }

    #[test]
    fn wire_shape_round_trips() {
        let channel: ChannelDescriptor = toml::from_str(documented_channel_toml()).unwrap();
        let json = serde_json::to_string(&channel).unwrap();
        let back: ChannelDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(channel, back);
    }
}
