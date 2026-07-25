use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extensions::{InstallationOwner, SharedExtensionRegistry};
use ironclaw_host_api::{EffectKind, ExtensionId, RuntimeKind, UserId};
use ironclaw_product::{RebornOperatorToolCatalog, RebornOperatorToolInfo};

use crate::extension_host::extension_lifecycle::RebornLocalExtensionManagementPort;

#[derive(Clone)]
pub(crate) struct ActiveRegistryOperatorToolCatalog {
    registry: Arc<SharedExtensionRegistry>,
    synthetic_tools: Arc<[RebornOperatorToolInfo]>,
    /// Source of the installation owner-by-extension map (#5459 P1). Present
    /// for the local-dev runtime; `None` for assemblies without extension
    /// management, where every registry tool is treated as tenant-shared
    /// (there is no per-user install path to leak).
    owner_source: Option<Arc<RebornLocalExtensionManagementPort>>,
}

impl ActiveRegistryOperatorToolCatalog {
    pub(crate) fn new(
        registry: Arc<SharedExtensionRegistry>,
        synthetic_tools: Vec<RebornOperatorToolInfo>,
        owner_source: Option<Arc<RebornLocalExtensionManagementPort>>,
    ) -> Self {
        Self {
            registry,
            synthetic_tools: Arc::from(synthetic_tools),
            owner_source,
        }
    }
}

/// Owner data available to one `list_operator_tools` read.
enum OwnerVisibility {
    /// No extension management wired: no per-user install path exists, so
    /// every registry tool is tenant-shared (pre-#5459 behavior).
    AllShared,
    /// Owner-aware assembly with a healthy owner map.
    Owners(std::collections::BTreeMap<ExtensionId, InstallationOwner>),
    /// Owner-aware assembly whose owner map could not be read. Install-backed
    /// tools must fail CLOSED — an empty map is indistinguishable from
    /// "no private owners" (#5525 review).
    Unavailable,
}

#[async_trait]
impl RebornOperatorToolCatalog for ActiveRegistryOperatorToolCatalog {
    async fn list_operator_tools(&self, caller: &UserId) -> Vec<RebornOperatorToolInfo> {
        // #5459 P1: the settings/tools catalog is read by any authenticated
        // member, so it MUST hide another user's private tool. The global
        // registry carries no owner, so join the installation owner map and
        // keep an install-backed capability only when its provider's owner row
        // says it is tenant-shared or owned by `caller`. Host-authored
        // builtins (`FirstParty`/`System` runtime — kinds the manifest wire
        // format cannot even declare) have no install path and stay visible.
        let owner_by_extension = match &self.owner_source {
            Some(port) => match port.installation_owners().await {
                Ok(owners) => OwnerVisibility::Owners(owners),
                Err(error) => {
                    tracing::warn!(
                        %error,
                        "settings tool catalog could not read installation owners; \
                         hiding install-backed registry tools for this read"
                    );
                    OwnerVisibility::Unavailable
                }
            },
            None => OwnerVisibility::AllShared,
        };
        let snapshot = self.registry.snapshot();
        let mut tools = snapshot
            .capabilities()
            .filter(|descriptor| match &owner_by_extension {
                OwnerVisibility::AllShared => true,
                _ if matches!(
                    descriptor.runtime,
                    RuntimeKind::FirstParty | RuntimeKind::System
                ) =>
                {
                    true
                }
                // Fail closed on a missing owner row: a published
                // install-backed capability without one is anomalous and could
                // be private (#5525 review).
                OwnerVisibility::Owners(owners) => owners
                    .get(&descriptor.provider)
                    .is_some_and(|owner| owner.visible_to(caller)),
                OwnerVisibility::Unavailable => false,
            })
            .map(|descriptor| RebornOperatorToolInfo {
                capability_id: descriptor.id.clone(),
                provider: descriptor.provider.clone(),
                description: Arc::<str>::from(descriptor.description.as_str()),
                default_permission: descriptor.default_permission,
                effects: Arc::<[EffectKind]>::from(descriptor.effects.clone()),
            })
            .collect::<Vec<_>>();
        tools.extend(self.synthetic_tools.iter().cloned());
        tools
    }
}
