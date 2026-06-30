//! Host-owned HTTP ingress contract vocabulary.
//!
//! Product/API crates use these types to describe the HTTP routes they want the
//! host to mount. The types deliberately stop at vocabulary and validation:
//! listener binding, Axum router composition, auth enforcement, scope
//! extraction, limits, CORS/Origin policy, audit, and effect dispatch all remain
//! host-composition responsibilities.

use std::num::{NonZeroU32, NonZeroU64};

use serde::{Deserialize, Serialize};

use crate::{
    CapabilityId, HostApiError, HostPortId, NetworkMethod,
    dotted_id::{PrefixRule, VersionRule, validate_dotted_id},
};

fn validate_ingress_route_id(value: &str) -> Result<(), HostApiError> {
    validate_dotted_id(
        "ingress_route",
        value,
        2,
        "must have at least surface and route segments",
        PrefixRule::Any,
        VersionRule::Unversioned,
    )
}

fn validate_route_pattern(value: &str) -> Result<(), HostApiError> {
    if value.is_empty() {
        return Err(HostApiError::invalid_path(value, "must not be empty"));
    }
    if value.len() > 512 {
        return Err(HostApiError::invalid_path(
            value,
            "must be at most 512 bytes",
        ));
    }
    if value.trim() != value {
        return Err(HostApiError::invalid_path(
            value,
            "leading or trailing whitespace is not allowed",
        ));
    }
    if !value.starts_with('/') {
        return Err(HostApiError::invalid_path(
            value,
            "must be an absolute local path pattern",
        ));
    }
    if value.starts_with("//") || value.contains("://") {
        return Err(HostApiError::invalid_path(
            value,
            "must not be a URL or network-path reference",
        ));
    }
    if value.contains("//") {
        return Err(HostApiError::invalid_path(
            value,
            "duplicate slashes are not allowed",
        ));
    }
    if value.contains('?') || value.contains('#') {
        return Err(HostApiError::invalid_path(
            value,
            "query strings and fragments are not part of route patterns",
        ));
    }
    if value.contains('\\') {
        return Err(HostApiError::invalid_path(
            value,
            "backslashes are not allowed",
        ));
    }
    if value.chars().any(|ch| ch == '\0' || ch.is_control()) {
        return Err(HostApiError::invalid_path(
            value,
            "NUL/control characters are not allowed",
        ));
    }
    if value.split('/').any(|segment| segment == "..") {
        return Err(HostApiError::invalid_path(
            value,
            "path traversal segments are not allowed",
        ));
    }
    Ok(())
}

fn validate_justification(kind: &'static str, value: &str) -> Result<(), HostApiError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(HostApiError::invariant(format!(
            "{kind} justification must not be empty"
        )));
    }
    if trimmed != value {
        return Err(HostApiError::invariant(format!(
            "{kind} justification must not contain leading or trailing whitespace"
        )));
    }
    if value.len() > 512 {
        return Err(HostApiError::invariant(format!(
            "{kind} justification must be at most 512 bytes"
        )));
    }
    if value.chars().any(|ch| ch == '\0' || ch.is_control()) {
        return Err(HostApiError::invariant(format!(
            "{kind} justification must not contain NUL/control characters"
        )));
    }
    Ok(())
}

/// Stable route identifier for a host-mounted HTTP ingress surface.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IngressRouteId(String);

