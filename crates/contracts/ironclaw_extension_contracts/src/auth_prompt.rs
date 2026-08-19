//! The channel-rendered **auth prompt** view family.
//!
//! The channel output traits' `OutboundPart::AuthPrompt` carries an
//! [`AuthPromptView`], and both shipped message-channel packages call
//! [`render_channel_auth_prompt`] from their send methods — so this family crosses the
//! host↔extension membrane and belongs on this side of it, not in
//! `ironclaw_product_contracts`. PROPOSAL §6.1.3 lists "auth/approval
//! prompt-view DTOs" together; at this base only the **auth** half is named by
//! a channel capability signature. The approval half (`ApprovalPrompt*View`) is reached
//! only by product and WebUI and stays in `product_contracts::outbound`.
//!
//! `AuthPromptContextView` moved with the rest of the family rather than
//! staying behind: it is `AuthPromptView`'s projection-side companion, built
//! by [`AuthPromptContextView::from_auth_prompt`], and splitting the two would
//! have put half a validated wire shape on each side of a crate boundary.
//!
//! The three private validators at the bottom are a deliberate, recorded
//! duplicate of the identically named helpers in
//! `ironclaw_product_contracts::outbound`, which still needs them for ~50
//! other projection types. Hoisting them instead would make a generic
//! display-text validator part of the extension membrane's *public* API to
//! serve a product-tier caller; that is the worse trade. WS1's "evict behavior
//! from `host_api` to product" row already owns `render_channel_auth_prompt`
//! and is where the two copies converge.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};

use ironclaw_host_api::ids::InvocationId;
use ironclaw_host_api::product_adapter_error::ProductAdapterError;
use ironclaw_host_api::turn::TurnRunId;

use crate::device_link::{
    DeviceLinkDisplayKind, DeviceLinkErrorCode, DeviceLinkInputKind, DeviceLinkMode,
    DeviceLinkStepKind,
};

/// Maximum byte length for a bounded identifier-shaped prompt field.
const PROJECTION_ITEM_ID_MAX_BYTES: usize = 512;
/// Maximum byte length for a free-text prompt field.
const PROJECTION_TEXT_MAX_BYTES: usize = 128 * 1024;

/// Discriminator for the kind of auth challenge surfaced in an `AuthPromptView`.
///
/// Added in issue #4112 as additive optional context. Legacy consumers that
/// serialized `AuthPromptView` before this field existed will deserialize it
/// as `None` (via `serde(default)`) without error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthPromptChallengeKind {
    /// Browser-based OAuth relay challenge. When `authorization_url` is present,
    /// the browser can open it in a new tab and wait for the OAuth callback to
    /// resume the run server-side. When the provider is unavailable or
    /// unconfigured, the URL may be absent so UI can still render an
    /// OAuth-specific unavailable state instead of the generic auth fallback.
    ///
    /// Wire value is `oauth_url` (for browser OAuth). The challenge kind is
    /// always re-derived at projection time from the persisted credential
    /// setup, never deserialized back from the wire.
    #[serde(rename = "oauth_url")]
    OAuthUrl,
    /// User pastes a secret string into the chat form. Wire value is
    /// `manual_token` (via `rename_all = "snake_case"`): paste a credential
    /// such as a GitHub PAT or API key.
    ManualToken,
    /// Host-issued channel pairing (WebGeneratedCode direction): the UI shows
    /// a code/deep-link panel and completion happens on the EXTERNAL side
    /// (e.g. Telegram `/start <code>`), then the run resumes server-side.
    /// Nothing is pasted into IronClaw. Wire value is `pairing`.
    Pairing,
    /// Multi-step device link: the card walks a user through a vendor's own
    /// "link a device" handshake — showing a scannable payload or a link,
    /// polling while the vendor waits, and asking for whatever the vendor
    /// demands next (an identifier, a one-time code, an account password).
    ///
    /// Distinct from [`AuthPromptChallengeKind::Pairing`] in both directions of
    /// travel: pairing is host-issued and completes when the user carries a
    /// host code to the external side, while a device link is vendor-issued and
    /// completes when the vendor accepts. A pairing card is one screen; this
    /// one advances through steps and can ask for a secret. Wire value is
    /// `device_link`.
    DeviceLink,
    /// Other challenge kind (account selection, setup required, reauthorize).
    /// The UI should fall back to a generic "authentication required" card.
    Other,
}

/// Manifest-derived connection context for a channel authentication challenge.
/// The strategy selects presentation while `channel` identifies the generic
/// connection route. Additive + serde-default so older rows deserialize as
/// `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ConnectionPromptContext {
    /// Connectable channel id (e.g. `telegram`).
    pub channel: String,
    /// Connect strategy wire value (e.g. `web_generated_code`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<String>,
    /// Backend-authored connect instructions for the pairing card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Placeholder for the paste input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_placeholder: Option<String>,
    /// Submit button label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submit_label: Option<String>,
    /// Error copy shown when the pasted code is rejected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// Host-issued pairing affordance for a `Pairing` auth challenge. The code,
/// deep link, expiry, and copy all come from the same manifest-driven pairing
/// service used by WebUI; channel adapters only choose native formatting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingPromptView {
    pub channel: String,
    pub display_name: String,
    pub instructions: String,
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deep_link: Option<String>,
    pub expires_at: DateTime<Utc>,
}

