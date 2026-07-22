//! Channel-surface declaration vocabulary (`[channel]` in a v3 manifest).
//!
//! One extension declares at most one channel surface. The host consumes the
//! descriptor everywhere — ingress routing, config forms, conversation
//! binding, presentation policy — so the vocabulary lives in the contracts
//! crate; adapters implement behavior only and are never asked for metadata.

use serde::{Deserialize, Serialize};

use crate::{
    HostApiError, IngressVerificationRecipe, NetworkScheme, RecipeSecretField,
    RecipeValidationError, SecretHandle,
};

/// How external conversations map to IronClaw conversations
/// (`docs/reborn/extension-runtime/overview.md` §3). The host WebUI's
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

/// The declared channel surface of one extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelDescriptor {
    /// Channel surface id within the extension (e.g. `messages`).
    pub id: String,
    pub display_name: String,
    #[serde(default)]
    pub inbound: bool,
    #[serde(default)]
    pub outbound: bool,
    /// Required: how external conversations bind (checklist MAN-10).
    pub conversation_model: ConversationModel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingress: Option<ChannelIngressDescriptor>,
    #[serde(default)]
    pub config: ChannelConfigDescriptor,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub egress: Vec<ChannelEgressDescriptor>,
    #[serde(default)]
    pub presentation: ChannelPresentation,
}