impl IngressRouteId {
    pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
        let value = value.into();
        validate_ingress_route_id(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Display for IngressRouteId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for IngressRouteId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for IngressRouteId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Host-local route path pattern.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IngressRoutePattern(String);

impl IngressRoutePattern {
    pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
        let value = value.into();
        validate_route_pattern(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Display for IngressRoutePattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for IngressRoutePattern {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for IngressRoutePattern {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Human-readable reason for intentionally widening ingress exposure.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IngressJustification(String);

impl IngressJustification {
    pub fn new(kind: &'static str, value: impl Into<String>) -> Result<Self, HostApiError> {
        let value = value.into();
        validate_justification(kind, &value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Display for IngressJustification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for IngressJustification {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for IngressJustification {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new("ingress", value).map_err(serde::de::Error::custom)
    }
}

/// Route policy after host composition has resolved any inheritance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IngressPolicy {
    listener_class: ListenerClass,
    auth: IngressAuthPolicy,
    scope_source: IngressScopeSource,
    body_limit: BodyLimitPolicy,
    rate_limit: RateLimitPolicy,
    cors: CorsPolicy,
    websocket_origin: WebSocketOriginPolicy,
    streaming: StreamingMode,
    audit: AuditTraceClass,
    effect_path: AllowedEffectPath,
}

impl<'de> Deserialize<'de> for IngressPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Helper {
            listener_class: ListenerClass,
            auth: IngressAuthPolicy,
            scope_source: IngressScopeSource,
            body_limit: BodyLimitPolicy,
            rate_limit: RateLimitPolicy,
            cors: CorsPolicy,
            websocket_origin: WebSocketOriginPolicy,
            streaming: StreamingMode,
            audit: AuditTraceClass,
            effect_path: AllowedEffectPath,
        }

        let helper = Helper::deserialize(deserializer)?;
        Self::new(IngressPolicyParts {
            listener_class: helper.listener_class,
            auth: helper.auth,
            scope_source: helper.scope_source,
            body_limit: helper.body_limit,
            rate_limit: helper.rate_limit,
            cors: helper.cors,
            websocket_origin: helper.websocket_origin,
            streaming: helper.streaming,
            audit: helper.audit,
            effect_path: helper.effect_path,
        })
        .map_err(serde::de::Error::custom)
    }
}

impl IngressPolicy {
    pub fn new(parts: IngressPolicyParts) -> Result<Self, HostApiError> {
        validate_auth_policy(&parts.auth)?;
        validate_auth_scope(parts.listener_class, &parts.auth, parts.scope_source)?;
        validate_effect_scope(parts.scope_source, &parts.effect_path)?;
        validate_listener_auth(parts.listener_class, &parts.auth, &parts.effect_path)?;
        validate_streaming_origin(parts.streaming, parts.websocket_origin)?;

        Ok(Self {
            listener_class: parts.listener_class,
            auth: parts.auth,
            scope_source: parts.scope_source,
            body_limit: parts.body_limit,
            rate_limit: parts.rate_limit,
            cors: parts.cors,
            websocket_origin: parts.websocket_origin,
            streaming: parts.streaming,
            audit: parts.audit,
            effect_path: parts.effect_path,
        })
    }

    pub fn listener_class(&self) -> ListenerClass {
        self.listener_class
    }

    pub fn auth(&self) -> &IngressAuthPolicy {
        &self.auth
    }

    pub fn scope_source(&self) -> IngressScopeSource {
        self.scope_source
    }

    pub fn body_limit(&self) -> BodyLimitPolicy {
        self.body_limit
    }

    pub fn rate_limit(&self) -> &RateLimitPolicy {
        &self.rate_limit
    }

    pub fn cors(&self) -> CorsPolicy {
        self.cors
    }

    pub fn websocket_origin(&self) -> WebSocketOriginPolicy {
        self.websocket_origin
    }

    pub fn streaming(&self) -> StreamingMode {
        self.streaming
    }

    pub fn audit(&self) -> AuditTraceClass {
        self.audit
    }

    pub fn effect_path(&self) -> &AllowedEffectPath {
        &self.effect_path
    }
}

/// Construction parts for [`IngressPolicy`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngressPolicyParts {
    pub listener_class: ListenerClass,
    pub auth: IngressAuthPolicy,
    pub scope_source: IngressScopeSource,
    pub body_limit: BodyLimitPolicy,
    pub rate_limit: RateLimitPolicy,
    pub cors: CorsPolicy,
    pub websocket_origin: WebSocketOriginPolicy,
    pub streaming: StreamingMode,
    pub audit: AuditTraceClass,
    pub effect_path: AllowedEffectPath,
}

/// Complete route descriptor handed to host composition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IngressRouteDescriptor {
    route_id: IngressRouteId,
    method: NetworkMethod,
    route_pattern: IngressRoutePattern,
    policy: IngressPolicy,
}

impl<'de> Deserialize<'de> for IngressRouteDescriptor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Helper {
            route_id: IngressRouteId,
            method: NetworkMethod,
            route_pattern: IngressRoutePattern,
            policy: IngressPolicy,
        }

        let helper = Helper::deserialize(deserializer)?;
        Ok(Self {
            route_id: helper.route_id,
            method: helper.method,
            route_pattern: helper.route_pattern,
            policy: helper.policy,
        })
    }
}

impl IngressRouteDescriptor {
    pub fn new(
        route_id: impl Into<String>,
        method: NetworkMethod,
        route_pattern: impl Into<String>,
        policy: IngressPolicy,
    ) -> Result<Self, HostApiError> {
        Ok(Self {
            route_id: IngressRouteId::new(route_id)?,
            method,
            route_pattern: IngressRoutePattern::new(route_pattern)?,
            policy,
        })
    }

    pub fn route_id(&self) -> &IngressRouteId {
        &self.route_id
    }

    pub fn method(&self) -> NetworkMethod {
        self.method
    }

    pub fn route_pattern(&self) -> &IngressRoutePattern {
        &self.route_pattern
    }

    pub fn policy(&self) -> &IngressPolicy {
        &self.policy
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListenerClass {
    LocalGateway,
    PublicWebhook,
    OAuthCallback,
    InternalWorker,
    TestOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum IngressAuthPolicy {
    Required { schemes: Vec<IngressAuthScheme> },
    Public { justification: IngressJustification },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IngressAuthScheme {
    BearerToken,
    SessionCookie,
    Oidc,
    CsrfToken,
    WebhookSignature,
    OAuthState,
    InternalToken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IngressScopeSource {
    AuthenticatedCaller,
    HostResolved,
    RouteBinding,
    PublicRoute,
    TestFixture,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum BodyLimitPolicy {
    NoBody,
    Limited { max_bytes: NonZeroU64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum RateLimitPolicy {
    Limited {
        scope: RateLimitScope,
        max_requests: NonZeroU32,
        window_seconds: NonZeroU32,
    },
    Disabled {
        justification: IngressJustification,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitScope {
    PerCaller,
    PerTenant,
    PerIp,
    PerRoute,
    Global,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CorsPolicy {
    NotApplicable,
    SameOriginOnly,
    HostConfiguredAllowlist,
    PublicReadOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebSocketOriginPolicy {
    NotApplicable,
    SameOriginRequired,
    HostConfiguredAllowlist,
    LocalhostAllowed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamingMode {
    None,
    Sse,
    WebSocket,
    LongPoll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditTraceClass {
    HealthCheck,
    UserAction,
    PublicCallback,
    StreamingSubscription,
    InternalControl,
    TestOnly,
}

/// Host-mediated path that a route handler may enter after ingress policy.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum AllowedEffectPath {
    NoEffect,
    ProjectionOnly,
    ProductWorkflow,
    TurnCoordinator,
    HostPort { id: HostPortId },
    CapabilityHost { capability_id: CapabilityId },
}

fn validate_auth_policy(auth: &IngressAuthPolicy) -> Result<(), HostApiError> {
    match auth {
        IngressAuthPolicy::Required { schemes } => {
            if schemes.is_empty() {
                return Err(HostApiError::invariant(
                    "required ingress auth must list at least one scheme",
                ));
            }
        }
        IngressAuthPolicy::Public { .. } => {}
    }
    Ok(())
}

fn validate_auth_scope(
    listener_class: ListenerClass,
    auth: &IngressAuthPolicy,
    scope_source: IngressScopeSource,
) -> Result<(), HostApiError> {
    match (auth, scope_source) {
        (IngressAuthPolicy::Public { .. }, IngressScopeSource::AuthenticatedCaller) => Err(
            HostApiError::invariant("public ingress auth must not use authenticated caller scope"),
        ),
        (IngressAuthPolicy::Required { .. }, IngressScopeSource::PublicRoute) => Err(
            HostApiError::invariant("required ingress auth must not use public route scope"),
        ),
        (_, IngressScopeSource::TestFixture) if listener_class != ListenerClass::TestOnly => {
            Err(HostApiError::invariant(
                "test fixture ingress scope requires a test-only listener class",
            ))
        }
        _ => Ok(()),
    }
}

fn validate_effect_scope(
    scope_source: IngressScopeSource,
    effect_path: &AllowedEffectPath,
) -> Result<(), HostApiError> {
    if scope_source == IngressScopeSource::PublicRoute && is_effectful_path(effect_path) {
        return Err(HostApiError::invariant(
            "public route scope must not enter effectful host paths",
        ));
    }
    Ok(())
}

fn validate_listener_auth(
    listener_class: ListenerClass,
    auth: &IngressAuthPolicy,
    effect_path: &AllowedEffectPath,
) -> Result<(), HostApiError> {
    if listener_class == ListenerClass::OAuthCallback
        && matches!(auth, IngressAuthPolicy::Public { .. })
        && matches!(effect_path, AllowedEffectPath::NoEffect)
    {
        return Ok(());
    }

    match listener_class {
        ListenerClass::PublicWebhook => require_auth_scheme(
            auth,
            IngressAuthScheme::WebhookSignature,
            "public webhook ingress requires webhook signature auth",
        ),
        ListenerClass::InternalWorker => require_auth_scheme(
            auth,
            IngressAuthScheme::InternalToken,
            "internal worker ingress requires internal token auth",
        ),
        ListenerClass::LocalGateway if is_effectful_path(effect_path) => require_any_auth_scheme(
            auth,
            &[
                IngressAuthScheme::BearerToken,
                IngressAuthScheme::SessionCookie,
            ],
            "effectful local gateway ingress requires bearer token or session cookie auth",
        ),
        ListenerClass::OAuthCallback => require_auth_scheme(
            auth,
            IngressAuthScheme::OAuthState,
            "oauth callback ingress requires oauth state auth unless it is public no-effect",
        ),
        ListenerClass::LocalGateway | ListenerClass::TestOnly => Ok(()),
    }
}

fn require_auth_scheme(
    auth: &IngressAuthPolicy,
    required: IngressAuthScheme,
    reason: &'static str,
) -> Result<(), HostApiError> {
    require_any_auth_scheme(auth, &[required], reason)
}

fn require_any_auth_scheme(
    auth: &IngressAuthPolicy,
    required: &[IngressAuthScheme],
    reason: &'static str,
) -> Result<(), HostApiError> {
    if auth_has_any_scheme(auth, required) {
        Ok(())
    } else {
        Err(HostApiError::invariant(reason))
    }
}

fn auth_has_any_scheme(auth: &IngressAuthPolicy, required: &[IngressAuthScheme]) -> bool {
    matches!(
        auth,
        IngressAuthPolicy::Required { schemes }
            if required.iter().any(|scheme| schemes.contains(scheme))
    )
}

fn is_effectful_path(effect_path: &AllowedEffectPath) -> bool {
    !matches!(
        effect_path,
        AllowedEffectPath::NoEffect | AllowedEffectPath::ProjectionOnly
    )
}

fn validate_streaming_origin(
    streaming: StreamingMode,
    websocket_origin: WebSocketOriginPolicy,
) -> Result<(), HostApiError> {
    match (streaming, websocket_origin) {
        (StreamingMode::WebSocket, WebSocketOriginPolicy::NotApplicable) => {
            Err(HostApiError::invariant(
                "websocket ingress routes must declare a websocket origin policy",
            ))
        }
        (StreamingMode::WebSocket, _) => Ok(()),
        (_, WebSocketOriginPolicy::NotApplicable) => Ok(()),
        _ => Err(HostApiError::invariant(
            "non-websocket ingress routes must not declare a websocket origin policy",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nz32(value: u32) -> NonZeroU32 {
        NonZeroU32::new(value).expect("test value must be non-zero")
    }

    fn nz64(value: u64) -> NonZeroU64 {
        NonZeroU64::new(value).expect("test value must be non-zero")
    }

    fn justification() -> IngressJustification {
        IngressJustification::new("test", "public OAuth provider callback")
            .expect("valid justification")
    }

    fn base_policy_parts() -> IngressPolicyParts {
        IngressPolicyParts {
            listener_class: ListenerClass::LocalGateway,
            auth: IngressAuthPolicy::Required {
                schemes: vec![IngressAuthScheme::BearerToken],
            },
            scope_source: IngressScopeSource::AuthenticatedCaller,
            body_limit: BodyLimitPolicy::Limited {
                max_bytes: nz64(16 * 1024),
            },
            rate_limit: RateLimitPolicy::Limited {
                scope: RateLimitScope::PerCaller,
                max_requests: nz32(30),
                window_seconds: nz32(60),
            },
            cors: CorsPolicy::SameOriginOnly,
            websocket_origin: WebSocketOriginPolicy::NotApplicable,
            streaming: StreamingMode::None,
            audit: AuditTraceClass::UserAction,
            effect_path: AllowedEffectPath::ProductWorkflow,
        }
    }

    fn public_route_parts(
        listener_class: ListenerClass,
        effect_path: AllowedEffectPath,
    ) -> IngressPolicyParts {
        let mut parts = base_policy_parts();
        parts.listener_class = listener_class;
        parts.auth = IngressAuthPolicy::Public {
            justification: justification(),
        };
        parts.scope_source = IngressScopeSource::PublicRoute;
        parts.audit = match listener_class {
            ListenerClass::TestOnly => AuditTraceClass::TestOnly,
            _ => AuditTraceClass::PublicCallback,
        };
        parts.effect_path = effect_path;
        parts
    }

    fn host_resolved_required_parts(
        listener_class: ListenerClass,
        schemes: Vec<IngressAuthScheme>,
        effect_path: AllowedEffectPath,
    ) -> IngressPolicyParts {
        let mut parts = base_policy_parts();
        parts.listener_class = listener_class;
        parts.auth = IngressAuthPolicy::Required { schemes };
        parts.scope_source = IngressScopeSource::HostResolved;
        parts.effect_path = effect_path;
        parts
    }

    fn valid_policy() -> IngressPolicy {
        IngressPolicy::new(base_policy_parts()).expect("valid policy")
    }

    #[test]
    fn route_pattern_must_be_local_absolute_path() {
        for invalid in [
            "",
            "api/chat",
            "https://example.com/api",
            "//example.com/api",
            " /api/chat",
            "/api/chat ",
            "/api//chat",
            "/api/../chat",
            "/../api/chat",
            "/api/chat?debug=true",
            "/api/chat#fragment",
            "/api\\chat",
            "/api/\nchat",
        ] {
            assert!(
                IngressRoutePattern::new(invalid).is_err(),
                "{invalid:?} must reject"
            );
        }

        let pattern = IngressRoutePattern::new("/api/chat/v2/messages")
            .expect("absolute local route pattern should pass");
        assert_eq!(pattern.as_str(), "/api/chat/v2/messages");
    }

    #[test]
    fn public_auth_requires_justification() {
        let empty = IngressJustification::new("public route", "  ")
            .expect_err("empty public justification must reject");
        assert!(empty.to_string().contains("must not be empty"));

        let leading = IngressJustification::new("public route", " operational exception")
            .expect_err("leading whitespace must reject");
        assert!(leading.to_string().contains("leading or trailing"));

        let trailing = IngressJustification::new("public route", "operational exception ")
            .expect_err("trailing whitespace must reject");
        assert!(trailing.to_string().contains("leading or trailing"));

        IngressPolicy::new(public_route_parts(
            ListenerClass::OAuthCallback,
            AllowedEffectPath::NoEffect,
        ))
        .expect("public route with justification should pass");
    }

    #[test]
    fn auth_policy_must_match_scope_source() {
        let mut public_with_authenticated_scope = base_policy_parts();
        public_with_authenticated_scope.auth = IngressAuthPolicy::Public {
            justification: justification(),
        };
        let err = IngressPolicy::new(public_with_authenticated_scope)
            .expect_err("public auth must reject authenticated caller scope");
        assert!(err.to_string().contains("public ingress auth"));

        let mut required_with_public_scope = base_policy_parts();
        required_with_public_scope.scope_source = IngressScopeSource::PublicRoute;
        let err = IngressPolicy::new(required_with_public_scope)
            .expect_err("required auth must reject public route scope");
        assert!(err.to_string().contains("required ingress auth"));

        let mut test_scope_on_non_test_listener = base_policy_parts();
        test_scope_on_non_test_listener.scope_source = IngressScopeSource::TestFixture;
        let err = IngressPolicy::new(test_scope_on_non_test_listener)
            .expect_err("test fixture scope must reject non-test listeners");
        assert!(err.to_string().contains("test fixture"));

        let mut test_only = base_policy_parts();
        test_only.listener_class = ListenerClass::TestOnly;
        test_only.scope_source = IngressScopeSource::TestFixture;
        test_only.audit = AuditTraceClass::TestOnly;
        IngressPolicy::new(test_only).expect("test-only listener may use test fixture scope");
    }

    #[test]
    fn public_route_scope_cannot_enter_effectful_paths() {
        let effectful_paths = [
            AllowedEffectPath::ProductWorkflow,
            AllowedEffectPath::TurnCoordinator,
            AllowedEffectPath::HostPort {
                id: HostPortId::new("host.storage.sql_transaction.first_party")
                    .expect("valid host port"),
            },
            AllowedEffectPath::CapabilityHost {
                capability_id: CapabilityId::new("builtin.read_file").expect("valid capability"),
            },
        ];

        for effect_path in effectful_paths {
            let err = IngressPolicy::new(public_route_parts(ListenerClass::TestOnly, effect_path))
                .expect_err("public route scope must reject effectful host paths");
            assert!(err.to_string().contains("public route scope"));
        }

        for effect_path in [
            AllowedEffectPath::NoEffect,
            AllowedEffectPath::ProjectionOnly,
        ] {
            IngressPolicy::new(public_route_parts(ListenerClass::TestOnly, effect_path))
                .expect("public route scope may describe non-effectful routes");
        }
    }

    #[test]
    fn listener_class_must_match_network_auth_mechanism() {
        for (listener_class, schemes, effect_path, expected) in [
            (
                ListenerClass::PublicWebhook,
                vec![IngressAuthScheme::BearerToken],
                AllowedEffectPath::NoEffect,
                "webhook signature",
            ),
            (
                ListenerClass::InternalWorker,
                vec![IngressAuthScheme::BearerToken],
                AllowedEffectPath::NoEffect,
                "internal token",
            ),
            (
                ListenerClass::LocalGateway,
                vec![IngressAuthScheme::Oidc],
                AllowedEffectPath::ProductWorkflow,
                "local gateway",
            ),
            (
                ListenerClass::OAuthCallback,
                vec![IngressAuthScheme::BearerToken],
                AllowedEffectPath::NoEffect,
                "oauth state",
            ),
        ] {
            let err = IngressPolicy::new(host_resolved_required_parts(
                listener_class,
                schemes,
                effect_path,
            ))
            .expect_err("listener class must require its matching auth scheme");
            assert!(err.to_string().contains(expected));
        }

        for (listener_class, schemes) in [
            (
                ListenerClass::PublicWebhook,
                vec![IngressAuthScheme::WebhookSignature],
            ),
            (
                ListenerClass::InternalWorker,
                vec![IngressAuthScheme::InternalToken],
            ),
            (
                ListenerClass::OAuthCallback,
                vec![IngressAuthScheme::OAuthState],
            ),
        ] {
            IngressPolicy::new(host_resolved_required_parts(
                listener_class,
                schemes,
                AllowedEffectPath::NoEffect,
            ))
            .expect("matching auth scheme satisfies listener class");
        }

        IngressPolicy::new(public_route_parts(
            ListenerClass::OAuthCallback,
            AllowedEffectPath::NoEffect,
        ))
        .expect("oauth callback may be public only when it has no effect");
    }

    #[test]
    fn websocket_routes_require_origin_policy() {
        let mut missing = base_policy_parts();
        missing.streaming = StreamingMode::WebSocket;
        let err = IngressPolicy::new(missing).expect_err("missing origin policy must reject");
        assert!(err.to_string().contains("websocket origin policy"));

        let mut invalid_non_ws = base_policy_parts();
        invalid_non_ws.websocket_origin = WebSocketOriginPolicy::SameOriginRequired;
        let err = IngressPolicy::new(invalid_non_ws)
            .expect_err("non-websocket route must reject websocket origin policy");
        assert!(err.to_string().contains("non-websocket"));

        let mut valid = base_policy_parts();
        valid.streaming = StreamingMode::WebSocket;
        valid.websocket_origin = WebSocketOriginPolicy::SameOriginRequired;
        IngressPolicy::new(valid).expect("websocket origin policy should pass");
    }

    #[test]
    fn required_auth_must_name_at_least_one_scheme() {
        let mut parts = base_policy_parts();
        parts.auth = IngressAuthPolicy::Required {
            schemes: Vec::new(),
        };
        let err = IngressPolicy::new(parts).expect_err("empty auth scheme list must reject");
        assert!(err.to_string().contains("at least one scheme"));
    }

    #[test]
    fn zero_body_or_rate_limits_fail_deserialization() {
        let zero_body = r#"{"type":"limited","max_bytes":0}"#;
        assert!(
            serde_json::from_str::<BodyLimitPolicy>(zero_body).is_err(),
            "zero body limit must reject"
        );

        let zero_rate =
            r#"{"type":"limited","scope":"per_caller","max_requests":0,"window_seconds":60}"#;
        assert!(
            serde_json::from_str::<RateLimitPolicy>(zero_rate).is_err(),
            "zero rate limit must reject"
        );

        let disabled_without_reason = r#"{"type":"disabled","justification":""}"#;
        assert!(
            serde_json::from_str::<RateLimitPolicy>(disabled_without_reason).is_err(),
            "disabled rate limit must justify the exception"
        );
    }

    #[test]
    fn descriptor_round_trips_through_validated_wire_shape() {
        let descriptor = IngressRouteDescriptor::new(
            "web_chat.send",
            NetworkMethod::Post,
            "/api/chat/v2/messages",
            valid_policy(),
        )
        .expect("valid descriptor");

        let json = serde_json::to_value(&descriptor).expect("serialize descriptor");
        assert_eq!(json["route_id"], "web_chat.send");
        assert_eq!(json["method"], "post");
        assert_eq!(json["route_pattern"], "/api/chat/v2/messages");
        assert_eq!(json["policy"]["streaming"], "none");

        let reparsed: IngressRouteDescriptor =
            serde_json::from_value(json).expect("validated descriptor should deserialize");
        assert_eq!(reparsed.route_id().as_str(), "web_chat.send");
        assert_eq!(reparsed.method(), NetworkMethod::Post);
        assert_eq!(reparsed.route_pattern().as_str(), "/api/chat/v2/messages");
        assert_eq!(reparsed.policy().streaming(), StreamingMode::None);
    }

    #[test]
    fn descriptor_rejects_unknown_fields() {
        let raw = serde_json::json!({
            "route_id": "web_chat.send",
            "method": "post",
            "route_pattern": "/api/chat/v2/messages",
            "policy": serde_json::to_value(valid_policy()).expect("policy serializes"),
            "unexpected": true
        });
        let err = serde_json::from_value::<IngressRouteDescriptor>(raw)
            .expect_err("unknown root fields must reject");
        assert!(err.to_string().contains("unknown field"));
    }
}
