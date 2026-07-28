//! Run-scoped loop context: the resolved model route snapshot, the neutral
//! [`LoopRunContext`] carried across every port, and the run-info port.

use ironclaw_host_api::ThreadId;
use serde::{Deserialize, Serialize};

use crate::run_profile::refs::{CheckpointSchemaId, LoopDriverId};
use crate::run_profile::snapshot::ResolvedRunProfile;
use crate::{
    AcceptedMessageRef, ProductTurnContext, RunProfileVersion, TurnActor, TurnId, TurnRunId,
    TurnScope,
};

use super::validate::validate_model_route_component_value;

/// Placeholder component value marking a [`LoopModelRouteSnapshot`] as a
/// caller-requested advisory hint rather than an operator-resolved route.
const ADVISORY_MODEL_ROUTE_COMPONENT: &str = "requested";

/// A run's model route.
///
/// Persisted as a flat four-component object (`provider_id`, `model_id`,
/// `config_version`, `auth_version`); this enum's custom serde preserves that
/// exact wire shape, and an advisory route still writes the `"requested"`
/// sentinel in the three non-model components, so historical stored routes
/// round-trip byte-for-byte (no migration). In memory it is an enum so the two
/// route kinds are distinct types rather than a sentinel smeared across three
/// fields: `Advisory` carries only the caller-requested model id; `Resolved`
/// carries the full operator route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopModelRouteSnapshot {
    /// A caller-requested advisory hint (see [`LoopModelRouteSnapshot::advisory`]):
    /// only `model_id` is meaningful.
    Advisory { model_id: String },
    /// An operator-resolved route with every component bound.
    Resolved {
        provider_id: String,
        model_id: String,
        config_version: String,
        auth_version: String,
    },
}

impl LoopModelRouteSnapshot {
    pub fn new(
        provider_id: impl Into<String>,
        model_id: impl Into<String>,
        config_version: impl Into<String>,
        auth_version: impl Into<String>,
    ) -> Self {
        Self::from_components(
            provider_id.into(),
            model_id.into(),
            config_version.into(),
            auth_version.into(),
        )
    }

    pub fn try_new(
        provider_id: impl Into<String>,
        model_id: impl Into<String>,
        config_version: impl Into<String>,
        auth_version: impl Into<String>,
    ) -> Result<Self, String> {
        let snapshot = Self::new(provider_id, model_id, config_version, auth_version);
        snapshot.validate()?;
        Ok(snapshot)
    }

    /// Classify a flat four-component route into the typed enum: the advisory
    /// sentinel in all three non-model components means [`Advisory`](Self::Advisory),
    /// otherwise [`Resolved`](Self::Resolved). The single place the sentinel is
    /// interpreted — shared by [`new`](Self::new) and `Deserialize`.
    fn from_components(
        provider_id: String,
        model_id: String,
        config_version: String,
        auth_version: String,
    ) -> Self {
        if provider_id == ADVISORY_MODEL_ROUTE_COMPONENT
            && config_version == ADVISORY_MODEL_ROUTE_COMPONENT
            && auth_version == ADVISORY_MODEL_ROUTE_COMPONENT
        {
            Self::Advisory { model_id }
        } else {
            Self::Resolved {
                provider_id,
                model_id,
                config_version,
                auth_version,
            }
        }
    }

    /// The persisted flat components `(provider_id, model_id, config_version,
    /// auth_version)`. An advisory route reports the `"requested"` sentinel for
    /// the three non-model components, reproducing the historical wire shape.
    fn components(&self) -> (&str, &str, &str, &str) {
        match self {
            Self::Advisory { model_id } => (
                ADVISORY_MODEL_ROUTE_COMPONENT,
                model_id,
                ADVISORY_MODEL_ROUTE_COMPONENT,
                ADVISORY_MODEL_ROUTE_COMPONENT,
            ),
            Self::Resolved {
                provider_id,
                model_id,
                config_version,
                auth_version,
            } => (provider_id, model_id, config_version, auth_version),
        }
    }

    /// Build an *advisory* route from a caller-requested model string — only
    /// `model_id` carries meaning. Advisory routes exist so a caller (e.g. an
    /// OpenAI-compatible client) can request a model without an operator-approved
    /// route binding: the non-routed gateway honors the model id when its
    /// provider supports per-request overrides and otherwise falls back to the
    /// active model, while routed hosts still validate the route and fail closed.
    /// Returns `None` when the model is empty or not a valid route component, so
    /// the run falls back to the deployment's active model.
    pub fn advisory(requested_model: &str) -> Option<Self> {
        let model = requested_model.trim();
        if model.is_empty() {
            return None;
        }
        Self::try_new(
            ADVISORY_MODEL_ROUTE_COMPONENT,
            model,
            ADVISORY_MODEL_ROUTE_COMPONENT,
            ADVISORY_MODEL_ROUTE_COMPONENT,
        )
        .ok()
    }