impl ChannelDescriptor {
    /// Structural validation beyond field-level deserialization.
    pub fn validate(&self) -> Result<(), ChannelDescriptorError> {
        if self.id.trim().is_empty() {
            return Err(ChannelDescriptorError::EmptyId);
        }
        if self.display_name.trim().is_empty() {
            return Err(ChannelDescriptorError::EmptyDisplayName);
        }
        if self.inbound && self.ingress.is_none() {
            return Err(ChannelDescriptorError::InboundWithoutIngress);
        }
        if let Some(ingress) = &self.ingress {
            ingress
                .verification
                .validate()
                .map_err(ChannelDescriptorError::Verification)?;
            if let Some(handle) = ingress.verification.secret_handle()
                && !self.declares_config_handle(handle)
            {
                return Err(ChannelDescriptorError::UndeclaredVerificationHandle {
                    handle: handle.as_str().to_string(),
                });
            }
        }
        for egress in &self.egress {
            if egress.host.trim().is_empty() || egress.host.contains('*') {
                return Err(ChannelDescriptorError::WildcardOrEmptyEgressHost {
                    host: egress.host.clone(),
                });
            }
            if let Some(handle) = &egress.credential_handle
                && !self.declares_config_handle(handle)
            {
                return Err(ChannelDescriptorError::UndeclaredEgressHandle {
                    handle: handle.as_str().to_string(),
                });
            }
            if let Some(injection) = &egress.injection {
                if egress.credential_handle.is_none() {
                    return Err(ChannelDescriptorError::EgressInjectionWithoutCredential {
                        host: egress.host.clone(),
                    });
                }
                let well_formed = match injection {
                    crate::RuntimeCredentialTarget::Header { name, .. } => {
                        crate::valid_http_field_name(name)
                    }
                    crate::RuntimeCredentialTarget::QueryParam { name } => {
                        !name.trim().is_empty() && !name.contains(char::is_whitespace)
                    }
                    crate::RuntimeCredentialTarget::PathPlaceholder { placeholder } => {
                        !placeholder.is_empty()
                            && placeholder
                                .chars()
                                .all(|c| c.is_ascii_alphanumeric() || c == '_')
                    }
                    crate::RuntimeCredentialTarget::BodyJsonPointer { pointer } => {
                        pointer.starts_with('/')
                    }
                };
                if !well_formed {
                    return Err(ChannelDescriptorError::InvalidEgressInjection {
                        host: egress.host.clone(),
                    });
                }
            }
            let mut seen_body_handles: Vec<&str> = Vec::new();
            for body_credential in &egress.body_credentials {
                if !self.declares_config_handle(&body_credential.handle) {
                    return Err(ChannelDescriptorError::UndeclaredEgressHandle {
                        handle: body_credential.handle.as_str().to_string(),
                    });
                }
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

    fn declares_config_handle(&self, handle: &SecretHandle) -> bool {
        self.config
            .fields
            .iter()
            .any(|field| &field.handle == handle)
    }
}

/// Ingress declaration for an inbound channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelIngressDescriptor {
    pub route_suffix: RouteSuffix,
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

/// Operator setup fields; the host renders the generic form and stores
/// secret fields under the named handles.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelConfigDescriptor {
    #[serde(default)]
    pub fields: Vec<RecipeSecretField>,
}

/// One declared egress target for the channel adapter's vendor calls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelEgressDescriptor {
    #[serde(default = "default_https")]
    pub scheme: NetworkScheme,
    pub host: String,
    pub methods: Vec<crate::NetworkMethod>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_handle: Option<SecretHandle>,
    /// How the host injects the declared credential into vendor requests.
    /// Absent means the default `Authorization: Bearer <secret>` header.
    /// `path_placeholder` covers vendors that carry the credential in the URL
    /// path (the adapter writes `{placeholder}` into the path; the host
    /// substitutes the secret — bytes never reach the adapter).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub injection: Option<crate::RuntimeCredentialTarget>,
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
    /// pre-v3-declaration behavior; declared limits are enforced before I/O.
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
    injection: Option<&crate::RuntimeCredentialTarget>,
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
        Some(crate::RuntimeCredentialTarget::PathPlaceholder { placeholder }) => {
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

/// Presentation facts prompt construction consumes
/// (`CommunicationPresentationPolicy` derives from this).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelPresentation {
    #[serde(default)]
    pub supports_markdown: bool,
    #[serde(default)]
    pub supports_threads: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_message_chars: Option<u32>,
}

/// Structural channel-descriptor failures (path context added by the
/// manifest parser).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChannelDescriptorError {
    #[error("channel id must not be empty")]
    EmptyId,
    #[error("channel display_name must not be empty")]
    EmptyDisplayName,
    #[error("an inbound channel must declare [channel.ingress]")]
    InboundWithoutIngress,
    #[error(transparent)]
    Verification(RecipeValidationError),
    #[error("verification secret handle `{handle}` is not declared in [channel.config] fields")]
    UndeclaredVerificationHandle { handle: String },
    #[error("egress credential handle `{handle}` is not declared in [channel.config] fields")]
    UndeclaredEgressHandle { handle: String },
    #[error("egress target `{host}` declares an injection but no credential_handle")]
    EgressInjectionWithoutCredential { host: String },
    #[error("egress target `{host}` declares a malformed credential injection")]
    InvalidEgressInjection { host: String },
    #[error("egress host `{host}` must be a literal, non-empty host (no wildcards)")]
    WildcardOrEmptyEgressHost { host: String },
    #[error("egress target `{host}` declares an invalid path or transfer bound")]
    InvalidEgressConstraint { host: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn documented_channel_toml() -> &'static str {
        r#"
id = "messages"
display_name = "Vendor messages"
inbound = true
outbound = true
conversation_model = "continuous"

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

[config]
fields = [
  { handle = "vendor_bot_token", label = "Bot token", secret = true },
  { handle = "vendor_signing_secret", label = "Signing secret", secret = true },
]

[[egress]]
scheme = "https"
host = "vendor.example"
methods = ["post"]
credential_handle = "vendor_bot_token"

[presentation]
supports_markdown = true
supports_threads = true
max_message_chars = 40000
"#
    }

    #[test]
    fn channel_descriptor_parses_the_documented_shape() {
        let channel: ChannelDescriptor = toml::from_str(documented_channel_toml()).unwrap();
        channel.validate().unwrap();
        assert_eq!(channel.conversation_model, ConversationModel::Continuous);
        let ingress = channel.ingress.as_ref().unwrap();
        assert_eq!(ingress.route_suffix.as_str(), "events");
        assert_eq!(ingress.body_limit_bytes, 1_048_576);
        assert!(channel.presentation.supports_threads);
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
    fn verification_secret_must_be_declared_in_config() {
        let toml = documented_channel_toml().replace(
            "{ handle = \"vendor_signing_secret\", label = \"Signing secret\", secret = true },",
            "",
        );
        let channel: ChannelDescriptor = toml::from_str(&toml).unwrap();
        assert!(matches!(
            channel.validate().unwrap_err(),
            ChannelDescriptorError::UndeclaredVerificationHandle { .. }
        ));
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
            Some(crate::RuntimeCredentialTarget::PathPlaceholder { .. })
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

    #[test]
    fn malformed_egress_paths_and_transfer_bounds_fail_closed() {
        for declaration in [
            "paths = [\"https://evil.example/file\"]",
            "paths = [\"/a/../secret\"]",
            "path_prefixes = [\"//evil.example/\"]",
            "path_prefixes = [\"/file\\\\escape\"]",
            "request_body_limit_bytes = 10485761",
            "response_body_limit_bytes = 10485761",
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
    fn inbound_channels_require_ingress() {
        let channel: ChannelDescriptor = toml::from_str(
            r#"
id = "messages"
display_name = "Vendor messages"
inbound = true
conversation_model = "continuous"
"#,
        )
        .unwrap();
        assert!(matches!(
            channel.validate().unwrap_err(),
            ChannelDescriptorError::InboundWithoutIngress
        ));
    }

    #[test]
    fn unknown_channel_fields_fail_closed() {
        let toml = format!("{}\nsurprise = 1\n", documented_channel_toml());
        let error = toml::from_str::<ChannelDescriptor>(&toml).unwrap_err();
        assert!(error.to_string().contains("surprise"), "{error}");
    }

    #[test]
    fn wire_shape_round_trips() {
        let channel: ChannelDescriptor = toml::from_str(documented_channel_toml()).unwrap();
        let json = serde_json::to_string(&channel).unwrap();
        let back: ChannelDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(channel, back);
    }
}
