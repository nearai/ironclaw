//! The channel-rendered **auth prompt** view family.
//!
//! `ChannelAdapter`'s own `OutboundPart::AuthPrompt` carries an
//! [`AuthPromptView`], and both shipped channel packages call
//! [`render_channel_auth_prompt`] from `deliver` — so this family crosses the
//! host↔extension membrane and belongs on this side of it, not in
//! `ironclaw_product_contracts`. PROPOSAL §6.1.3 lists "auth/approval
//! prompt-view DTOs" together; at this base only the **auth** half is named by
//! an adapter signature. The approval half (`ApprovalPrompt*View`) is reached
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
        };
        view.validate()?;
        Ok(view)
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
        )
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
        Ok(())
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
        };

        assert!(AuthPromptContextView::from_auth_prompt(&prompt).is_err());
    }
}
