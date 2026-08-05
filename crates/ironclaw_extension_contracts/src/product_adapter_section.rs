//! The `[product_adapter.*]` manifest-surface schema.
//!
//! This is the neutral vocabulary half of the `ironclaw.product_adapter/v1`
//! host-API surface: what an extension **declares** in its manifest, and the
//! cross-field invariants that declaration must satisfy. It is the same shape
//! [`crate::channel`] holds for `[channel]` and [`crate::memory`] holds for
//! `[memory]` — a `Deserialize` declaration plus its validation, with no
//! manifest parsing, no section-path addressing, and no registry types.
//!
//! The *resolved projection* — pairing a resolved section with the
//! `ManifestSectionPath` it was declared at, walking the manifest's host-API
//! list, and the `HostApiManifestContract` that hooks this schema into v2
//! manifest ingestion — is the registry's, in
//! `ironclaw_extensions::host_api::product_adapter`. That split is PROPOSAL
//! §6.1.2 (this crate: "manifest-surface descriptors", "parses no manifests")
//! against §6.8.1 (the registry: "manifest schemas … resolved + digest").

use std::collections::BTreeSet;

use ironclaw_host_api::ids::ExtensionId;
use ironclaw_host_api::ingress::{IngressAuthPolicy, IngressRouteDescriptor, IngressRouteId};
use ironclaw_host_api::product_adapter::{
    AuthRequirement, ProductAdapterCapabilities, ProductAdapterId, ProductCapabilityFlag,
    ProductSurfaceKind,
};
use serde::Deserialize;
use thiserror::Error;

use crate::egress::{DeclaredEgressTarget, EgressCredentialHandle};

/// The host-API id a `[product_adapter.*]` section is declared under.
pub const PRODUCT_ADAPTER_HOST_API_ID: &str = "ironclaw.product_adapter/v1";

/// The manifest section-path prefix every product-adapter section shares.
pub const PRODUCT_ADAPTER_SECTION_PREFIX: &str = "product_adapter";

// ---------------------------------------------------------------------------
// Declared shapes
// ---------------------------------------------------------------------------

/// A host-ingress route declared by a ProductAdapter manifest section, paired
/// with the credential handles that verify it.
///
/// The route itself is the host-owned [`IngressRouteDescriptor`] vocabulary
/// (`ironclaw_host_api` owns route/policy validation, including the fail-closed
/// floor that a `PublicWebhook` listener must require `WebhookSignature`). That
/// descriptor deliberately carries **no** credential binding — host_api is
/// route/policy vocabulary only. The manifest layer is therefore where "which
/// credential handle verifies this route" is declared, and this module makes it
/// credential-coherent against the section's `required_credentials`
/// (see [`ProductAdapterSection`]'s validation).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostIngressRoute {
    descriptor: IngressRouteDescriptor,
    #[serde(default)]
    credential_handles: Vec<EgressCredentialHandle>,
}

impl HostIngressRoute {
    /// The host-owned, already-validated ingress route/policy descriptor.
    pub fn descriptor(&self) -> &IngressRouteDescriptor {
        &self.descriptor
    }

    /// Credential handles that verify this route. Every handle is guaranteed to
    /// be declared in the owning section's `required_credentials`; an
    /// auth-required route names at least one, and a public (no-auth) route
    /// names none.
    ///
    /// The handle type is [`EgressCredentialHandle`] — the single credential-
    /// handle newtype this crate owns. It is reused here rather than mirrored
    /// into an ingress-specific type (per the type-placement rule); its
    /// `Display` renders only the handle string, so no "egress" wording leaks
    /// into ingress error messages.
    pub fn credential_handles(&self) -> &[EgressCredentialHandle] {
        &self.credential_handles
    }
}

