//! Capability — the unit of effect.
//!
//! A capability bundles actions (tools), knowledge (skills), and policies
//! (hooks) into a single installable/activatable unit. Capabilities are
//! granted to threads via leases.
//!
//! Model-facing surfacing is intentionally split:
//! - `ActionInventory` contains callable actions for the current step
//! - `CapabilitySummary` contains background/contextual capability metadata,
//!   including blocked integrations that belong in `Activatable Integrations`
//!   rather than on the normal callable surface

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

use crate::types::thread::ThreadId;

// ── Granted actions ────────────────────────────────────────

/// Which actions a lease grants access to.
///
/// `All` means the lease covers every action in the capability (wildcard).
/// `Specific` restricts the lease to the listed action names.
///
/// Serializes as a JSON array for backward compatibility: `[]` = All,
/// `["a","b"]` = Specific.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantedActions {
    /// Wildcard — covers all actions in the capability.
    All,
    /// Restricted to specific action names.
    Specific(Vec<String>),
}

impl GrantedActions {
    /// Check whether a specific action is covered.
    pub fn covers(&self, action_name: &str) -> bool {
        let hyphenated = action_name.replace('_', "-");
        let underscored = action_name.replace('-', "_");
        match self {
            GrantedActions::All => true,
            GrantedActions::Specific(actions) => actions.iter().any(|action| {
                action == action_name || action == &hyphenated || action == &underscored
            }),
        }
    }

    /// Returns true if this is a wildcard grant.
    pub fn is_all(&self) -> bool {
        matches!(self, GrantedActions::All)
    }

    /// Returns the specific actions, or an empty slice for wildcard.
    pub fn actions(&self) -> &[String] {
        match self {
            GrantedActions::All => &[],
            GrantedActions::Specific(actions) => actions,
        }
    }
}

impl Serialize for GrantedActions {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            GrantedActions::All => Vec::<String>::new().serialize(serializer),
            GrantedActions::Specific(v) => v.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for GrantedActions {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = Vec::<String>::deserialize(deserializer)?;
        if v.is_empty() {
            Ok(GrantedActions::All)
        } else {
            Ok(GrantedActions::Specific(v))
        }
    }
}

/// Strongly-typed lease identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LeaseId(pub Uuid);

impl LeaseId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for LeaseId {
    fn default() -> Self {
        Self::new()
    }
}

// ── Effect types ────────────────────────────────────────────

/// Classification of side effects that an action may produce.
/// Used by the policy engine for allow/deny decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EffectType {
    /// Read from local filesystem or workspace.
    ReadLocal,
    /// Read from external APIs (no mutation).
    ReadExternal,
    /// Write to local filesystem or workspace.
    WriteLocal,
    /// Write to external services (create PR, send email).
    WriteExternal,
    /// Authenticated API call requiring credentials.
    CredentialedNetwork,
    /// Code execution or shell access.
    Compute,
    /// Financial operations (payments, transfers).
    Financial,
}

// ── Action definition ───────────────────────────────────────

/// Definition of a single action within a capability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionDef {
    /// Action name (e.g. "create_issue", "web_fetch").
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// JSON Schema for parameters.
    pub parameters_schema: serde_json::Value,
    /// Effect types this action may produce.
    pub effects: Vec<EffectType>,
    /// Whether this action requires user approval before execution.
    pub requires_approval: bool,
    /// How this action should be surfaced to the model.
    #[serde(default)]
    pub model_tool_surface: ModelToolSurface,
    /// Optional discovery metadata used by `tool_info` and prompt guidance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery: Option<ActionDiscoveryMetadata>,
}

/// Whether an action should be emitted as a provider-native tool definition or
/// only shown through compact prompt metadata with on-demand `tool_info`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelToolSurface {
    /// Emit the full callable schema to the provider-native tool array.
    #[default]
    FullSchema,
    /// Keep the action callable in-step, but surface it compactly in prompt
    /// metadata and rely on `tool_info(..., detail="schema")` for parameters.
    CompactToolInfo,
}

