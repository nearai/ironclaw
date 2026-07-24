use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
};

use async_trait::async_trait;
pub use ironclaw_host_api::OutboundDeliveryTargetId;
use ironclaw_host_api::{TenantId, UserId};
use ironclaw_turns::ReplyTargetBindingRef;
use serde::{Deserialize, Serialize};

use crate::{DeliveryTargetCapabilities, OutboundError, RunFinalReplyDestination};

const OUTBOUND_DELIVERY_CHANNEL_MAX_BYTES: usize = 64;
const OUTBOUND_DELIVERY_DISPLAY_NAME_MAX_BYTES: usize = 128;
const OUTBOUND_DELIVERY_DESCRIPTION_MAX_BYTES: usize = 512;

/// Authenticated caller scope for outbound delivery target discovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundDeliveryTargetScope {
    pub tenant_id: TenantId,
    pub user_id: UserId,
}

impl OutboundDeliveryTargetScope {
    pub fn new(tenant_id: TenantId, user_id: UserId) -> Self {
        Self { tenant_id, user_id }
    }
}

/// The `(tenant, user)` an outbound delivery-target entry belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundDeliveryTargetOwner {
    pub tenant_id: TenantId,
    pub user_id: UserId,
}

impl OutboundDeliveryTargetOwner {
    pub fn new(tenant_id: TenantId, user_id: UserId) -> Self {
        Self { tenant_id, user_id }
    }

    pub fn for_scope(scope: &OutboundDeliveryTargetScope) -> Self {
        Self {
            tenant_id: scope.tenant_id.clone(),
            user_id: scope.user_id.clone(),
        }
    }

    pub fn matches_scope(&self, scope: &OutboundDeliveryTargetScope) -> bool {
        self.tenant_id == scope.tenant_id && self.user_id == scope.user_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "UncheckedOutboundDeliveryTargetSummary")]
pub struct OutboundDeliveryTargetSummary {
    pub target_id: OutboundDeliveryTargetId,
    pub channel: OutboundDeliveryTargetChannel,
    pub display_name: OutboundDeliveryTargetDisplayName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<OutboundDeliveryTargetDescription>,
}