/// The wire shape of a `[product_adapter.*]` manifest section.
///
/// Deserialized by whoever holds the manifest TOML (the registry), then
/// [resolved](Self::resolve) into a validated [`ProductAdapterSection`].
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductAdapterSectionDeclaration {
    surface_kind: ProductSurfaceKind,
    auth: DeclaredAuth,
    capabilities: DeclaredCapabilities,
    #[serde(default)]
    required_credentials: Vec<DeclaredCredential>,
    #[serde(default)]
    egress: Vec<DeclaredEgressTarget>,
    #[serde(default)]
    host_ingress: Vec<HostIngressRoute>,
}

impl ProductAdapterSectionDeclaration {
    /// Project and validate this declaration into a resolved section.
    ///
    /// `subsection` is the section path's tail below
    /// [`PRODUCT_ADAPTER_SECTION_PREFIX`]; it is combined with `extension_id`
    /// into the [`ProductAdapterId`] so that multiple product-adapter sections
    /// within the same extension are distinguishable downstream.
    pub fn resolve(
        self,
        extension_id: &ExtensionId,
        subsection: &str,
    ) -> Result<ProductAdapterSection, ProductAdapterSectionError> {
        let adapter_id_str = format!("{}/{}", extension_id.as_str(), subsection);
        let adapter_id = ProductAdapterId::new(&adapter_id_str).map_err(|error| {
            ProductAdapterSectionError::InvalidValue {
                field: "adapter_id",
                reason: error.to_string(),
            }
        })?;
        let auth_requirement = self.auth.into_auth_requirement()?;
        let required_credentials = self
            .required_credentials
            .into_iter()
            .map(|c| c.handle)
            .collect();
        let projected = ProductAdapterSection {
            adapter_id,
            surface_kind: self.surface_kind,
            capabilities: ProductAdapterCapabilities::new(self.capabilities.flags),
            auth_requirement,
            declared_egress: self.egress,
            required_credentials,
            host_ingress: self.host_ingress,
        };
        projected.validate()?;
        Ok(projected)
    }
}

// ---------------------------------------------------------------------------
// Resolved section
// ---------------------------------------------------------------------------

/// A validated `[product_adapter.*]` section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductAdapterSection {
    adapter_id: ProductAdapterId,
    surface_kind: ProductSurfaceKind,
    capabilities: ProductAdapterCapabilities,
    auth_requirement: AuthRequirement,
    declared_egress: Vec<DeclaredEgressTarget>,
    required_credentials: Vec<EgressCredentialHandle>,
    host_ingress: Vec<HostIngressRoute>,
}

impl ProductAdapterSection {
    pub fn adapter_id(&self) -> &ProductAdapterId {
        &self.adapter_id
    }
    pub fn surface_kind(&self) -> ProductSurfaceKind {
        self.surface_kind
    }
    pub fn capabilities(&self) -> &ProductAdapterCapabilities {
        &self.capabilities
    }
    pub fn auth_requirement(&self) -> &AuthRequirement {
        &self.auth_requirement
    }
    pub fn declared_egress(&self) -> &[DeclaredEgressTarget] {
        &self.declared_egress
    }
    pub fn required_credentials(&self) -> &[EgressCredentialHandle] {
        &self.required_credentials
    }

    /// Host-ingress routes this ProductAdapter section declares. Each carries a
    /// host-owned [`IngressRouteDescriptor`] and its verifying credential
    /// handles; the serve layer projects these into mounted routes. Empty for
    /// sections that declare no ingress (the common case today).
    pub fn host_ingress(&self) -> &[HostIngressRoute] {
        &self.host_ingress
    }