/// Model-visible action inventory for a single execution step.
///
/// `inline` actions are callable now. `discoverable` actions are not callable
/// yet, but remain available to `tool_info` for step-scoped discovery (for
/// example blocked actions under `Activatable Integrations`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionInventory {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inline: Vec<ActionDef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub discoverable: Vec<ActionDef>,
}

/// Curated discovery guidance for a callable action.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionDiscoverySummary {
    /// Parameters that are always required.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub always_required: Vec<String>,
    /// Conditional requirements or cross-field invariants.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditional_requirements: Vec<String>,
    /// Additional notes for correct tool selection/use.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    /// Optional structured examples.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<serde_json::Value>,
}

/// Optional discovery metadata layered on top of an executable action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionDiscoveryMetadata {
    /// Canonical discovery name shown to the model.
    pub name: String,
    /// Optional curated discovery guidance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<ActionDiscoverySummary>,
    /// Optional discovery schema when it differs from the callable schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_override: Option<serde_json::Value>,
}

impl ActionDef {
    /// Whether this action should be emitted as a provider-native tool
    /// definition with its full schema.
    pub fn emits_full_schema_tool(&self) -> bool {
        matches!(self.model_tool_surface, ModelToolSurface::FullSchema)
    }

    /// Canonical discovery name for this action.
    pub fn discovery_name(&self) -> &str {
        self.discovery
            .as_ref()
            .map(|metadata| metadata.name.as_str())
            .unwrap_or(self.name.as_str())
    }

    /// Discovery schema, defaulting to the callable schema.
    pub fn discovery_schema(&self) -> &serde_json::Value {
        self.discovery
            .as_ref()
            .and_then(|metadata| metadata.schema_override.as_ref())
            .unwrap_or(&self.parameters_schema)
    }

    /// Curated discovery summary, when one exists.
    pub fn discovery_summary(&self) -> Option<&ActionDiscoverySummary> {
        self.discovery
            .as_ref()
            .and_then(|metadata| metadata.summary.as_ref())
    }

    /// Checks whether the given name resolves to this action.
    pub fn matches_name(&self, name: &str) -> bool {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return false;
        }
        let discovery_name = self.discovery_name();
        if self.name == trimmed || discovery_name == trimmed {
            return true;
        }
        if trimmed.contains('-') || self.name.contains('-') || discovery_name.contains('-') {
            let normalized = trimmed.replace('-', "_");
            return normalized == self.name.replace('-', "_")
                || normalized == discovery_name.replace('-', "_");
        }
        false
    }
}

/// Canonical model-visible status for capability background surfacing.
///
/// This is a normalized projection over host runtime truth. It is not itself
/// a source of truth for auth, activation, or installation state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityStatus {
    /// Capability is directly usable now.
    Ready,
    /// Capability is usable now, but only through a scoped or indirect route.
    ReadyScoped,
    /// Capability exists but needs authentication before use.
    NeedsAuth,
    /// Capability exists but needs setup before auth or execution can proceed.
    NeedsSetup,
    /// Capability is installed or known, but not currently active.
    Inactive,
    /// Capability is known to the runtime but not yet activated into a direct action.
    Latent,
    /// Capability lookup or readiness failed with a concrete runtime error.
    Error,
    /// Capability is known in the registry but not installed.
    AvailableNotInstalled,
}

/// High-level category for capability background summaries.
///
/// This is used for contextual capability rendering and activatable
/// integration rendering, not for normal callable action inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySummaryKind {
    /// Messaging or notification route usable through a bridge action.
    Channel,
    /// Extension-backed provider or integration.
    Provider,
    /// Engine-native runtime capability background.
    Runtime,
}

