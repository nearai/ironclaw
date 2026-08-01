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
}