    /// The requested (advisory) or resolved model id — meaningful for both kinds.
    pub fn model_id(&self) -> &str {
        match self {
            Self::Advisory { model_id } | Self::Resolved { model_id, .. } => model_id,
        }
    }

    /// The provider id component. An advisory route reports the `"requested"`
    /// sentinel (only [`model_id`](Self::model_id) is meaningful for it); a
    /// resolved route reports its bound provider. Prefer matching on the variant
    /// when the distinction matters — use [`is_advisory`](Self::is_advisory).
    pub fn provider_id(&self) -> &str {
        self.components().0
    }

    /// The config-version component (advisory = the `"requested"` sentinel).
    pub fn config_version(&self) -> &str {
        self.components().2
    }

    /// The auth-version component (advisory = the `"requested"` sentinel).
    pub fn auth_version(&self) -> &str {
        self.components().3
    }

    /// Whether this route is a caller-requested advisory hint rather than an
    /// operator-resolved route. A non-routed host passes an advisory snapshot
    /// through unvalidated but fails closed on an operator route it cannot
    /// validate without a resolver.
    pub fn is_advisory(&self) -> bool {
        matches!(self, Self::Advisory { .. })
    }

    pub fn validate(&self) -> Result<(), String> {
        let (provider_id, model_id, config_version, auth_version) = self.components();
        validate_model_route_component_value("provider_id", provider_id, 128, |character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
        })?;
        validate_model_route_component_value("model_id", model_id, 256, |character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | ':' | '/')
        })?;
        validate_model_route_component_value("config_version", config_version, 128, |character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | ':')
        })?;
        validate_model_route_component_value("auth_version", auth_version, 128, |character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | ':')
        })?;
        Ok(())
    }
}

/// Flat wire shape for [`LoopModelRouteSnapshot`], preserved across the
/// enum refactor so historical persisted routes round-trip unchanged.
#[derive(Serialize, Deserialize)]
struct LoopModelRouteSnapshotWire {
    provider_id: String,
    model_id: String,
    config_version: String,
    auth_version: String,
}