    fn validate(&self) -> Result<(), ProductAdapterSectionError> {
        validate_auth_requirement(&self.auth_requirement)?;
        let mut required = BTreeSet::new();
        for handle in &self.required_credentials {
            if !required.insert(handle.clone()) {
                return Err(ProductAdapterSectionError::DuplicateCredentialHandle {
                    handle: handle.clone(),
                });
            }
        }
        let mut pairs = BTreeSet::new();
        for target in &self.declared_egress {
            if let Some(handle) = target.credential_handle.as_ref()
                && !required.contains(handle)
            {
                return Err(
                    ProductAdapterSectionError::UndeclaredEgressCredentialHandle {
                        handle: handle.clone(),
                    },
                );
            }
            if !pairs.insert((target.host.clone(), target.credential_handle.clone())) {
                return Err(ProductAdapterSectionError::DuplicateEgressTarget);
            }
        }
        // Host-ingress credential coherence, fail closed. A route's declared
        // verifying credentials must line up with whether it is actually
        // authenticated, and every named handle must be declared in
        // `required_credentials` (mirroring the egress rule above, so ingress
        // handles flow into the same declared set installation bindings are
        // validated against). Route ids stay distinct within a section so a
        // mounted route can be addressed unambiguously.
        let mut route_ids: BTreeSet<&IngressRouteId> = BTreeSet::new();
        for route in &self.host_ingress {
            let route_id = route.descriptor.route_id();
            if !route_ids.insert(route_id) {
                return Err(ProductAdapterSectionError::DuplicateIngressRoute {
                    route_id: route_id.clone(),
                });
            }
            match route.descriptor.policy().auth() {
                // An auth-required route with no verifying credential is a route
                // nothing could authenticate — reject it.
                IngressAuthPolicy::Required { .. } => {
                    if route.credential_handles.is_empty() {
                        return Err(ProductAdapterSectionError::IngressRouteMissingCredential {
                            route_id: route_id.clone(),
                        });
                    }
                }
                // A public (no-auth) route is verified by nothing, so declaring a
                // credential handle on it is incoherent and misleading — a reader
                // would assume the route is authenticated by that credential.
                IngressAuthPolicy::Public { .. } => {
                    if !route.credential_handles.is_empty() {
                        return Err(
                            ProductAdapterSectionError::PublicIngressRouteHasCredential {
                                route_id: route_id.clone(),
                            },
                        );
                    }
                }
            }
            for handle in &route.credential_handles {
                if !required.contains(handle) {
                    return Err(
                        ProductAdapterSectionError::UndeclaredIngressCredentialHandle {
                            handle: handle.clone(),
                        },
                    );
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Why a declared `[product_adapter.*]` section is not a valid section.
///
/// Deserialization failures are not here: the caller owns the manifest text and
/// reports them in its own vocabulary.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProductAdapterSectionError {
    #[error("invalid {field}: {reason}")]
    InvalidValue { field: &'static str, reason: String },
    #[error("duplicate credential handle {handle}")]
    DuplicateCredentialHandle { handle: EgressCredentialHandle },
    #[error("duplicate egress target")]
    DuplicateEgressTarget,
    #[error("egress references undeclared credential handle {handle}")]
    UndeclaredEgressCredentialHandle { handle: EgressCredentialHandle },
    #[error("host-ingress route references undeclared credential handle {handle}")]
    UndeclaredIngressCredentialHandle { handle: EgressCredentialHandle },
    #[error("auth-required host-ingress route {route_id} declares no verifying credential handle")]
    IngressRouteMissingCredential { route_id: IngressRouteId },
    #[error(
        "public host-ingress route {route_id} declares a verifying credential handle but is not authenticated"
    )]
    PublicIngressRouteHasCredential { route_id: IngressRouteId },
    #[error("duplicate host-ingress route {route_id}")]
    DuplicateIngressRoute { route_id: IngressRouteId },
}

// ---------------------------------------------------------------------------
// Internal validation helpers
// ---------------------------------------------------------------------------

fn validate_auth_requirement(
    requirement: &AuthRequirement,
) -> Result<(), ProductAdapterSectionError> {
    match requirement {
        AuthRequirement::RequestSignature {
            header_name,
            timestamp_header_name,
        } => {
            validate_http_token("auth.header_name", header_name)?;
            if let Some(t) = timestamp_header_name.as_deref() {
                validate_http_token("auth.timestamp_header_name", t)?;
            }
        }
        AuthRequirement::SharedSecretHeader { header_name } => {
            validate_http_token("auth.header_name", header_name)?;
        }
        AuthRequirement::SessionCookie { name } => {
            validate_http_token("auth.name", name)?;
        }
        AuthRequirement::BearerToken => {}
    }
    Ok(())
}

fn validate_http_token(field: &'static str, value: &str) -> Result<(), ProductAdapterSectionError> {
    if value.is_empty() {
        return Err(ProductAdapterSectionError::InvalidValue {
            field,
            reason: "must not be empty".to_string(),
        });
    }
    for c in value.chars() {
        if !is_http_tchar(c) {
            return Err(ProductAdapterSectionError::InvalidValue {
                field,
                reason: format!(
                    "must be an RFC 7230 token (no CTL, whitespace, or separators); got {value:?}"
                ),
            });
        }
    }
    Ok(())
}

fn is_http_tchar(c: char) -> bool {
    matches!(
        c,
        '!' | '#' | '$' | '%' | '&' | '\'' | '*' | '+' | '-' | '.' | '^' | '_' | '`' | '|' | '~'
    ) || c.is_ascii_alphanumeric()
}

// ---------------------------------------------------------------------------
// Raw deserialization shapes
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeclaredCapabilities {
    flags: Vec<ProductCapabilityFlag>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeclaredCredential {
    handle: EgressCredentialHandle,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum DeclaredAuth {
    RequestSignature {
        header_name: String,
        #[serde(default)]
        timestamp_header_name: Option<String>,
    },
    SharedSecretHeader {
        header_name: String,
    },
    SessionCookie {
        name: String,
    },
    BearerToken,
}

impl DeclaredAuth {
    fn into_auth_requirement(self) -> Result<AuthRequirement, ProductAdapterSectionError> {
        let requirement = match self {
            Self::RequestSignature {
                header_name,
                timestamp_header_name,
            } => AuthRequirement::RequestSignature {
                header_name,
                timestamp_header_name,
            },
            Self::SharedSecretHeader { header_name } => {
                AuthRequirement::SharedSecretHeader { header_name }
            }
            Self::SessionCookie { name } => AuthRequirement::SessionCookie { name },
            Self::BearerToken => AuthRequirement::BearerToken,
        };
        validate_auth_requirement(&requirement)?;
        Ok(requirement)
    }
}

#[cfg(test)]
mod tests {
    //! Unit coverage for host-ingress credential coherence — the novel logic
    //! this schema adds on top of host_api's already-validated ingress
    //! descriptor. Descriptors are built in Rust (not TOML text) so these
    //! cases are robust to serde renames; the wire path is covered end-to-end
    //! by the registry's `product_adapter_manifest_ingestion` suite.
    use super::*;
    use ironclaw_host_api::{
        action::NetworkMethod,
        ingress::{
            AllowedEffectPath, AuditTraceClass, BodyLimitPolicy, CorsPolicy, IngressAuthScheme,
            IngressJustification, IngressPolicy, IngressPolicyParts, IngressScopeSource,
            ListenerClass, RateLimitPolicy, RateLimitScope, StreamingMode, WebSocketOriginPolicy,
        },
    };
    use serde::Serialize;
    use std::num::{NonZeroU32, NonZeroU64};

    /// A fail-closed public-webhook descriptor mirroring the values a channel
    /// package's events policy uses, parameterized by route id.
    fn webhook_descriptor(route_id: &str) -> IngressRouteDescriptor {
        let policy = IngressPolicy::new(IngressPolicyParts {
            listener_class: ListenerClass::PublicWebhook,
            auth: IngressAuthPolicy::Required {
                schemes: vec![IngressAuthScheme::WebhookSignature],
            },
            scope_source: IngressScopeSource::HostResolved,
            body_limit: BodyLimitPolicy::Limited {
                max_bytes: NonZeroU64::new(262_144).expect("nonzero"),
            },
            rate_limit: RateLimitPolicy::Limited {
                scope: RateLimitScope::Global,
                max_requests: NonZeroU32::new(600).expect("nonzero"),
                window_seconds: NonZeroU32::new(60).expect("nonzero"),
            },
            cors: CorsPolicy::NotApplicable,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::PublicCallback,
            effect_path: AllowedEffectPath::ProductSurface,
        })
        .expect("policy validates");
        IngressRouteDescriptor::new(
            route_id,
            NetworkMethod::Post,
            "/webhooks/example/updates",
            policy,
        )
        .expect("descriptor validates")
    }

    /// A valid public (no-auth) route, mirroring the SSO login mount's policy
    /// combination (LocalGateway + Public + PublicRoute + NoEffect).
    fn public_descriptor(route_id: &str) -> IngressRouteDescriptor {
        let policy = IngressPolicy::new(IngressPolicyParts {
            listener_class: ListenerClass::LocalGateway,
            auth: IngressAuthPolicy::Public {
                justification: IngressJustification::new("ingress", "public test route")
                    .expect("justification"),
            },
            scope_source: IngressScopeSource::PublicRoute,
            body_limit: BodyLimitPolicy::Limited {
                max_bytes: NonZeroU64::new(4096).expect("nonzero"),
            },
            rate_limit: RateLimitPolicy::Limited {
                scope: RateLimitScope::PerIp,
                max_requests: NonZeroU32::new(60).expect("nonzero"),
                window_seconds: NonZeroU32::new(60).expect("nonzero"),
            },
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::PublicCallback,
            effect_path: AllowedEffectPath::NoEffect,
        })
        .expect("public policy validates");
        IngressRouteDescriptor::new(route_id, NetworkMethod::Post, "/public/callback", policy)
            .expect("descriptor validates")
    }

    #[derive(Serialize)]
    struct RouteFixture {
        descriptor: IngressRouteDescriptor,
        credential_handles: Vec<String>,
    }

    /// Build a ProductAdapter section declaration with a valid base and the
    /// given host-ingress routes, then run it through the real resolution.
    fn project(
        routes: Vec<RouteFixture>,
    ) -> Result<ProductAdapterSection, ProductAdapterSectionError> {
        let mut value: toml::Value = toml::from_str(
            r#"
surface_kind = "external_channel"
[auth]
kind = "shared_secret_header"
header_name = "X-Example-Secret-Token"
[capabilities]
flags = ["inbound_messages"]
[[required_credentials]]
handle = "example_bot_token"
"#,
        )
        .expect("base section parses");
        let host_ingress = toml::Value::try_from(routes).expect("routes serialize");
        value
            .as_table_mut()
            .expect("section is a table")
            .insert("host_ingress".to_string(), host_ingress);

        let declaration: ProductAdapterSectionDeclaration =
            value.try_into().expect("declaration deserializes");
        let extension_id = ExtensionId::new("example-v2").expect("extension id");
        declaration.resolve(&extension_id, "inbound")
    }

    fn route(route_id: &str, credential_handles: &[&str]) -> RouteFixture {
        RouteFixture {
            descriptor: webhook_descriptor(route_id),
            credential_handles: credential_handles.iter().map(|h| h.to_string()).collect(),
        }
    }

    /// Resolve an arbitrary `[product_adapter.*]` section body through the
    /// real wire path, so these cases exercise deserialization + `resolve` +
    /// `validate` exactly as manifest ingestion does.
    fn project_toml(section: &str) -> Result<ProductAdapterSection, ProductAdapterSectionError> {
        let declaration: ProductAdapterSectionDeclaration =
            toml::from_str(section).expect("section declaration deserializes");
        let extension_id = ExtensionId::new("example-v2").expect("extension id");
        declaration.resolve(&extension_id, "inbound")
    }

    /// The cross-field invariants that came over from
    /// `ironclaw_product::adapter_registry` with WS5.
    ///
    /// Three of them arrived with no assertion anywhere: a duplicate
    /// credential handle, a duplicate `(host, handle)` egress pair, and the
    /// RFC 7230 token rule on every header/cookie name the auth requirement
    /// carries. Each is a fail-closed rule whose regression is silent — a
    /// duplicate handle would resolve to one binding, a duplicate egress pair
    /// would double-declare a host, and a header name with a separator or
    /// control character would be split or smuggled downstream — so each is
    /// pinned here rather than left to the ingestion suite, which covers only
    /// the ingress half.
    #[test]
    fn section_rejects_duplicate_handles_duplicate_egress_and_non_token_names() {
        const VALID_BASE: &str = r#"
surface_kind = "external_channel"
[auth]
kind = "shared_secret_header"
header_name = "X-Example-Secret-Token"
[capabilities]
flags = ["inbound_messages"]
[[required_credentials]]
handle = "example_bot_token"
"#;
        project_toml(VALID_BASE)
            .expect("the base section must be valid, or nothing below proves anything");

        // Duplicate credential handle.
        let err = project_toml(&format!(
            "{VALID_BASE}\n[[required_credentials]]\nhandle = \"example_bot_token\"\n"
        ))
        .expect_err("a repeated credential handle must reject");
        assert!(
            matches!(
                err,
                ProductAdapterSectionError::DuplicateCredentialHandle { ref handle }
                    if handle.as_str() == "example_bot_token"
            ),
            "got {err:?}"
        );

        // Duplicate `(host, credential_handle)` egress pair. The same host with
        // a *different* handle is legitimate, so only the exact pair collides.
        let err = project_toml(&format!(
            "{VALID_BASE}\n\
             [[egress]]\nhost = \"api.example.com\"\ncredential_handle = \"example_bot_token\"\n\
             [[egress]]\nhost = \"api.example.com\"\ncredential_handle = \"example_bot_token\"\n"
        ))
        .expect_err("a repeated (host, handle) egress pair must reject");
        assert!(
            matches!(err, ProductAdapterSectionError::DuplicateEgressTarget),
            "got {err:?}"
        );
        project_toml(&format!(
            "{VALID_BASE}\n\
             [[required_credentials]]\nhandle = \"example_other_token\"\n\
             [[egress]]\nhost = \"api.example.com\"\ncredential_handle = \"example_bot_token\"\n\
             [[egress]]\nhost = \"api.example.com\"\ncredential_handle = \"example_other_token\"\n"
        ))
        .expect("the same host under two distinct handles is not a duplicate");

        // Egress naming a handle that was never declared.
        let err = project_toml(&format!(
            "{VALID_BASE}\n[[egress]]\nhost = \"api.example.com\"\ncredential_handle = \"not_declared\"\n"
        ))
        .expect_err("egress must not name an undeclared handle");
        assert!(
            matches!(
                err,
                ProductAdapterSectionError::UndeclaredEgressCredentialHandle { ref handle }
                    if handle.as_str() == "not_declared"
            ),
            "got {err:?}"
        );

        // RFC 7230 token rule, on every field `validate_auth_requirement`
        // routes through `validate_http_token` — including
        // `auth.timestamp_header_name`, the optional one a rename could drop
        // from validation without any other case noticing.
        for (label, auth, field) in [
            (
                "empty shared-secret header",
                "kind = \"shared_secret_header\"\nheader_name = \"\"",
                "auth.header_name",
            ),
            (
                "separator in shared-secret header",
                "kind = \"shared_secret_header\"\nheader_name = \"X-Bad Header\"",
                "auth.header_name",
            ),
            (
                "control char in signature header",
                "kind = \"request_signature\"\nheader_name = \"X-Sig\\u0007\"",
                "auth.header_name",
            ),
            (
                "separator in timestamp header",
                "kind = \"request_signature\"\nheader_name = \"X-Sig\"\ntimestamp_header_name = \"X-Ts;v=1\"",
                "auth.timestamp_header_name",
            ),
            (
                "empty timestamp header",
                "kind = \"request_signature\"\nheader_name = \"X-Sig\"\ntimestamp_header_name = \"\"",
                "auth.timestamp_header_name",
            ),
            (
                "separator in session cookie name",
                "kind = \"session_cookie\"\nname = \"sid=x\"",
                "auth.name",
            ),
        ] {
            let section = format!(
                "surface_kind = \"external_channel\"\n\
                 [auth]\n{auth}\n\
                 [capabilities]\nflags = [\"inbound_messages\"]\n"
            );
            let err = project_toml(&section)
                .expect_err(&format!("{label}: a non-token name must reject"));
            match err {
                ProductAdapterSectionError::InvalidValue {
                    field: reported, ..
                } => assert_eq!(
                    reported, field,
                    "{label}: the rejection must name the field it validated"
                ),
                other => panic!("{label}: expected InvalidValue, got {other:?}"),
            }
        }

        // A valid `timestamp_header_name` still resolves, so the rule above is
        // rejecting the token shape and not the field itself.
        project_toml(
            "surface_kind = \"external_channel\"\n\
             [auth]\nkind = \"request_signature\"\nheader_name = \"X-Sig\"\n\
             timestamp_header_name = \"X-Timestamp\"\n\
             [capabilities]\nflags = [\"inbound_messages\"]\n",
        )
        .expect("a valid RFC 7230 timestamp header must resolve");
    }

    #[test]
    fn host_ingress_route_projects_descriptor_and_handles() {
        let section = project(vec![route("example.updates", &["example_bot_token"])])
            .expect("valid section projects");
        assert_eq!(section.host_ingress().len(), 1);
        let projected = &section.host_ingress()[0];
        assert_eq!(
            projected.descriptor().route_id().as_str(),
            "example.updates"
        );
        assert_eq!(
            projected.descriptor().route_pattern().as_str(),
            "/webhooks/example/updates"
        );
        assert_eq!(projected.credential_handles().len(), 1);
        assert_eq!(
            projected.credential_handles()[0].as_str(),
            "example_bot_token"
        );
    }

    #[test]
    fn host_ingress_undeclared_credential_handle_rejected() {
        let err = project(vec![route("example.updates", &["not_declared_token"])])
            .expect_err("undeclared handle must reject");
        assert!(
            matches!(
                err,
                ProductAdapterSectionError::UndeclaredIngressCredentialHandle { .. }
            ),
            "got {err:?}"
        );
    }

    #[test]
    fn host_ingress_auth_required_route_needs_credential() {
        // Fail closed: an auth-required route with no verifying credential
        // handle must reject, not mount a route nothing can authenticate.
        let err = project(vec![route("example.updates", &[])])
            .expect_err("auth-required route without a credential must reject");
        assert!(
            matches!(
                err,
                ProductAdapterSectionError::IngressRouteMissingCredential { .. }
            ),
            "got {err:?}"
        );
    }

    #[test]
    fn host_ingress_duplicate_route_id_rejected() {
        let err = project(vec![
            route("example.updates", &["example_bot_token"]),
            route("example.updates", &["example_bot_token"]),
        ])
        .expect_err("duplicate route id must reject");
        assert!(
            matches!(
                err,
                ProductAdapterSectionError::DuplicateIngressRoute { .. }
            ),
            "got {err:?}"
        );
    }

    #[test]
    fn host_ingress_public_route_must_not_declare_credentials() {
        // Fail closed on the dual of the auth-required rule: a public (no-auth)
        // route is verified by nothing, so declaring a credential handle on it
        // is incoherent and would mislead a reader into assuming it is
        // authenticated.
        let err = project(vec![RouteFixture {
            descriptor: public_descriptor("public.callback"),
            credential_handles: vec!["example_bot_token".to_string()],
        }])
        .expect_err("public route with a credential handle must reject");
        assert!(
            matches!(
                err,
                ProductAdapterSectionError::PublicIngressRouteHasCredential { .. }
            ),
            "got {err:?}"
        );
    }

    #[test]
    fn host_ingress_public_route_without_credentials_projects() {
        // The complement: a public route that declares no credentials is valid.
        let section = project(vec![RouteFixture {
            descriptor: public_descriptor("public.callback"),
            credential_handles: vec![],
        }])
        .expect("public route with no credentials projects");
        assert_eq!(section.host_ingress().len(), 1);
    }
}