impl OutboundDeliveryTargetSummary {
    pub fn new(
        target_id: OutboundDeliveryTargetId,
        channel: impl Into<String>,
        display_name: impl Into<String>,
        description: Option<String>,
    ) -> Result<Self, String> {
        Ok(Self {
            target_id,
            channel: OutboundDeliveryTargetChannel::new(channel)?,
            display_name: OutboundDeliveryTargetDisplayName::new(display_name)?,
            description: description
                .map(OutboundDeliveryTargetDescription::new)
                .transpose()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct UncheckedOutboundDeliveryTargetSummary {
    target_id: OutboundDeliveryTargetId,
    channel: String,
    display_name: String,
    #[serde(default)]
    description: Option<String>,
}

impl TryFrom<UncheckedOutboundDeliveryTargetSummary> for OutboundDeliveryTargetSummary {
    type Error = String;

    fn try_from(value: UncheckedOutboundDeliveryTargetSummary) -> Result<Self, Self::Error> {
        Self::new(
            value.target_id,
            value.channel,
            value.display_name,
            value.description,
        )
    }
}

macro_rules! bounded_outbound_text {
    ($name:ident, $field:literal, $max:ident, $required:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String")]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, String> {
                let value = value.into();
                validate_outbound_delivery_display_field($field, &value, $max, $required)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = String;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

bounded_outbound_text!(
    OutboundDeliveryTargetChannel,
    "outbound delivery channel",
    OUTBOUND_DELIVERY_CHANNEL_MAX_BYTES,
    true
);
bounded_outbound_text!(
    OutboundDeliveryTargetDisplayName,
    "outbound delivery display name",
    OUTBOUND_DELIVERY_DISPLAY_NAME_MAX_BYTES,
    true
);
bounded_outbound_text!(
    OutboundDeliveryTargetDescription,
    "outbound delivery description",
    OUTBOUND_DELIVERY_DESCRIPTION_MAX_BYTES,
    false
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundDeliveryTargetEntry {
    pub summary: OutboundDeliveryTargetSummary,
    pub capabilities: DeliveryTargetCapabilities,
    pub destination: RunFinalReplyDestination,
    pub owner: OutboundDeliveryTargetOwner,
}

/// Generic provider for a host-owned target such as WebApp. The destination
/// is product-neutral outbound domain data; composition only registers the
/// provider and never re-declares its policy.
pub struct HostOwnedOutboundDeliveryTargetProvider {
    summary: OutboundDeliveryTargetSummary,
    capabilities: DeliveryTargetCapabilities,
    destination: RunFinalReplyDestination,
}

impl HostOwnedOutboundDeliveryTargetProvider {
    pub fn new(
        summary: OutboundDeliveryTargetSummary,
        capabilities: DeliveryTargetCapabilities,
        destination: RunFinalReplyDestination,
    ) -> Self {
        Self {
            summary,
            capabilities,
            destination,
        }
    }
}

#[async_trait]
impl OutboundDeliveryTargetProvider for HostOwnedOutboundDeliveryTargetProvider {
    async fn list_outbound_delivery_targets(
        &self,
        scope: &OutboundDeliveryTargetScope,
    ) -> Result<Vec<OutboundDeliveryTargetEntry>, OutboundError> {
        Ok(vec![OutboundDeliveryTargetEntry {
            summary: self.summary.clone(),
            capabilities: self.capabilities.clone(),
            destination: self.destination.clone(),
            owner: OutboundDeliveryTargetOwner::for_scope(scope),
        }])
    }
}

#[async_trait]
pub trait OutboundDeliveryTargetProvider: Send + Sync {
    async fn list_outbound_delivery_targets(
        &self,
        scope: &OutboundDeliveryTargetScope,
    ) -> Result<Vec<OutboundDeliveryTargetEntry>, OutboundError>;

    async fn resolve_outbound_delivery_target(
        &self,
        scope: &OutboundDeliveryTargetScope,
        target_id: &OutboundDeliveryTargetId,
    ) -> Result<Option<OutboundDeliveryTargetEntry>, OutboundError> {
        Ok(self
            .list_outbound_delivery_targets(scope)
            .await?
            .into_iter()
            .find(|entry| {
                entry.capabilities.final_replies
                    && entry.summary.target_id.as_str() == target_id.as_str()
            }))
    }

    async fn resolve_reply_target_binding(
        &self,
        scope: &OutboundDeliveryTargetScope,
        target: &ReplyTargetBindingRef,
    ) -> Result<Option<OutboundDeliveryTargetEntry>, OutboundError> {
        Ok(self
            .list_outbound_delivery_targets(scope)
            .await?
            .into_iter()
            .find(|entry| {
                entry.capabilities.final_replies
                    && matches!(
                        &entry.destination,
                        RunFinalReplyDestination::External { reply_target_binding_ref }
                            if reply_target_binding_ref.as_str() == target.as_str()
                    )
            }))
    }
}

pub struct OutboundDeliveryTargetRegistry {
    providers: Vec<Arc<dyn OutboundDeliveryTargetProvider>>,
}

impl std::fmt::Debug for OutboundDeliveryTargetRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OutboundDeliveryTargetRegistry")
            .field("providers", &self.providers.len())
            .finish()
    }
}

impl OutboundDeliveryTargetRegistry {
    pub fn new(providers: Vec<Arc<dyn OutboundDeliveryTargetProvider>>) -> Self {
        Self { providers }
    }
}

pub struct MutableOutboundDeliveryTargetRegistry {
    providers: RwLock<BTreeMap<String, Arc<dyn OutboundDeliveryTargetProvider>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboundDeliveryTargetRegistrationOutcome {
    Registered,
    Replaced,
}

impl std::fmt::Debug for MutableOutboundDeliveryTargetRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let provider_count = match self.providers.read() {
            Ok(providers) => providers.len(),
            Err(error) => {
                tracing::debug!(
                    target = "ironclaw::outbound::delivery_targets",
                    error = ?error,
                    "outbound target registry read lock failed during debug formatting"
                );
                0
            }
        };
        formatter
            .debug_struct("MutableOutboundDeliveryTargetRegistry")
            .field("providers", &provider_count)
            .finish()
    }
}

impl Default for MutableOutboundDeliveryTargetRegistry {
    fn default() -> Self {
        Self {
            providers: RwLock::new(BTreeMap::new()),
        }
    }
}

impl MutableOutboundDeliveryTargetRegistry {
    pub fn register_provider(
        &self,
        provider_key: impl Into<String>,
        provider: Arc<dyn OutboundDeliveryTargetProvider>,
    ) -> Result<OutboundDeliveryTargetRegistrationOutcome, OutboundError> {
        let mut providers = self.providers.write().map_err(|error| {
            tracing::debug!(
                target = "ironclaw::outbound::delivery_targets",
                error = ?error,
                "outbound target registry write lock failed"
            );
            OutboundError::Backend
        })?;
        let outcome = match providers.insert(provider_key.into(), provider) {
            Some(_) => OutboundDeliveryTargetRegistrationOutcome::Replaced,
            None => OutboundDeliveryTargetRegistrationOutcome::Registered,
        };
        Ok(outcome)
    }

    fn providers(&self) -> Result<Vec<Arc<dyn OutboundDeliveryTargetProvider>>, OutboundError> {
        self.providers
            .read()
            .map(|providers| providers.values().cloned().collect())
            .map_err(|error| {
                tracing::debug!(
                    target = "ironclaw::outbound::delivery_targets",
                    error = ?error,
                    "outbound target registry read lock failed"
                );
                OutboundError::Backend
            })
    }
}

#[async_trait]
impl OutboundDeliveryTargetProvider for MutableOutboundDeliveryTargetRegistry {
    async fn list_outbound_delivery_targets(
        &self,
        scope: &OutboundDeliveryTargetScope,
    ) -> Result<Vec<OutboundDeliveryTargetEntry>, OutboundError> {
        OutboundDeliveryTargetRegistry::new(self.providers()?)
            .list_outbound_delivery_targets(scope)
            .await
    }

    async fn resolve_outbound_delivery_target(
        &self,
        scope: &OutboundDeliveryTargetScope,
        target_id: &OutboundDeliveryTargetId,
    ) -> Result<Option<OutboundDeliveryTargetEntry>, OutboundError> {
        OutboundDeliveryTargetRegistry::new(self.providers()?)
            .resolve_outbound_delivery_target(scope, target_id)
            .await
    }

    async fn resolve_reply_target_binding(
        &self,
        scope: &OutboundDeliveryTargetScope,
        target: &ReplyTargetBindingRef,
    ) -> Result<Option<OutboundDeliveryTargetEntry>, OutboundError> {
        OutboundDeliveryTargetRegistry::new(self.providers()?)
            .resolve_reply_target_binding(scope, target)
            .await
    }
}

#[async_trait]
impl OutboundDeliveryTargetProvider for OutboundDeliveryTargetRegistry {
    async fn list_outbound_delivery_targets(
        &self,
        scope: &OutboundDeliveryTargetScope,
    ) -> Result<Vec<OutboundDeliveryTargetEntry>, OutboundError> {
        let mut entries = Vec::new();
        for provider in &self.providers {
            entries.extend(provider.list_outbound_delivery_targets(scope).await?);
        }
        Ok(entries
            .into_iter()
            .filter(|entry| entry.owner.matches_scope(scope))
            .collect())
    }

    async fn resolve_outbound_delivery_target(
        &self,
        scope: &OutboundDeliveryTargetScope,
        target_id: &OutboundDeliveryTargetId,
    ) -> Result<Option<OutboundDeliveryTargetEntry>, OutboundError> {
        for provider in &self.providers {
            if let Some(entry) = provider
                .resolve_outbound_delivery_target(scope, target_id)
                .await?
                .filter(|entry| {
                    entry.owner.matches_scope(scope) && entry.capabilities.final_replies
                })
            {
                return Ok(Some(entry));
            }
        }
        Ok(None)
    }

    async fn resolve_reply_target_binding(
        &self,
        scope: &OutboundDeliveryTargetScope,
        target: &ReplyTargetBindingRef,
    ) -> Result<Option<OutboundDeliveryTargetEntry>, OutboundError> {
        for provider in &self.providers {
            if let Some(entry) = provider
                .resolve_reply_target_binding(scope, target)
                .await?
                .filter(|entry| {
                    entry.owner.matches_scope(scope) && entry.capabilities.final_replies
                })
            {
                return Ok(Some(entry));
            }
        }
        Ok(None)
    }
}

fn validate_outbound_delivery_display_field(
    field_name: &str,
    value: &str,
    max_bytes: usize,
    require_non_empty: bool,
) -> Result<(), String> {
    if require_non_empty && value.trim().is_empty() {
        return Err(format!("{field_name} must not be empty"));
    }
    if value.len() > max_bytes {
        return Err(format!("{field_name} must be at most {max_bytes} bytes"));
    }
    if value.trim() != value {
        return Err(format!(
            "{field_name} must not contain leading or trailing whitespace"
        ));
    }
    if value.chars().any(|c| c.is_control()) {
        return Err(format!("{field_name} must not contain control characters"));
    }
    if has_unsafe_unicode_format_character(value) {
        return Err(format!(
            "{field_name} must not contain unsafe Unicode formatting characters"
        ));
    }
    if has_line_or_paragraph_separator(value) {
        return Err(format!(
            "{field_name} must not contain line or paragraph separators"
        ));
    }
    Ok(())
}

fn has_unsafe_unicode_format_character(value: &str) -> bool {
    value.chars().any(|c| {
        matches!(
            c,
            '\u{061c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
                | '\u{00ad}'
                | '\u{034f}'
                | '\u{180e}'
                | '\u{200b}'..='\u{200d}'
                | '\u{2060}'
                | '\u{feff}'
        )
    })
}

fn has_line_or_paragraph_separator(value: &str) -> bool {
    value.chars().any(|c| matches!(c, '\u{2028}' | '\u{2029}'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct StaticProvider {
        entries: Vec<OutboundDeliveryTargetEntry>,
    }

    #[async_trait]
    impl OutboundDeliveryTargetProvider for StaticProvider {
        async fn list_outbound_delivery_targets(
            &self,
            _scope: &OutboundDeliveryTargetScope,
        ) -> Result<Vec<OutboundDeliveryTargetEntry>, OutboundError> {
            Ok(self.entries.clone())
        }
    }

    #[tokio::test]
    async fn registry_drops_entries_not_owned_by_querying_scope() {
        let scope = target_scope("tenant-alpha", "user-alpha");
        let registry = OutboundDeliveryTargetRegistry::new(vec![Arc::new(StaticProvider {
            entries: vec![
                target_entry("slack-foreign", "tenant-alpha", "user-bravo", true),
                target_entry("slack-owned", "tenant-alpha", "user-alpha", true),
            ],
        })]);

        let listed = registry
            .list_outbound_delivery_targets(&scope)
            .await
            .expect("list targets");
        assert_eq!(
            listed
                .iter()
                .map(|entry| entry.summary.target_id.as_str())
                .collect::<Vec<_>>(),
            vec!["slack-owned"]
        );
        assert!(
            registry
                .resolve_outbound_delivery_target(&scope, &target_id("slack-foreign"))
                .await
                .expect("resolve target")
                .is_none()
        );
    }

    #[tokio::test]
    async fn registry_resolves_only_final_reply_capable_targets() {
        let scope = target_scope("tenant-alpha", "user-alpha");
        let registry = OutboundDeliveryTargetRegistry::new(vec![Arc::new(StaticProvider {
            entries: vec![target_entry(
                "slack-progress",
                "tenant-alpha",
                "user-alpha",
                false,
            )],
        })]);

        assert!(
            registry
                .resolve_outbound_delivery_target(&scope, &target_id("slack-progress"))
                .await
                .expect("resolve target")
                .is_none()
        );
        assert!(
            registry
                .resolve_reply_target_binding(&scope, &reply_ref("reply:slack-progress"))
                .await
                .expect("resolve binding")
                .is_none()
        );
    }

    #[test]
    fn target_id_rejects_ambiguous_display_values() {
        for value in ["", " target", "target ", "bad\nid", "\u{202e}target"] {
            assert!(
                OutboundDeliveryTargetId::new(value).is_err(),
                "target id should reject {value:?}"
            );
        }
    }

    fn target_entry(
        target_id_value: &str,
        owner_tenant: &str,
        owner_user: &str,
        final_replies: bool,
    ) -> OutboundDeliveryTargetEntry {
        OutboundDeliveryTargetEntry {
            summary: OutboundDeliveryTargetSummary::new(
                target_id(target_id_value),
                "slack",
                "Slack DM",
                Some("Slack direct message".to_string()),
            )
            .expect("target summary"),
            capabilities: DeliveryTargetCapabilities {
                final_replies,
                progress: !final_replies,
                gate_prompts: true,
                auth_prompts: true,
                modalities: Vec::new(),
            },
            destination: RunFinalReplyDestination::External {
                reply_target_binding_ref: reply_ref(format!("reply:{target_id_value}")),
            },
            owner: OutboundDeliveryTargetOwner::new(tenant(owner_tenant), user(owner_user)),
        }
    }

    fn target_scope(tenant_id: &str, user_id: &str) -> OutboundDeliveryTargetScope {
        OutboundDeliveryTargetScope::new(tenant(tenant_id), user(user_id))
    }

    fn target_id(value: &str) -> OutboundDeliveryTargetId {
        OutboundDeliveryTargetId::new(value).expect("target id")
    }

    fn reply_ref(value: impl Into<String>) -> ReplyTargetBindingRef {
        ReplyTargetBindingRef::new(value).expect("reply target binding ref")
    }

    fn tenant(value: &str) -> TenantId {
        TenantId::new(value).expect("tenant id")
    }

    fn user(value: &str) -> UserId {
        UserId::new(value).expect("user id")
    }
}