/// Background summary for a contextual or activatable capability.
///
/// Ready callable actions stay in `ActionInventory`. `CapabilitySummary`
/// covers:
/// - runtime/contextual information that should stay in background prompt/UI
/// - integrations that need user setup before becoming callable
///   (`NeedsSetup`, `Inactive`, `Latent`, `AvailableNotInstalled`); these
///   surface to the model under `Activatable Integrations` so it can tell
///   the user what's available but cannot be enabled by the model itself
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitySummary {
    /// Stable capability identifier (for example `telegram` or `slack`).
    pub name: String,
    /// Human-readable display name when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// High-level category used by prompt/UI renderers.
    pub kind: CapabilitySummaryKind,
    /// Canonical normalized status.
    pub status: CapabilityStatus,
    /// Optional human-readable description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional preview of actions unlocked by enabling this capability.
    ///
    /// This is primarily for activatable integrations so the model can see
    /// what becomes callable after enablement without dumping every action
    /// into the default callable surface.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub action_preview: Vec<String>,
    /// Optional routing guidance such as `Usable through message`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing_hint: Option<String>,
}

// ── Capability ──────────────────────────────────────────────

/// A capability — bundles actions, knowledge, and policies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capability {
    /// Capability name (e.g. "github", "deployment").
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Executable actions (replaces tools).
    pub actions: Vec<ActionDef>,
    /// Domain knowledge blocks (replaces skills).
    pub knowledge: Vec<String>,
    /// Policy rules (replaces hooks).
    pub policies: Vec<PolicyRule>,
}

// ── Policy ──────────────────────────────────────────────────

/// A named policy rule within a capability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    pub name: String,
    pub condition: PolicyCondition,
    pub effect: PolicyEffect,
}

/// When a policy rule applies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PolicyCondition {
    /// Always applies.
    Always,
    /// Applies when the action name exactly matches the pattern.
    ActionMatches { pattern: String },
    /// Applies when the action has a specific effect type.
    EffectTypeIs(EffectType),
}

/// What the policy engine decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyEffect {
    Allow,
    Deny,
    RequireApproval,
}

// ── Capability lease ────────────────────────────────────────

/// A time/use-limited grant of capability access to a thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityLease {
    pub id: LeaseId,
    /// The thread this lease is granted to.
    pub thread_id: ThreadId,
    /// Which capability this lease covers.
    pub capability_name: String,
    /// Which actions from the capability are granted.
    pub granted_actions: GrantedActions,
    /// When the lease was granted.
    pub granted_at: DateTime<Utc>,
    /// When the lease expires (None = no expiry).
    pub expires_at: Option<DateTime<Utc>>,
    /// Maximum number of action invocations (None = unlimited).
    pub max_uses: Option<u32>,
    /// Remaining invocations (None = unlimited).
    pub uses_remaining: Option<u32>,
    /// Whether the lease has been explicitly revoked.
    pub revoked: bool,
    /// Why the lease was revoked (for audit trail).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_reason: Option<String>,
}

impl CapabilityLease {
    /// Check whether this lease is currently valid.
    pub fn is_valid(&self) -> bool {
        if self.revoked {
            return false;
        }
        if let Some(expires_at) = self.expires_at
            && Utc::now() >= expires_at
        {
            return false;
        }
        if let Some(remaining) = self.uses_remaining
            && remaining == 0
        {
            return false;
        }
        true
    }

    /// Check whether a specific action is covered by this lease.
    pub fn covers_action(&self, action_name: &str) -> bool {
        self.granted_actions.covers(action_name)
    }

    /// Consume one use of this lease. Returns false if no uses remain.
    pub fn consume_use(&mut self) -> bool {
        if let Some(ref mut remaining) = self.uses_remaining {
            if *remaining == 0 {
                return false;
            }
            *remaining -= 1;
        }
        true
    }