impl Serialize for LoopModelRouteSnapshot {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let (provider_id, model_id, config_version, auth_version) = self.components();
        LoopModelRouteSnapshotWire {
            provider_id: provider_id.to_string(),
            model_id: model_id.to_string(),
            config_version: config_version.to_string(),
            auth_version: auth_version.to_string(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for LoopModelRouteSnapshot {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = LoopModelRouteSnapshotWire::deserialize(deserializer)?;
        let snapshot = Self::from_components(
            wire.provider_id,
            wire.model_id,
            wire.config_version,
            wire.auth_version,
        );
        // A rehydrated snapshot MUST route through validation so a persisted or
        // tampered wire route can never deserialize into an unvalidated
        // `Resolved` route. The advisory `"requested"` sentinel passes these
        // component checks, so advisory routes still round-trip.
        snapshot.validate().map_err(serde::de::Error::custom)?;
        Ok(snapshot)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopRunContext {
    pub scope: TurnScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<TurnActor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_message_ref: Option<AcceptedMessageRef>,
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub run_id: TurnRunId,
    pub resolved_run_profile: ResolvedRunProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model_route: Option<LoopModelRouteSnapshot>,
    pub loop_driver_id: LoopDriverId,
    pub loop_driver_version: RunProfileVersion,
    pub checkpoint_schema_id: CheckpointSchemaId,
    pub checkpoint_schema_version: RunProfileVersion,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_context: Option<ProductTurnContext>,
}

impl LoopRunContext {
    pub fn new(
        scope: TurnScope,
        turn_id: TurnId,
        run_id: TurnRunId,
        resolved_run_profile: ResolvedRunProfile,
    ) -> Self {
        let thread_id = scope.thread_id.clone();
        let loop_driver_id = resolved_run_profile.loop_driver.id.clone();
        let loop_driver_version = resolved_run_profile.loop_driver.version;
        let checkpoint_schema_id = resolved_run_profile.checkpoint_schema_id.clone();
        let checkpoint_schema_version = resolved_run_profile.checkpoint_schema_version;
        Self {
            scope,
            actor: None,
            accepted_message_ref: None,
            thread_id,
            turn_id,
            run_id,
            resolved_run_profile,
            resolved_model_route: None,
            loop_driver_id,
            loop_driver_version,
            checkpoint_schema_id,
            checkpoint_schema_version,
            product_context: None,
        }
    }

    pub fn with_actor(mut self, actor: TurnActor) -> Self {
        self.actor = Some(actor);
        self
    }

    pub fn with_accepted_message_ref(mut self, accepted_message_ref: AcceptedMessageRef) -> Self {
        self.accepted_message_ref = Some(accepted_message_ref);
        self
    }

    pub fn actor(&self) -> Option<&TurnActor> {
        self.actor.as_ref()
    }

    pub fn with_resolved_model_route(mut self, snapshot: LoopModelRouteSnapshot) -> Self {
        self.resolved_model_route = Some(snapshot);
        self
    }

    pub fn with_product_context(mut self, product_context: ProductTurnContext) -> Self {
        self.product_context = Some(product_context);
        self
    }
}

pub trait LoopRunInfoPort: Send + Sync {
    fn run_context(&self) -> &LoopRunContext;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advisory_model_route_carries_model_and_marks_itself_advisory() {
        let route = LoopModelRouteSnapshot::advisory("gpt-4o").expect("valid model");
        assert_eq!(route.model_id(), "gpt-4o");
        assert!(route.is_advisory());
        assert!(route.validate().is_ok());
    }

    #[test]
    fn operator_resolved_route_is_not_advisory() {
        let route = LoopModelRouteSnapshot::new("openai", "gpt-4o", "config:v1", "auth:v1");
        assert!(!route.is_advisory());
    }

    #[test]
    fn wire_shape_is_the_flat_four_component_object_and_round_trips() {
        // The enum refactor must not change the persisted shape: historical
        // stored routes (flat objects, advisory = the "requested" sentinel in
        // three components) must deserialize to the right variant AND serialize
        // back to the identical flat object, so pre-existing run records survive.
        let advisory_json = r#"{"provider_id":"requested","model_id":"gpt-4o","config_version":"requested","auth_version":"requested"}"#;
        let advisory: LoopModelRouteSnapshot =
            serde_json::from_str(advisory_json).expect("advisory route deserializes");
        assert_eq!(
            advisory,
            LoopModelRouteSnapshot::Advisory {
                model_id: "gpt-4o".to_string()
            }
        );
        assert!(advisory.is_advisory());
        assert_eq!(
            serde_json::to_string(&advisory).expect("serialize"),
            advisory_json
        );

        let resolved_json = r#"{"provider_id":"anthropic","model_id":"claude","config_version":"cfg:v1","auth_version":"auth:v1"}"#;
        let resolved: LoopModelRouteSnapshot =
            serde_json::from_str(resolved_json).expect("resolved route deserializes");
        assert_eq!(
            resolved,
            LoopModelRouteSnapshot::Resolved {
                provider_id: "anthropic".to_string(),
                model_id: "claude".to_string(),
                config_version: "cfg:v1".to_string(),
                auth_version: "auth:v1".to_string(),
            }
        );
        assert!(!resolved.is_advisory());
        assert_eq!(
            serde_json::to_string(&resolved).expect("serialize"),
            resolved_json
        );
    }

    #[test]
    fn deserialize_validates_route_components() {
        // A well-formed operator route round-trips.
        let valid = LoopModelRouteSnapshot::new("openai", "gpt-4o", "config:v1", "auth:v1");
        let json = serde_json::to_string(&valid).expect("serialize");
        let restored: LoopModelRouteSnapshot = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored, valid);

        // Deserialization must not bypass validation: a secret-like component
        // that `new` would happily construct must be rejected on the wire so a
        // tampered/legacy snapshot cannot rehydrate into an unvalidated route.
        let secret_like = serde_json::json!({
            "provider_id": "sk-secret-provider",
            "model_id": "gpt-4",
            "config_version": "config:v1",
            "auth_version": "auth:v1",
        })
        .to_string();
        serde_json::from_str::<LoopModelRouteSnapshot>(&secret_like)
            .expect_err("secret-like provider_id must be rejected on deserialize");

        let forbidden_marker = serde_json::json!({
            "provider_id": "openrouter",
            "model_id": "gpt-4",
            "config_version": "config:api_key",
            "auth_version": "auth:v1",
        })
        .to_string();
        serde_json::from_str::<LoopModelRouteSnapshot>(&forbidden_marker)
            .expect_err("forbidden marker in config_version must be rejected on deserialize");
    }

    #[test]
    fn advisory_model_route_trims_and_rejects_empty_or_invalid_models() {
        assert_eq!(LoopModelRouteSnapshot::advisory("   "), None);
        assert_eq!(LoopModelRouteSnapshot::advisory(""), None);
        // A model id with a space is not a valid route component → falls back.
        assert_eq!(LoopModelRouteSnapshot::advisory("gpt 4o"), None);
        // Surrounding whitespace is trimmed before validation.
        assert_eq!(
            LoopModelRouteSnapshot::advisory("  claude-opus-4-6  ")
                .map(|route| route.model_id().to_string()),
            Some("claude-opus-4-6".to_string())
        );
    }
}