/// One frame of a multi-step device link, as a card renders it.
///
/// **Everything here is presentation.** The step machine, its revision
/// compare-and-swap, and the TTLs are the auth engine's; the protocol is the
/// extension's. This carries what a screen has to draw and what a poller has to
/// obey, and nothing that would let a consumer advance the flow itself.
///
/// **`qr_payload` is sensitive.** On the scan path it *is* the vendor's login
/// token: whoever renders it can invite a device onto the account. Show it to
/// the account's own user, never log it, never forward it to a channel that is
/// not the user's own surface. It is a `String` here only because the wire
/// shape is a string; the typed, redacting form is
/// [`crate::device_link::DeviceLinkPayload`], which is what the engine holds.
///
/// `revision` is the flow revision the frame was rendered from — a consumer
/// submitting the next step echoes it so a stale card cannot overwrite a newer
/// state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceLinkPromptView {
    /// Short provider id (the extension the link belongs to).
    pub provider: String,
    /// Human-readable name for the account being linked.
    pub display_name: String,
    /// Which frame this is.
    pub step: DeviceLinkStepKind,
    /// Copy for the current step, authored by the recipe or the host.
    pub instructions: String,
    /// The payload to render as a scannable code or a link, when the current
    /// step displays one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qr_payload: Option<String>,
    /// A short code to show the user, when the vendor issues one directly
    /// rather than as a scannable payload.
    ///
    /// Means only that. It used to double as the completed frame's resolved
    /// account identity, which left a card unable to tell "read this code to
    /// your phone" from "this is who you linked as" — see
    /// [`DeviceLinkPromptView::vendor_user_ref`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// The resolved account identity on a completed frame.
    ///
    /// Showing it is the one control that makes a substituted login visible
    /// (PROPOSAL §3.2), so it carries its own slot rather than borrowing the
    /// one a mid-flow code uses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor_user_ref: Option<String>,
    /// Label for the input the current step is asking for ("Login code",
    /// "Account password"). Present only on an input step; its presence is what
    /// tells a card to render a field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_label: Option<String>,
    /// When the current frame stops being valid.
    pub expires_at: DateTime<Utc>,
    /// The flow revision this frame was rendered from.
    pub revision: u64,
    /// How long a consumer waits between polls.
    pub poll_interval_ms: u64,
    /// Back-off the vendor asked for, when it asked for one. Overrides
    /// `poll_interval_ms` for the next poll only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    /// Why the last attempt failed, on a failed frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<DeviceLinkErrorCode>,
    /// The durable flow this frame belongs to (§8.12). A card with no flow id
    /// cannot poll or submit, so it starts a flow of its own instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_id: Option<String>,
    /// Which value an input step is asking for. Drives the masked-password
    /// affordance — absent, a consumer falls back to a code-shaped field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_kind: Option<DeviceLinkInputKind>,
    /// Which of the extension's declared paths the flow is on, so a card can
    /// offer "use the other path instead".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<DeviceLinkMode>,
    /// Whether the extension declares a second path at all.
    ///
    /// Load-bearing, not decorative: without it a card renders a switch for
    /// every vendor, and one that declares no alternate answers
    /// [`crate::device_link::DeviceLinkError::UnsupportedMode`] — a wedge the
    /// user cannot retry out of. Absent, a consumer must assume `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alternate_available: Option<bool>,
    /// The recipe's own name for the primary path ("Scan a code").
    ///
    /// The recipe promises the labels a user reads come from the extension.
    /// Carrying them here is what keeps that promise: a card with no label
    /// falls back to generic host copy, never to one vendor's ceremony.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_mode_label: Option<String>,
    /// The recipe's own name for the fallback path ("Use my phone number").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alternate_mode_label: Option<String>,
    /// How the payload is meant to be rendered.
    ///
    /// The contract has always distinguished a scannable code from a link;
    /// this is where that reaches a card. Absent, a consumer renders both
    /// affordances as it did before the field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_kind: Option<DeviceLinkDisplayKind>,
    /// The installed extension this link belongs to.
    ///
    /// Distinct from `provider`, which is the credential-authority namespace:
    /// a card needs the installed identity to name what it is linking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension_id: Option<String>,
    /// Whether a fresh `begin` could succeed after a failed frame. Mirrors
    /// `DeviceLinkStep::Failed`'s own bit; absent, a consumer derives it from
    /// `error_code`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restartable: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthPromptView {
    pub turn_run_id: TurnRunId,
    pub auth_request_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_id: Option<InvocationId>,
    pub headline: String,
    pub body: String,
    /// Challenge kind — present when the projection layer has auth-flow
    /// metadata available for this gate. Absent on rows written before #4112.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub challenge_kind: Option<AuthPromptChallengeKind>,
    /// Short provider id (e.g. `"google"`, `"github"`, `"notion"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Human-readable account label (e.g. `"work@example.com"`, `"GitHub PAT"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_label: Option<String>,
    /// Opaque IDP authorization URL. Only present for `OAuthUrl` challenges.
    /// This is the same URL already surfaced in the legacy
    /// `AppEvent::OnboardingState.auth_url` field — safe to render in the
    /// browser. Never contains a PKCE verifier, client secret, or token.
    ///
    /// Upstream projection converts this from validated `OAuthAuthorizationUrl`;
    /// the DTO stores a `String` only to preserve the stable JSON wire shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_url: Option<String>,
    /// Challenge expiry. Present when the auth flow has a bounded TTL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Manifest-derived channel connection context. Presentation is selected
    /// by `challenge_kind` and `connection.strategy`, never by provider name.
    /// Additive + serde-default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<ConnectionPromptContext>,
    /// Host-issued WebGeneratedCode presentation. Present only when
    /// `challenge_kind == Pairing` and the target is authorized to receive the
    /// bearer challenge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pairing: Option<PairingPromptView>,
    /// Device-link frame. Present only when `challenge_kind == DeviceLink`.
    /// Additive + serde-default, so rows written before the device-link method
    /// existed deserialize as `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_link: Option<DeviceLinkPromptView>,
}