    /// Refund one previously consumed use when execution was interrupted
    /// before the action actually completed.
    pub fn refund_use(&mut self) {
        if let (Some(max_uses), Some(remaining)) = (self.max_uses, self.uses_remaining.as_mut())
            && *remaining < max_uses
        {
            *remaining += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_lease() -> CapabilityLease {
        CapabilityLease {
            id: LeaseId::new(),
            thread_id: ThreadId::new(),
            capability_name: "test".into(),
            granted_actions: GrantedActions::All,
            granted_at: Utc::now(),
            expires_at: None,
            max_uses: None,
            uses_remaining: None,
            revoked: false,
            revoked_reason: None,
        }
    }

    #[test]
    fn valid_lease() {
        let lease = make_lease();
        assert!(lease.is_valid());
    }

    #[test]
    fn revoked_lease_is_invalid() {
        let mut lease = make_lease();
        lease.revoked = true;
        assert!(!lease.is_valid());
    }

    #[test]
    fn expired_lease_is_invalid() {
        let mut lease = make_lease();
        lease.expires_at = Some(Utc::now() - chrono::Duration::seconds(10));
        assert!(!lease.is_valid());
    }

    #[test]
    fn exhausted_lease_is_invalid() {
        let mut lease = make_lease();
        lease.max_uses = Some(1);
        lease.uses_remaining = Some(0);
        assert!(!lease.is_valid());
    }

    #[test]
    fn consume_use_decrements() {
        let mut lease = make_lease();
        lease.max_uses = Some(2);
        lease.uses_remaining = Some(2);
        assert!(lease.consume_use());
        assert_eq!(lease.uses_remaining, Some(1));
        assert!(lease.consume_use());
        assert_eq!(lease.uses_remaining, Some(0));
        assert!(!lease.consume_use());
    }

    #[test]
    fn unlimited_consume_always_succeeds() {
        let mut lease = make_lease();
        for _ in 0..100 {
            assert!(lease.consume_use());
        }
    }

    #[test]
    fn refund_use_restores_budget_up_to_max() {
        let mut lease = make_lease();
        lease.max_uses = Some(2);
        lease.uses_remaining = Some(2);
        assert!(lease.consume_use());
        assert_eq!(lease.uses_remaining, Some(1));
        lease.refund_use();
        assert_eq!(lease.uses_remaining, Some(2));
        lease.refund_use();
        assert_eq!(lease.uses_remaining, Some(2));
    }

    #[test]
    fn covers_action_empty_grants_all() {
        let lease = make_lease();
        assert!(lease.covers_action("anything"));
    }

    #[test]
    fn covers_action_with_specific_grants() {
        let mut lease = make_lease();
        lease.granted_actions =
            GrantedActions::Specific(vec!["create_issue".into(), "list_prs".into()]);
        assert!(lease.covers_action("create_issue"));
        assert!(lease.covers_action("list_prs"));
        assert!(!lease.covers_action("delete_repo"));
    }

    #[test]
    fn covers_action_matches_hyphen_underscore_aliases() {
        let mut lease = make_lease();
        lease.granted_actions = GrantedActions::Specific(vec!["create_issue".into()]);
        assert!(lease.covers_action("create-issue"));

        lease.granted_actions = GrantedActions::Specific(vec!["list-prs".into()]);
        assert!(lease.covers_action("list_prs"));
    }

    #[test]
    fn action_def_matches_exact_and_hyphenated_names() {
        let action = ActionDef {
            name: "mission_create".to_string(),
            description: "Create mission".to_string(),
            parameters_schema: json!({"type": "object"}),
            effects: vec![],
            requires_approval: false,
            model_tool_surface: ModelToolSurface::FullSchema,
            discovery: None,
        };

        assert!(action.matches_name("mission_create"));
        assert!(action.matches_name("mission-create"));
        assert!(!action.matches_name("mission_resume"));
        assert!(!action.matches_name(" "));
    }

    #[test]
    fn action_def_matches_discovery_aliases() {
        let action = ActionDef {
            name: "mission_create".to_string(),
            description: "Create mission".to_string(),
            parameters_schema: json!({"type": "object"}),
            effects: vec![],
            requires_approval: false,
            model_tool_surface: ModelToolSurface::FullSchema,
            discovery: Some(ActionDiscoveryMetadata {
                name: "mission-create".to_string(),
                summary: None,
                schema_override: None,
            }),
        };

        assert!(action.matches_name("mission-create"));
        assert!(action.matches_name("mission_create"));
    }

    #[test]
    fn action_def_matches_hyphenated_canonical_names_from_underscore_input() {
        let action = ActionDef {
            name: "mission-create".to_string(),
            description: "Create mission".to_string(),
            parameters_schema: json!({"type": "object"}),
            effects: vec![],
            requires_approval: false,
            model_tool_surface: ModelToolSurface::FullSchema,
            discovery: None,
        };

        assert!(action.matches_name("mission_create"));
        assert!(action.matches_name("mission-create"));
    }

    #[test]
    fn capability_status_serializes_as_snake_case() {
        let cases = [
            (CapabilityStatus::Ready, json!("ready")),
            (CapabilityStatus::ReadyScoped, json!("ready_scoped")),
            (CapabilityStatus::NeedsAuth, json!("needs_auth")),
            (CapabilityStatus::NeedsSetup, json!("needs_setup")),
            (CapabilityStatus::Inactive, json!("inactive")),
            (CapabilityStatus::Latent, json!("latent")),
            (CapabilityStatus::Error, json!("error")),
            (
                CapabilityStatus::AvailableNotInstalled,
                json!("available_not_installed"),
            ),
        ];

        for (status, expected) in cases {
            assert_eq!(serde_json::to_value(status).unwrap(), expected);
        }
    }

    #[test]
    fn capability_status_round_trips_from_wire_values() {
        let cases = [
            ("ready", CapabilityStatus::Ready),
            ("ready_scoped", CapabilityStatus::ReadyScoped),
            ("needs_auth", CapabilityStatus::NeedsAuth),
            ("needs_setup", CapabilityStatus::NeedsSetup),
            ("inactive", CapabilityStatus::Inactive),
            ("latent", CapabilityStatus::Latent),
            ("error", CapabilityStatus::Error),
            (
                "available_not_installed",
                CapabilityStatus::AvailableNotInstalled,
            ),
        ];

        for (wire, expected) in cases {
            let parsed: CapabilityStatus = serde_json::from_value(json!(wire)).unwrap();
            assert_eq!(parsed, expected);
        }
    }

    #[test]
    fn capability_summary_kind_serializes_as_snake_case() {
        let cases = [
            (CapabilitySummaryKind::Channel, json!("channel")),
            (CapabilitySummaryKind::Provider, json!("provider")),
            (CapabilitySummaryKind::Runtime, json!("runtime")),
        ];

        for (kind, expected) in cases {
            assert_eq!(serde_json::to_value(kind).unwrap(), expected);
        }
    }

    #[test]
    fn capability_summary_round_trips_with_optional_fields() {
        let summary = CapabilitySummary {
            name: "telegram".to_string(),
            display_name: Some("Telegram".to_string()),
            kind: CapabilitySummaryKind::Channel,
            status: CapabilityStatus::ReadyScoped,
            description: Some("Telegram messaging".to_string()),
            action_preview: vec!["telegram_send".to_string()],
            routing_hint: Some("Usable through message".to_string()),
        };

        let json = serde_json::to_value(&summary).unwrap();
        assert_eq!(json["name"], "telegram");
        assert_eq!(json["display_name"], "Telegram");
        assert_eq!(json["kind"], "channel");
        assert_eq!(json["status"], "ready_scoped");
        assert_eq!(json["description"], "Telegram messaging");
        assert_eq!(json["action_preview"], serde_json::json!(["telegram_send"]));
        assert_eq!(json["routing_hint"], "Usable through message");

        let parsed: CapabilitySummary = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, summary);
    }

    #[test]
    fn capability_summary_allows_minimal_payload_and_omits_none_fields() {
        let summary = CapabilitySummary {
            name: "notion".to_string(),
            display_name: None,
            kind: CapabilitySummaryKind::Provider,
            status: CapabilityStatus::NeedsAuth,
            description: None,
            action_preview: Vec::new(),
            routing_hint: None,
        };

        let json = serde_json::to_value(&summary).unwrap();
        assert_eq!(json["name"], "notion");
        assert_eq!(json["kind"], "provider");
        assert_eq!(json["status"], "needs_auth");
        assert!(json.get("display_name").is_none());
        assert!(json.get("description").is_none());
        assert!(json.get("action_preview").is_none());
        assert!(json.get("routing_hint").is_none());

        let parsed: CapabilitySummary = serde_json::from_value(serde_json::json!({
            "name": "notion",
            "kind": "provider",
            "status": "needs_auth"
        }))
        .unwrap();
        assert_eq!(parsed, summary);
    }
}