/// Render one structured auth challenge for text-based channel adapters.
/// Recipe selection and challenge materialization happened upstream; this
/// helper only formats the already-typed view without naming a provider.
pub fn render_channel_auth_prompt(view: &AuthPromptView, direct_message: bool) -> String {
    let body = view
        .pairing
        .as_ref()
        .map(|pairing| pairing.instructions.as_str())
        .unwrap_or(view.body.as_str());
    let mut text = format!("{}\n\n{}", view.headline, body);
    if let Some(pairing) = view.pairing.as_ref() {
        text.push_str("\n\nPairing code: `");
        text.push_str(&pairing.code);
        text.push('`');
        if let Some(deep_link) = pairing.deep_link.as_deref() {
            text.push_str("\n\nOpen ");
            text.push_str(&pairing.display_name);
            text.push_str(": ");
            text.push_str(deep_link);
        }
        text.push_str("\n\nExpires: ");
        text.push_str(&pairing.expires_at.to_rfc3339());
    }
    text.push_str("\n\n");
    if direct_message {
        text.push_str("Reply `auth deny ");
        text.push_str(&view.auth_request_ref);
        text.push_str("` here to cancel this run.");
    } else {
        text.push_str("Mention me with `auth deny ");
        text.push_str(&view.auth_request_ref);
        text.push_str("` in this thread to cancel this run.");
    }
    if let Some(url) = view.authorization_url.as_deref() {
        text.push_str("\n\nSetup link: ");
        text.push_str(url);
    }
    text
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuthPromptContextView {
    pub challenge_kind: AuthPromptChallengeKind,
    /// Short provider id (e.g. `"google"`, `"github"`, `"notion"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Human-readable account label (e.g. `"work@example.com"`, `"GitHub PAT"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_label: Option<String>,
    /// Opaque IDP authorization URL. Only present for `OAuthUrl` challenges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_url: Option<String>,
    /// Challenge expiry. Present when the auth flow has a bounded TTL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Channel-pairing connection context — see [`AuthPromptView::connection`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<ConnectionPromptContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pairing: Option<PairingPromptView>,
    /// Device-link frame — see [`AuthPromptView::device_link`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_link: Option<DeviceLinkPromptView>,
}

impl AuthPromptContextView {
    pub fn new(
        challenge_kind: AuthPromptChallengeKind,
        provider: Option<String>,
        account_label: Option<String>,
        authorization_url: Option<String>,
        expires_at: Option<DateTime<Utc>>,
        connection: Option<ConnectionPromptContext>,
    ) -> Result<Self, ProductAdapterError> {
        Self::new_with_pairing(
            challenge_kind,
            provider,
            account_label,
            authorization_url,
            expires_at,
            connection,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_pairing(
        challenge_kind: AuthPromptChallengeKind,
        provider: Option<String>,
        account_label: Option<String>,
        authorization_url: Option<String>,
        expires_at: Option<DateTime<Utc>>,
        connection: Option<ConnectionPromptContext>,
        pairing: Option<PairingPromptView>,
    ) -> Result<Self, ProductAdapterError> {
        let view = Self {
            challenge_kind,
            provider,
            account_label,
            authorization_url,
            expires_at,
            connection,
            pairing,
            device_link: None,
        };
        view.validate()?;
        Ok(view)
    }

    /// Attach (or clear) the device-link frame, re-validating the whole view.
    ///
    /// A builder rather than an eighth constructor parameter: the two existing
    /// constructors already carry seven, and every caller that does not link a
    /// device would have to pass `None` through a wider signature for a field
    /// only one challenge kind uses.
    pub fn with_device_link(
        mut self,
        device_link: Option<DeviceLinkPromptView>,
    ) -> Result<Self, ProductAdapterError> {
        self.device_link = device_link;
        self.validate()?;
        Ok(self)
    }

    pub fn from_auth_prompt(prompt: &AuthPromptView) -> Result<Option<Self>, ProductAdapterError> {
        let Some(challenge_kind) = prompt.challenge_kind else {
            return Ok(None);
        };
        Self::new_with_pairing(
            challenge_kind,
            prompt.provider.clone(),
            prompt.account_label.clone(),
            prompt.authorization_url.clone(),
            prompt.expires_at,
            prompt.connection.clone(),
            prompt.pairing.clone(),
        )?
        .with_device_link(prompt.device_link.clone())
        .map(Some)
    }

    /// Re-validate an assembled view.
    ///
    /// `pub` because `ironclaw_product_contracts`'
    /// `ProductProjectionItem::validate` calls it across the crate boundary
    /// when a projection item carries auth context; it was private only while
    /// the two lived in one module.
    pub fn validate(&self) -> Result<(), ProductAdapterError> {
        validate_optional_display_text(
            "auth_prompt_provider",
            self.provider.as_deref(),
            PROJECTION_ITEM_ID_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "auth_prompt_account_label",
            self.account_label.as_deref(),
            PROJECTION_TEXT_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "auth_prompt_authorization_url",
            self.authorization_url.as_deref(),
            PROJECTION_TEXT_MAX_BYTES,
        )?;
        if let Some(connection) = self.connection.as_ref() {
            connection.validate()?;
        }
        if let Some(pairing) = self.pairing.as_ref() {
            pairing.validate()?;
        }
        if let Some(device_link) = self.device_link.as_ref() {
            device_link.validate()?;
        }
        Ok(())
    }
}

impl DeviceLinkPromptView {
    fn validate(&self) -> Result<(), ProductAdapterError> {
        validate_bounded_text(
            "device_link_prompt_provider",
            &self.provider,
            PROJECTION_ITEM_ID_MAX_BYTES,
        )?;
        validate_bounded_text(
            "device_link_prompt_display_name",
            &self.display_name,
            PROJECTION_TEXT_MAX_BYTES,
        )?;
        validate_bounded_text(
            "device_link_prompt_instructions",
            &self.instructions,
            PROJECTION_TEXT_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "device_link_prompt_qr_payload",
            self.qr_payload.as_deref(),
            PROJECTION_TEXT_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "device_link_prompt_code",
            self.code.as_deref(),
            PROJECTION_ITEM_ID_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "device_link_prompt_secret_label",
            self.secret_label.as_deref(),
            PROJECTION_ITEM_ID_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "device_link_prompt_flow_id",
            self.flow_id.as_deref(),
            PROJECTION_ITEM_ID_MAX_BYTES,
        )?;
        // The recipe-authored strings are extension-supplied, so they are
        // bounded on the same terms as every other text a card renders.
        validate_optional_display_text(
            "device_link_prompt_default_mode_label",
            self.default_mode_label.as_deref(),
            PROJECTION_ITEM_ID_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "device_link_prompt_alternate_mode_label",
            self.alternate_mode_label.as_deref(),
            PROJECTION_ITEM_ID_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "device_link_prompt_extension_id",
            self.extension_id.as_deref(),
            PROJECTION_ITEM_ID_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "device_link_prompt_vendor_user_ref",
            self.vendor_user_ref.as_deref(),
            PROJECTION_ITEM_ID_MAX_BYTES,
        )
    }
}

impl PairingPromptView {
    fn validate(&self) -> Result<(), ProductAdapterError> {
        validate_bounded_text(
            "pairing_prompt_channel",
            &self.channel,
            PROJECTION_ITEM_ID_MAX_BYTES,
        )?;
        validate_bounded_text(
            "pairing_prompt_display_name",
            &self.display_name,
            PROJECTION_TEXT_MAX_BYTES,
        )?;
        validate_bounded_text(
            "pairing_prompt_instructions",
            &self.instructions,
            PROJECTION_TEXT_MAX_BYTES,
        )?;
        validate_bounded_text(
            "pairing_prompt_code",
            &self.code,
            PROJECTION_ITEM_ID_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "pairing_prompt_deep_link",
            self.deep_link.as_deref(),
            PROJECTION_TEXT_MAX_BYTES,
        )
    }
}

impl ConnectionPromptContext {
    fn validate(&self) -> Result<(), ProductAdapterError> {
        validate_optional_display_text(
            "connection_channel",
            Some(self.channel.as_str()),
            PROJECTION_ITEM_ID_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "connection_strategy",
            self.strategy.as_deref(),
            PROJECTION_ITEM_ID_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "connection_instructions",
            self.instructions.as_deref(),
            PROJECTION_TEXT_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "connection_input_placeholder",
            self.input_placeholder.as_deref(),
            PROJECTION_TEXT_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "connection_submit_label",
            self.submit_label.as_deref(),
            PROJECTION_TEXT_MAX_BYTES,
        )?;
        validate_optional_display_text(
            "connection_error_message",
            self.error_message.as_deref(),
            PROJECTION_TEXT_MAX_BYTES,
        )
    }
}

impl<'de> Deserialize<'de> for AuthPromptContextView {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            challenge_kind: AuthPromptChallengeKind,
            #[serde(default)]
            provider: Option<String>,
            #[serde(default)]
            account_label: Option<String>,
            #[serde(default)]
            authorization_url: Option<String>,
            #[serde(default)]
            expires_at: Option<DateTime<Utc>>,
            #[serde(default)]
            connection: Option<ConnectionPromptContext>,
            #[serde(default)]
            pairing: Option<PairingPromptView>,
            #[serde(default)]
            device_link: Option<DeviceLinkPromptView>,
        }

        let wire = Wire::deserialize(deserializer)?;
        AuthPromptContextView::new_with_pairing(
            wire.challenge_kind,
            wire.provider,
            wire.account_label,
            wire.authorization_url,
            wire.expires_at,
            wire.connection,
            wire.pairing,
        )
        .and_then(|view| view.with_device_link(wire.device_link))
        .map_err(serde::de::Error::custom)
    }
}

fn invalid(kind: &'static str, reason: impl Into<String>) -> ProductAdapterError {
    ProductAdapterError::InvalidIdentifier {
        kind,
        reason: reason.into(),
    }
}

fn validate_bounded_text(
    kind: &'static str,
    value: &str,
    max: usize,
) -> Result<(), ProductAdapterError> {
    if value.is_empty() {
        return Err(invalid(kind, "must not be empty"));
    }
    if value.len() > max {
        return Err(invalid(kind, format!("must be at most {max} bytes")));
    }
    if value
        .chars()
        .any(|c| c == '\0' || c.is_control() && c != '\n' && c != '\t')
    {
        return Err(invalid(
            kind,
            "must not contain unsupported control characters",
        ));
    }
    Ok(())
}

fn validate_optional_display_text(
    kind: &'static str,
    value: Option<&str>,
    max: usize,
) -> Result<(), ProductAdapterError> {
    if let Some(value) = value {
        validate_bounded_text(kind, value, max)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_prompt_challenge_kind_all_variants_roundtrip() {
        // Stable wire values: browser OAuth, pasted credentials, and
        // host-issued channel pairing.
        for (variant, expected) in [
            (AuthPromptChallengeKind::OAuthUrl, "\"oauth_url\""),
            (AuthPromptChallengeKind::ManualToken, "\"manual_token\""),
            (AuthPromptChallengeKind::Pairing, "\"pairing\""),
            (AuthPromptChallengeKind::DeviceLink, "\"device_link\""),
            (AuthPromptChallengeKind::Other, "\"other\""),
        ] {
            let serialized = serde_json::to_string(&variant).expect("serialize challenge kind");
            assert_eq!(serialized, expected);
            let decoded: AuthPromptChallengeKind =
                serde_json::from_str(&serialized).expect("deserialize challenge kind");
            assert_eq!(decoded, variant);
        }
    }
    #[test]
    fn auth_prompt_context_from_prompt_rejects_invalid_prompt_context() {
        let prompt = AuthPromptView {
            turn_run_id: TurnRunId::new(),
            auth_request_ref: "gate:auth-test".to_string(),
            invocation_id: None,
            headline: "Authentication required".to_string(),
            body: "Authenticate to continue this run.".to_string(),
            challenge_kind: Some(AuthPromptChallengeKind::OAuthUrl),
            provider: Some("github".to_string()),
            account_label: None,
            authorization_url: Some("x".repeat(PROJECTION_TEXT_MAX_BYTES + 1)),
            expires_at: None,
            connection: None,
            pairing: None,
            device_link: None,
        };

        assert!(AuthPromptContextView::from_auth_prompt(&prompt).is_err());
    }

    fn pairing(deep_link: Option<&str>) -> PairingPromptView {
        PairingPromptView {
            channel: "telegram".to_string(),
            display_name: "Telegram".to_string(),
            instructions: "Open the app with the code.".to_string(),
            code: "ABC-123".to_string(),
            deep_link: deep_link.map(str::to_string),
            expires_at: DateTime::from_timestamp(1_700_000_000, 0).expect("timestamp"),
        }
    }

    fn view(pairing: Option<PairingPromptView>, url: Option<&str>) -> AuthPromptView {
        AuthPromptView {
            turn_run_id: TurnRunId::new(),
            auth_request_ref: "gate:auth-1".to_string(),
            invocation_id: None,
            headline: "Authentication required".to_string(),
            body: "Authenticate to continue this run.".to_string(),
            challenge_kind: Some(AuthPromptChallengeKind::OAuthUrl),
            provider: Some("github".to_string()),
            account_label: None,
            authorization_url: url.map(str::to_string),
            expires_at: None,
            connection: None,
            pairing,
            device_link: None,
        }
    }

    fn device_link_view() -> DeviceLinkPromptView {
        DeviceLinkPromptView {
            provider: "example".to_string(),
            display_name: "Personal account".to_string(),
            step: DeviceLinkStepKind::Display,
            instructions: "Open your account's device settings and scan this.".to_string(),
            qr_payload: Some("scheme://login?token=AAAA-BBBB".to_string()),
            code: None,
            secret_label: None,
            expires_at: DateTime::from_timestamp(1_700_000_000, 0).expect("timestamp"),
            revision: 3,
            poll_interval_ms: 3_000,
            retry_after_ms: None,
            error_code: None,
            flow_id: Some("11111111-2222-3333-4444-555555555555".to_string()),
            input_kind: None,
            mode: Some(DeviceLinkMode::Default),
            restartable: None,
            alternate_available: Some(false),
            default_mode_label: None,
            alternate_mode_label: None,
            display_kind: Some(DeviceLinkDisplayKind::QrCode),
            extension_id: Some("example".to_string()),
            vendor_user_ref: None,
        }
    }

    /// The rendering both shipped channel packages call from `deliver`: the DM
    /// and non-DM cancel affordances differ, and the setup link only appears
    /// when the challenge carries one.
    #[test]
    fn render_uses_the_body_and_switches_the_cancel_affordance_on_direct_message() {
        let direct = render_channel_auth_prompt(&view(None, None), true);
        assert!(
            direct.starts_with("Authentication required\n\nAuthenticate to continue this run.")
        );
        assert!(direct.contains("Reply `auth deny gate:auth-1` here to cancel this run."));
        assert!(!direct.contains("Mention me"));
        assert!(!direct.contains("Setup link"));

        let mention =
            render_channel_auth_prompt(&view(None, Some("https://example.test/authorize")), false);
        assert!(mention.contains(
            "Mention me with `auth deny gate:auth-1` in this thread to cancel this run."
        ));
        assert!(mention.ends_with("\n\nSetup link: https://example.test/authorize"));
    }

    /// A pairing challenge replaces the body with the pairing instructions and
    /// renders the code, expiry, and — only when present — the deep link.
    #[test]
    fn render_pairing_prefers_instructions_and_renders_the_optional_deep_link() {
        let without = render_channel_auth_prompt(&view(Some(pairing(None)), None), true);
        assert!(without.contains("Open the app with the code."));
        assert!(!without.contains("Authenticate to continue this run."));
        assert!(without.contains("Pairing code: `ABC-123`"));
        assert!(without.contains("Expires: 2023-11-14T22:13:20+00:00"));
        assert!(!without.contains("Open Telegram:"));

        let with = render_channel_auth_prompt(
            &view(Some(pairing(Some("https://t.me/bot?start=ABC-123"))), None),
            true,
        );
        assert!(with.contains("Open Telegram: https://t.me/bot?start=ABC-123"));
    }

    #[test]
    fn context_view_constructors_accept_a_valid_challenge_and_round_trip_the_wire() {
        let built = AuthPromptContextView::new(
            AuthPromptChallengeKind::ManualToken,
            Some("github".to_string()),
            Some("GitHub PAT".to_string()),
            None,
            None,
            None,
        )
        .expect("valid context");
        assert_eq!(built.challenge_kind, AuthPromptChallengeKind::ManualToken);

        let encoded = serde_json::to_value(&built).expect("serialize");
        let decoded: AuthPromptContextView = serde_json::from_value(encoded).expect("deserialize");
        assert_eq!(decoded, built);

        let from_prompt = AuthPromptContextView::from_auth_prompt(&view(None, None))
            .expect("valid")
            .expect("challenge kind present");
        assert_eq!(
            from_prompt.challenge_kind,
            AuthPromptChallengeKind::OAuthUrl
        );
    }

    /// `from_auth_prompt` is `None` — not an error — when the gate predates the
    /// challenge-kind field (#4112).
    #[test]
    fn context_view_is_absent_when_the_prompt_carries_no_challenge_kind() {
        let mut prompt = view(None, None);
        prompt.challenge_kind = None;
        assert!(
            AuthPromptContextView::from_auth_prompt(&prompt)
                .expect("no error")
                .is_none()
        );
    }

    /// Every nested validator fails closed, and the deserializer runs them —
    /// so an oversized field cannot enter through the wire either.
    #[test]
    fn nested_validators_reject_oversized_and_control_character_fields() {
        let oversize = "x".repeat(PROJECTION_TEXT_MAX_BYTES + 1);

        let mut bad_pairing = pairing(None);
        bad_pairing.display_name = oversize.clone();
        assert!(
            AuthPromptContextView::new_with_pairing(
                AuthPromptChallengeKind::Pairing,
                None,
                None,
                None,
                None,
                None,
                Some(bad_pairing),
            )
            .is_err()
        );

        let bad_connection = ConnectionPromptContext {
            channel: "telegram".to_string(),
            strategy: None,
            instructions: Some(format!("bad{}instructions", '\u{0}')),
            input_placeholder: None,
            submit_label: None,
            error_message: None,
        };
        assert!(
            AuthPromptContextView::new(
                AuthPromptChallengeKind::Pairing,
                None,
                None,
                None,
                None,
                Some(bad_connection),
            )
            .is_err()
        );

        assert!(
            AuthPromptContextView::new(
                AuthPromptChallengeKind::OAuthUrl,
                None,
                Some(oversize),
                None,
                None,
                None,
            )
            .is_err()
        );

        // The wire path validates too: a hand-built payload cannot bypass it.
        let wire = serde_json::json!({
            "challenge_kind": "oauth_url",
            "provider": format!("{}bad", '\u{0}'),
        });
        assert!(serde_json::from_value::<AuthPromptContextView>(wire).is_err());
    }

    /// An empty bounded field is rejected distinctly from an oversized one.
    #[test]
    fn bounded_text_rejects_empty_as_well_as_oversized() {
        let mut empty_code = pairing(None);
        empty_code.code = String::new();
        assert!(
            AuthPromptContextView::new_with_pairing(
                AuthPromptChallengeKind::Pairing,
                None,
                None,
                None,
                None,
                None,
                Some(empty_code),
            )
            .is_err()
        );
    }

    /// Every optional field populated and valid, so each validator call in
    /// `PairingPromptView::validate` and `ConnectionPromptContext::validate`
    /// runs to completion instead of short-circuiting on the first rejection.
    #[test]
    fn fully_populated_pairing_and_connection_pass_every_validator_arm() {
        let mut full_pairing = pairing(Some("https://t.me/bot?start=ABC-123"));
        full_pairing.display_name = "Telegram".to_string();

        let connection = ConnectionPromptContext {
            channel: "telegram".to_string(),
            strategy: Some("web_generated_code".to_string()),
            instructions: Some("Open the app with the generated code.".to_string()),
            input_placeholder: Some("Paste the code".to_string()),
            submit_label: Some("Connect".to_string()),
            error_message: Some("Invalid or expired pairing code.".to_string()),
        };

        let context = AuthPromptContextView::new_with_pairing(
            AuthPromptChallengeKind::Pairing,
            Some("telegram".to_string()),
            Some("Telegram DM".to_string()),
            Some("https://example.test/authorize".to_string()),
            Some(DateTime::from_timestamp(1_700_000_000, 0).expect("timestamp")),
            Some(connection),
            Some(full_pairing),
        )
        .expect("every populated field is within bounds");

        // Re-running validation from the public entry point exercises the same
        // arms a second time through the nested `?` chain.
        context.validate().expect("revalidation succeeds");

        let decoded: AuthPromptContextView =
            serde_json::from_value(serde_json::to_value(&context).expect("serialize"))
                .expect("wire round-trip revalidates every arm");
        assert_eq!(decoded, context);
    }

    /// Each connection field rejects independently, so a later arm cannot be
    /// masked by an earlier one that already failed.
    #[test]
    fn every_connection_field_rejects_on_its_own() {
        let oversize = "x".repeat(PROJECTION_TEXT_MAX_BYTES + 1);
        let base = ConnectionPromptContext {
            channel: "telegram".to_string(),
            strategy: None,
            instructions: None,
            input_placeholder: None,
            submit_label: None,
            error_message: None,
        };

        for (label, mutate) in [
            ("channel", 5usize),
            ("strategy", 0usize),
            ("instructions", 1),
            ("input_placeholder", 2),
            ("submit_label", 3),
            ("error_message", 4),
        ] {
            let mut connection = base.clone();
            match mutate {
                5 => connection.channel = "x".repeat(PROJECTION_ITEM_ID_MAX_BYTES + 1),
                0 => connection.strategy = Some("x".repeat(PROJECTION_ITEM_ID_MAX_BYTES + 1)),
                1 => connection.instructions = Some(oversize.clone()),
                2 => connection.input_placeholder = Some(oversize.clone()),
                3 => connection.submit_label = Some(oversize.clone()),
                _ => connection.error_message = Some(oversize.clone()),
            }
            assert!(
                AuthPromptContextView::new(
                    AuthPromptChallengeKind::Pairing,
                    None,
                    None,
                    None,
                    None,
                    Some(connection),
                )
                .is_err(),
                "{label} must reject independently"
            );
        }
    }

    /// Same, for the pairing view's own bounded fields.
    #[test]
    fn every_pairing_field_rejects_on_its_own() {
        let oversize = "x".repeat(PROJECTION_TEXT_MAX_BYTES + 1);
        for mutate in 0..4usize {
            let mut view = pairing(Some("https://t.me/bot?start=ABC-123"));
            match mutate {
                0 => view.channel = "x".repeat(PROJECTION_ITEM_ID_MAX_BYTES + 1),
                1 => view.display_name = oversize.clone(),
                2 => view.instructions = oversize.clone(),
                _ => view.deep_link = Some(oversize.clone()),
            }
            assert!(
                AuthPromptContextView::new_with_pairing(
                    AuthPromptChallengeKind::Pairing,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(view),
                )
                .is_err(),
                "pairing field {mutate} must reject independently"
            );
        }
    }

    /// The device-link frame rides both views as an additive optional field,
    /// survives the wire in both directions, and reaches the context view
    /// through `from_auth_prompt` rather than having to be re-attached by hand.
    #[test]
    fn device_link_frame_round_trips_through_both_views() {
        let mut prompt = view(None, None);
        prompt.challenge_kind = Some(AuthPromptChallengeKind::DeviceLink);
        prompt.device_link = Some(device_link_view());

        let encoded = serde_json::to_value(&prompt).expect("serialize prompt");
        assert_eq!(encoded["challenge_kind"], "device_link");
        assert_eq!(encoded["device_link"]["step"], "display");
        assert_eq!(encoded["device_link"]["revision"], 3);
        assert_eq!(encoded["device_link"]["poll_interval_ms"], 3_000);
        assert!(
            encoded["device_link"].get("retry_after_ms").is_none(),
            "absent optionals are omitted, not encoded as null: {encoded}"
        );
        assert_eq!(
            serde_json::from_value::<AuthPromptView>(encoded).expect("deserialize prompt"),
            prompt
        );

        let context = AuthPromptContextView::from_auth_prompt(&prompt)
            .expect("valid")
            .expect("challenge kind present");
        assert_eq!(context.challenge_kind, AuthPromptChallengeKind::DeviceLink);
        assert_eq!(context.device_link.as_ref(), prompt.device_link.as_ref());

        let encoded = serde_json::to_value(&context).expect("serialize context");
        assert_eq!(
            serde_json::from_value::<AuthPromptContextView>(encoded).expect("deserialize context"),
            context
        );
    }

    /// A prompt written before the device-link method existed has no field at
    /// all; it must deserialize as `None` rather than failing.
    #[test]
    fn device_link_field_is_additive_for_rows_written_without_it() {
        let legacy = serde_json::json!({
            "turn_run_id": TurnRunId::new(),
            "auth_request_ref": "gate:auth-legacy",
            "headline": "Authentication required",
            "body": "Authenticate to continue this run.",
            "challenge_kind": "oauth_url",
        });
        let decoded: AuthPromptView =
            serde_json::from_value(legacy).expect("legacy row deserializes");
        assert!(decoded.device_link.is_none());

        let context = serde_json::json!({ "challenge_kind": "device_link" });
        let decoded: AuthPromptContextView =
            serde_json::from_value(context).expect("legacy context deserializes");
        assert!(decoded.device_link.is_none());
    }

    /// The frame's bounded fields fail closed, independently, and through the
    /// wire — the same contract every sibling projection here holds.
    #[test]
    fn every_device_link_frame_field_rejects_on_its_own() {
        let oversize = "x".repeat(PROJECTION_TEXT_MAX_BYTES + 1);
        let long_id = "x".repeat(PROJECTION_ITEM_ID_MAX_BYTES + 1);

        for (label, mutate) in [
            ("provider", 0usize),
            ("display_name", 1),
            ("instructions", 2),
            ("qr_payload", 3),
            ("code", 4),
            ("secret_label", 5),
        ] {
            let mut frame = device_link_view();
            match mutate {
                0 => frame.provider = long_id.clone(),
                1 => frame.display_name = oversize.clone(),
                2 => frame.instructions = oversize.clone(),
                3 => frame.qr_payload = Some(oversize.clone()),
                4 => frame.code = Some(long_id.clone()),
                _ => frame.secret_label = Some(long_id.clone()),
            }
            assert!(
                AuthPromptContextView::new(
                    AuthPromptChallengeKind::DeviceLink,
                    None,
                    None,
                    None,
                    None,
                    None,
                )
                .expect("base context")
                .with_device_link(Some(frame))
                .is_err(),
                "{label} must reject independently"
            );
        }

        // Empty is rejected distinctly from oversized.
        let mut empty_instructions = device_link_view();
        empty_instructions.instructions = String::new();
        assert!(
            AuthPromptContextView::new(
                AuthPromptChallengeKind::DeviceLink,
                None,
                None,
                None,
                None,
                None,
            )
            .expect("base context")
            .with_device_link(Some(empty_instructions))
            .is_err()
        );

        // And the wire path validates too, so a hand-built payload cannot slip
        // past the constructor.
        let wire = serde_json::json!({
            "challenge_kind": "device_link",
            "device_link": {
                "provider": format!("{}bad", '\u{0}'),
                "display_name": "Personal account",
                "step": "display",
                "instructions": "Scan this.",
                "expires_at": "2023-11-14T22:13:20Z",
                "revision": 1,
                "poll_interval_ms": 3000,
            },
        });
        assert!(serde_json::from_value::<AuthPromptContextView>(wire).is_err());
    }

    /// Every optional populated and valid, so each validator arm in
    /// `DeviceLinkPromptView::validate` runs instead of short-circuiting.
    #[test]
    fn fully_populated_device_link_frame_passes_every_validator_arm() {
        let frame = DeviceLinkPromptView {
            provider: "example".to_string(),
            display_name: "Personal account".to_string(),
            step: DeviceLinkStepKind::Failed,
            instructions: "That code was not accepted. Try again.".to_string(),
            qr_payload: Some("scheme://login?token=AAAA-BBBB".to_string()),
            code: Some("A1B2C3".to_string()),
            secret_label: Some("Login code".to_string()),
            expires_at: DateTime::from_timestamp(1_700_000_000, 0).expect("timestamp"),
            revision: 9,
            poll_interval_ms: 3_000,
            retry_after_ms: Some(30_000),
            error_code: Some(DeviceLinkErrorCode::InvalidInput),
            flow_id: Some("11111111-2222-3333-4444-555555555555".to_string()),
            input_kind: Some(DeviceLinkInputKind::Code),
            mode: Some(DeviceLinkMode::Alternate),
            restartable: Some(true),
            alternate_available: Some(true),
            default_mode_label: Some("Scan a code".to_string()),
            alternate_mode_label: Some("Use your account name".to_string()),
            display_kind: Some(DeviceLinkDisplayKind::Link),
            extension_id: Some("example".to_string()),
            vendor_user_ref: Some("@example-user".to_string()),
        };

        let context = AuthPromptContextView::new(
            AuthPromptChallengeKind::DeviceLink,
            Some("example".to_string()),
            Some("Personal account".to_string()),
            None,
            Some(DateTime::from_timestamp(1_700_000_000, 0).expect("timestamp")),
            None,
        )
        .expect("base context")
        .with_device_link(Some(frame))
        .expect("every populated field is within bounds");

        context.validate().expect("revalidation succeeds");

        let encoded = serde_json::to_value(&context).expect("serialize");
        assert_eq!(encoded["device_link"]["error_code"], "invalid_input");
        assert_eq!(encoded["device_link"]["retry_after_ms"], 30_000);
        assert_eq!(
            serde_json::from_value::<AuthPromptContextView>(encoded)
                .expect("wire round-trip revalidates every arm"),
            context
        );
    }

    /// `validate_bounded_text` treats newline and tab as legal formatting but
    /// every other control character as a rejection. Both halves of that
    /// predicate matter: the prompt copy channels render is multi-line, so a
    /// stricter rule would reject real instructions.
    #[test]
    fn bounded_text_allows_newline_and_tab_but_rejects_other_control_characters() {
        let mut formatted = pairing(None);
        formatted.instructions = "Open the app.\n\n\tThen paste the code.".to_string();
        let context = AuthPromptContextView::new_with_pairing(
            AuthPromptChallengeKind::Pairing,
            None,
            None,
            None,
            None,
            None,
            Some(formatted),
        )
        .expect("newline and tab are legal formatting in prompt copy");
        assert!(
            context
                .pairing
                .as_ref()
                .expect("pairing")
                .instructions
                .contains('\t')
        );

        for control in ['\u{0}', '\u{1}', '\u{7}', '\u{1b}', '\u{7f}'] {
            let mut bad = pairing(None);
            bad.instructions = format!("Open{control}the app.");
            assert!(
                AuthPromptContextView::new_with_pairing(
                    AuthPromptChallengeKind::Pairing,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(bad),
                )
                .is_err(),
                "control character {control:?} must be rejected"
            );
        }
    }
}
