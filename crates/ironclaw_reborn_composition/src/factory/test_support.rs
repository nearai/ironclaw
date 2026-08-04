use super::*;

#[cfg(feature = "test-support")]
use ironclaw_host_api::{
    action::NetworkPolicy,
    capability::{CapabilityGrant, EffectKind, GrantConstraints},
    ids::{CapabilityGrantId, ExtensionId},
    scope::Principal,
};
use ironclaw_product_contracts::channel_config::ChannelConfigProductService;
#[cfg(feature = "test-support")]
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};

/// Harness-facing wiring for
/// [`RebornRuntimeStores::start_channel_host_assembly_for_test`]: the test group
/// supplies its own run-world services; everything else is production.
#[cfg(any(test, feature = "test-support"))]
pub struct ChannelHostAssemblyTestWiring {
    pub thread_service: Arc<dyn SessionThreadService>,
    pub turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator>,
    pub identity: ironclaw_extension_host::channel_host::ChannelHostIdentity,
    pub run_delivery_settings: ironclaw_product::RunDeliverySettings,
}

#[allow(
    dead_code,
    reason = "test-support helper methods are consumed selectively by downstream integration harnesses"
)]
impl RebornRuntimeStores {
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn local_runtime_for_test(&self) -> Option<&Self> {
        Some(self)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn approval_requests_for_test(&self) -> &Arc<ComposedApprovalRequestStore> {
        &self.approval_requests
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn capability_leases_for_test(&self) -> &Arc<ComposedCapabilityLeaseStore> {
        &self.capability_leases
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn capability_policy_for_test(&self) -> &Arc<BuiltinCapabilityPolicy> {
        &self.capability_policy
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn persistent_approval_policies_for_test(
        &self,
    ) -> &Arc<ComposedPersistentApprovalPolicyStore> {
        &self.persistent_approval_policies
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn tool_permission_overrides_for_test(
        &self,
    ) -> &Arc<ComposedToolPermissionOverrideStore> {
        &self.tool_permission_overrides
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn auto_approve_settings_for_test(&self) -> &Arc<ComposedAutoApproveSettingStore> {
        &self.auto_approve_settings
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn external_tool_catalog_for_test(&self) -> &Arc<dyn ExternalToolCatalog> {
        &self.external_tool_catalog
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn outbound_delivery_targets_for_test(
        &self,
    ) -> &Arc<crate::outbound::MutableOutboundDeliveryTargetRegistry> {
        &self.outbound_delivery_targets
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn outbound_preferences_for_test(
        &self,
    ) -> &Arc<dyn CommunicationPreferenceRepository> {
        &self.outbound_preferences
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn skill_auto_activate_learned_for_test(&self) -> &Arc<AtomicBool> {
        &self.skill_auto_activate_learned
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn outbound_state_for_test(
        &self,
    ) -> &Arc<dyn ironclaw_outbound::OutboundStateStorePort> {
        &self.outbound_state
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn delivered_gate_routes_for_test(&self) -> &Arc<dyn DeliveredGateRouteStore> {
        &self.delivered_gate_routes
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn triggered_run_delivery_for_test(&self) -> &Arc<dyn TriggeredRunDeliveryStore> {
        &self.triggered_run_delivery
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn in_memory_budget_event_sink_for_test(
        &self,
    ) -> &Arc<ironclaw_resources::InMemoryBudgetEventSink> {
        &self.in_memory_budget_event_sink
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn admin_configuration_uses_for_test(
        &self,
    ) -> &Arc<Vec<AdminConfigurationCatalogUse>> {
        &self.admin_configuration_uses
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn channel_disconnect_slot_for_test(
        &self,
    ) -> &Arc<std::sync::OnceLock<Arc<dyn ironclaw_product::ChannelConnectionService>>> {
        &self.channel_disconnect_slot
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn skill_mounts_for_test(&self) -> &MountView {
        &self.skill_mounts
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn memory_mounts_for_test(&self) -> &MountView {
        &self.memory_mounts
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn system_extensions_lifecycle_mounts_for_test(&self) -> &MountView {
        &self.system_extensions_lifecycle_mounts
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn skill_filesystem_for_test(
        &self,
    ) -> &Arc<ScopedFilesystem<CompositeRootFilesystem>> {
        &self.skill_filesystem
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn workspace_filesystem_for_test(
        &self,
    ) -> &Arc<ScopedFilesystem<CompositeRootFilesystem>> {
        &self.workspace_filesystem
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn extension_filesystem_for_test(&self) -> &Arc<CompositeRootFilesystem> {
        &self.extension_filesystem
    }

    /// The deployment's workspace mount policy, for tests that build a
    /// production-shaped capability-port factory.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn workspace_mount_policy_for_test(
        &self,
    ) -> &crate::runtime_mounts::WorkspaceMountPolicy {
        &self.workspace_mounts
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn standalone_storage_root_for_direct_test(&self) -> &PathBuf {
        self.standalone_storage_root
            .as_ref()
            .expect("local runtime storage root")
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn default_system_prompt_path_for_test(&self) -> &PathBuf {
        self.default_system_prompt_path
            .as_ref()
            .expect("local runtime default system prompt path")
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn extension_registry_for_test(&self) -> &Arc<ExtensionRegistry> {
        &self.extension_registry
    }

    /// The shared scoped secret store backing this composition.
    pub(crate) fn secret_store(&self) -> Arc<dyn ironclaw_secrets::SecretStorePort> {
        Arc::clone(&self.secret_store)
    }
    /// The composed generic channel ingress (router + per-extension
    /// registration surface), when this composition path built the generic
    /// extension host (extension-runtime P4).
    pub(crate) fn extension_ingress_parts(
        &self,
    ) -> Option<ironclaw_extension_host::extension_ingress::ExtensionIngressParts> {
        self.extension_ingress.clone()
    }

    /// Mint (or rotate) a pairing code through the composed generic pairing
    /// service — tests only. Mirrors the production `pairing/mint` route
    /// handler in `channel_pairing_serve`; returns the code text.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) async fn pairing_mint_for_test(
        &self,
        extension_id: &str,
        user_id: &ironclaw_host_api::ids::UserId,
    ) -> Option<String> {
        let service = self.channel_pairing.as_ref()?.get(extension_id)?;
        service
            .issue_or_rotate(user_id)
            .await
            .ok()
            .map(|issue| issue.code.as_str().to_string())
    }

    /// Mint the full product-safe pairing presentation through the composed
    /// generic service — tests only. Mirrors `PairingIssueBody::from` in the
    /// production `pairing/mint` route so caller-level tests can pin the code,
    /// deep-link, and expiry inputs consumed by the QR/countdown UI.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) async fn pairing_issue_for_test(
        &self,
        extension_id: &str,
        user_id: &ironclaw_host_api::ids::UserId,
    ) -> Option<(String, Option<String>, chrono::DateTime<chrono::Utc>)> {
        let service = self.channel_pairing.as_ref()?.get(extension_id)?;
        service.issue_or_rotate(user_id).await.ok().map(|issue| {
            (
                issue.code.as_str().to_string(),
                issue.deep_link,
                issue.expires_at,
            )
        })
    }

    /// Consume a pairing code through the composed generic service — tests
    /// only. Mirrors the production channel-ingress pairing interceptor and
    /// dispatches the same provider-keyed auth continuation. Integration
    /// groups supply their separately-built shared turn world so the
    /// continuation can see the runs that group actually executes; production
    /// composition uses one coordinator/store and needs no override.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) async fn pairing_consume_for_test(
        &self,
        extension_id: &str,
        authenticated_installation_id: &str,
        raw_code: &str,
        actor: (&str, &str, Option<&str>, &str),
        turn_world: (
            Arc<dyn ironclaw_turns::TurnCoordinator>,
            Arc<dyn ironclaw_processes::ProcessGateQuerySource<Error = ironclaw_turns::TurnError>>,
            ironclaw_host_api::ids::TenantId,
        ),
    ) -> Result<Option<ironclaw_host_api::ids::UserId>, String> {
        let (actor_kind, external_actor_id, conversation_space_id, conversation_id) = actor;
        let Some(service) = self
            .channel_pairing
            .as_ref()
            .and_then(|registry| registry.get(extension_id))
        else {
            return Ok(None);
        };
        let installation_id =
            ironclaw_product::AdapterInstallationId::new(authenticated_installation_id)
                .map_err(|error| error.to_string())?;
        let outcome = service
            .consume(
                &installation_id,
                raw_code,
                actor_kind,
                external_actor_id,
                conversation_space_id,
                conversation_id,
            )
            .await
            .map_err(|error| error.to_string())?;
        let paired_user = match outcome {
            ironclaw_extension_host::channel_pairing::ChannelPairingConsumeOutcome::Paired {
                user_id,
            }
            | ironclaw_extension_host::channel_pairing::ChannelPairingConsumeOutcome::AlreadyPairedSameUser {
                user_id,
            } => Some(user_id),
            ironclaw_extension_host::channel_pairing::ChannelPairingConsumeOutcome::AlreadyBoundToOtherUser
            | ironclaw_extension_host::channel_pairing::ChannelPairingConsumeOutcome::ExpiredOrUnknown => None,
        };
        if let Some(user_id) = paired_user.as_ref() {
            let (turn_coordinator, turn_state, tenant_id) = turn_world;
            let continuation = auth_continuation_dispatcher(turn_coordinator, Some(turn_state));
            service
                .dispatch_pairing_completion_with_for_test(user_id, tenant_id, continuation)
                .await
                .map_err(|error| error.to_string())?;
        }
        Ok(paired_user)
    }

    /// The caller's pairing connection state through the composed generic
    /// pairing service — tests only. Mirrors the production `pairing/status`
    /// route handler and the channel-connection service read.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) async fn pairing_connected_for_test(
        &self,
        extension_id: &str,
        user_id: &ironclaw_host_api::ids::UserId,
    ) -> Option<bool> {
        let service = self.channel_pairing.as_ref()?.get(extension_id)?;
        service
            .status_for(user_id)
            .await
            .ok()
            .map(|status| status.connected)
    }

    /// The generic delivery coordinator (extension-runtime §5.4), when this
    /// composition path built the channel egress transport.
    pub(crate) fn delivery_coordinator(
        &self,
    ) -> Option<Arc<ironclaw_product::DeliveryCoordinator>> {
        self.delivery_coordinator.clone()
    }

    /// The generic `[channel.config]` configure port (extension-runtime
    /// §6.4): the production surface the WebUI setup service and the
    /// lifecycle configure action route operator channel config through.
    /// `None` without a standalone runtime.
    pub(crate) fn channel_config_service(&self) -> Option<Arc<dyn ChannelConfigProductService>> {
        let service = self.channel_config_service.clone();
        Some(Arc::new(
            ironclaw_extension_manager::RebornChannelConfigProductService::new(service),
        ))
    }

    /// Test-support flavor of [`Self::start_channel_host_assembly`]: the
    /// integration harness supplies its own run-world services (thread
    /// service, turn coordinator, identity) because the harness's runs
    /// execute on the test group's shared turn runtime, not this composed
    /// runtime's. Everything else (snapshot watch, ingress registry,
    /// channel-config secret storage, workflow state substrate, delivery
    /// coordinator + outbound stores) is the production wiring.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn start_channel_host_assembly_for_test(
        &self,
        wiring: ChannelHostAssemblyTestWiring,
    ) -> Option<Arc<ironclaw_extension_host::channel_host::GenericChannelHostAssembly>> {
        let admin_users =
            crate::extension_host_assembly::channel_admin_users(self, &wiring.identity);
        crate::extension_host_assembly::start_channel_host_from_stores(
            self,
            crate::extension_host_assembly::ChannelHostAssemblyWiring {
                thread_service: wiring.thread_service,
                turn_coordinator: wiring.turn_coordinator,
                approval_interaction: None,
                auth_interaction: None,
                identity: wiring.identity,
                approval_context: None,
                blocked_auth_prompts: None,
                auth_flow_cancel: None,
                run_delivery_settings: wiring.run_delivery_settings,
                admin_users,
            },
        )
    }

    /// Test-support access to the shared scoped secret store backing the
    /// composed runtime.
    #[cfg(feature = "test-support")]
    pub(crate) fn secret_store_for_test(&self) -> Arc<dyn ironclaw_secrets::SecretStorePort> {
        Arc::clone(&self.secret_store)
    }

    /// Read-write project-scoped workspace filesystem, built over
    /// `extension_filesystem` + `workspace_mounts`.
    /// `None` when no local runtime is composed.
    ///
    /// This deliberately does NOT reuse `workspace_filesystem`:
    /// that handle is intentionally read-only (it backs setup-marker reads —
    /// see `standalone_setup_marker_workspace_filesystem_is_read_only`), so
    /// writing through it fails closed with `PermissionDenied`.
    ///
    /// Single owner of this recipe — both `RebornRuntime::webui_workspace_filesystem`
    /// (production attachment landing) and `standalone_attachment_test_support_for_test`
    /// (C-ATTACH test seam) call this rather than each rebuilding the view, so the
    /// two can never drift apart.
    pub(crate) fn read_write_workspace_filesystem(
        &self,
    ) -> Option<Arc<ScopedFilesystem<CompositeRootFilesystem>>> {
        crate::runtime_mounts::read_write_workspace_filesystem(
            &self.extension_filesystem,
            &self.workspace_mounts,
        )
    }

    #[cfg(feature = "test-support")]
    pub(crate) fn standalone_approval_test_parts(&self) -> Option<RebornApprovalTestParts> {
        let approval_requests: Arc<dyn ironclaw_approvals::ApprovalRequestStorePort> =
            self.approval_requests.clone();
        let capability_leases: Arc<dyn ironclaw_authorization::CapabilityLeaseStorePort> =
            self.capability_leases.clone();
        // Build over the same shared composite root production `capability_wiring`
        // uses, so these test-support stores persist across the group's
        // threads/turns and round-trip identically to production.
        let capability_store_filesystem =
            crate::wrap_scoped(Arc::clone(&self.extension_filesystem));
        let gate_record_store: Arc<dyn ironclaw_approvals::GateRecordStorePort> = Arc::new(
            ironclaw_approvals::GateRecordStore::new(Arc::clone(&capability_store_filesystem)),
        );
        let replay_payload_store: Arc<dyn ironclaw_capabilities::ReplayPayloadStorePort> = Arc::new(
            ironclaw_capabilities::ReplayPayloadStore::new(capability_store_filesystem),
        );
        Some(RebornApprovalTestParts {
            approval_requests,
            capability_leases,
            gate_record_store,
            replay_payload_store,
        })
    }

    #[cfg(feature = "test-support")]
    pub(crate) fn standalone_auto_approve_settings_for_test(
        &self,
    ) -> Option<Arc<dyn ironclaw_approvals::AutoApproveSettingStorePort>> {
        let auto_approve_settings: Arc<dyn ironclaw_approvals::AutoApproveSettingStorePort> =
            self.auto_approve_settings.clone();
        Some(auto_approve_settings)
    }

    /// Test-support access to the extension installation store for this
    /// composition. Returns `None` for production-profile compositions that did
    /// not wire up standalone extension management.
    ///
    /// Mirrors the `installation_store` that `build_local_runtime` wires into
    /// `RebornLocalExtensionManagementPort`. For tests only — zero bytes
    /// shipped in production builds.
    #[cfg(feature = "test-support")]
    pub(crate) fn extension_installation_store_for_test(
        &self,
    ) -> Option<Arc<dyn ironclaw_extensions::ExtensionInstallationStorePort>> {
        Some(self.extension_management.installation_store_for_test())
    }

    /// Test-support access to the standalone memory filesystem that backs the
    /// user-profile source (E-PROFILE seam). This is the raw `RootFilesystem`
    /// that `MemoryBackedUserProfileSource` reads `context/profile.json` from and
    /// that the `profile_set` capability writes through, enabling a profile
    /// write→read-back round-trip at the integration tier. Returns `None` for
    /// production-profile compositions without a standalone runtime.
    #[cfg(feature = "test-support")]
    pub(crate) fn standalone_profile_filesystem_for_test(
        &self,
    ) -> Option<Arc<dyn ironclaw_filesystem::RootFilesystem>> {
        Some(Arc::clone(&self.extension_filesystem) as Arc<dyn ironclaw_filesystem::RootFilesystem>)
    }

    /// Test-support access to the standalone project service backing the synthetic
    /// `project_create` capability (E-PROJ seam). Returns `None` for
    /// production-profile compositions without a standalone runtime.
    #[cfg(feature = "test-support")]
    pub(crate) fn standalone_project_service_for_test(&self) -> Option<Arc<dyn ProjectService>> {
        Some(Arc::clone(&self.project_service))
    }

    /// Test-support access to the standalone session thread service (durable
    /// tool-result projection seam, issue #5838). This is the SAME `Arc`
    /// production's `capability_wiring` passes to
    /// `StagedCapabilityIo::new_with_durable_previews` and to the
    /// `result_read` synthetic capability, so a harness built over this
    /// `RebornServices` can drive its own real `StagedCapabilityIo` through
    /// `staged_capability_io_for_test`. Returns `None` for production-profile
    /// compositions without a standalone runtime.
    #[cfg(feature = "test-support")]
    pub(crate) fn standalone_thread_service_for_test(
        &self,
    ) -> Option<Arc<dyn ironclaw_threads::SessionThreadService>> {
        Some(Arc::clone(&self.thread_service))
    }

    /// Test-support access to the standalone communication-preference repository
    /// (W6-COLD-SPOTS seam). This is the SAME `Arc` that `build_local_runtime_runtime_stores`
    /// wires into `RebornRuntimeStores::outbound_preferences` via
    /// `build_outbound_stores`, for tests only. Returns `None` for
    /// production-profile compositions without a standalone runtime.
    #[cfg(feature = "test-support")]
    pub(crate) fn standalone_outbound_preferences_for_test(
        &self,
    ) -> Option<Arc<dyn CommunicationPreferenceRepository>> {
        Some(Arc::clone(&self.outbound_preferences))
    }

    /// Test-support access to the on-disk standalone storage root (W6-COLD-SPOTS
    /// seam), for tests only — mirrors the same `standalone_storage_root`
    /// that `build_local_runtime_runtime_stores` establishes in production. Used to reopen
    /// a fresh outbound-preferences store at the same root (see
    /// `open_standalone_outbound_preferences_store_for_test`). Returns `None` for
    /// production-profile compositions without a standalone runtime.
    #[cfg(feature = "test-support")]
    pub(crate) fn standalone_storage_root_for_test(&self) -> Option<PathBuf> {
        self.standalone_storage_root.clone()
    }

    /// Single owner of the `ProjectScopedAttachmentReader` construction recipe
    /// over `workspace_filesystem` (mirrors the
    /// `read_write_workspace_filesystem` "single owner" pattern above). The
    /// concrete reader implements both `LoopAttachmentReadPort` and
    /// `InboundAttachmentReader`, so callers cast the same `Arc` into whichever
    /// trait object they need instead of re-deriving the recipe. Test-support
    /// only; zero bytes shipped in production builds.
    #[cfg(feature = "test-support")]
    fn standalone_workspace_attachment_reader_for_test(
        &self,
    ) -> Option<Arc<ironclaw_product::ProjectScopedAttachmentReader<CompositeRootFilesystem>>> {
        Some(Arc::new(
            ironclaw_product::ProjectScopedAttachmentReader::new(Arc::clone(
                &self.workspace_filesystem,
            )),
        ))
    }

    /// Test-support access to the attachment read port + inbound lander backing
    /// the C-ATTACH seam. The read port is built over `workspace_filesystem`,
    /// exactly like production's `attachment_read_port` (`runtime.rs` ~line 3328) —
    /// that handle is intentionally read-only (it backs setup-marker reads), which
    /// is fine for reading. The lander is built over the SAME read-write view
    /// `RebornRuntime::webui_workspace_filesystem` uses in production, via the
    /// shared [`Self::read_write_workspace_filesystem`] helper — landing through
    /// the read-only `workspace_filesystem` handle fails closed with
    /// `PermissionDenied`. Bundled into one accessor (rather than two, mirroring
    /// `standalone_profile_filesystem_for_test` / `standalone_project_service_for_test`
    /// above) because the two are always populated together. Returns `None` for
    /// production-profile compositions without a standalone runtime.
    #[cfg(feature = "test-support")]
    pub(crate) fn standalone_attachment_test_support_for_test(
        &self,
    ) -> Option<AttachmentTestSupport> {
        let read_port = self.standalone_workspace_attachment_reader_for_test()?
            as Arc<dyn ironclaw_loop_host::LoopAttachmentReadPort>;
        let read_write_workspace_filesystem = self.read_write_workspace_filesystem()?;
        Some(AttachmentTestSupport {
            read_port,
            lander: Arc::new(ironclaw_attachments::ProjectScopedAttachmentLander::new(
                read_write_workspace_filesystem,
            )),
        })
    }

    /// Test-support access to the standalone per-tool permission override store
    /// (C-SYNTH outbound seam). Backs `StoreApprovalSettingsProvider::tool_override`,
    /// which the synthetic `outbound_delivery_target_set` capability consults for
    /// its settings decision — a `Disabled` override drives the `policy_denied`
    /// route. Mirrors `standalone_auto_approve_settings_for_test`; `None` for
    /// production-profile compositions without a standalone runtime.
    #[cfg(feature = "test-support")]
    pub(crate) fn standalone_tool_permission_overrides_for_test(
        &self,
    ) -> Option<Arc<dyn ironclaw_approvals::ToolPermissionOverrideStorePort>> {
        let overrides: Arc<dyn ironclaw_approvals::ToolPermissionOverrideStorePort> =
            self.tool_permission_overrides.clone();
        Some(overrides)
    }

    /// Test-support access to the standalone persistent approval-policy store
    /// (C-SYNTH outbound seam). Backs `StoreApprovalSettingsProvider::tool_always_allow`.
    /// Mirrors `standalone_auto_approve_settings_for_test`; `None` for
    /// production-profile compositions without a standalone runtime.
    #[cfg(feature = "test-support")]
    pub(crate) fn standalone_persistent_approval_policies_for_test(
        &self,
    ) -> Option<Arc<dyn ironclaw_approvals::PersistentApprovalPolicyStorePort>> {
        let policies: Arc<dyn ironclaw_approvals::PersistentApprovalPolicyStorePort> =
            self.persistent_approval_policies.clone();
        Some(policies)
    }

    /// SAME live trigger repository `trigger_repository_for_durable_backend` builds and
    /// capability dispatch uses (the `trigger_repository` binding in
    /// `build_local_runtime`, above) — not a fresh reopen. Contrast
    /// [`open_standalone_trigger_repository_for_test`] (independent reopened
    /// repo, for persistence/reopen tests). Backs the cold-LIST scenario
    /// (W5-WEBUI-API-1 Enabler B.1). Test-support only; zero bytes shipped in
    /// production builds. `None` w/o standalone runtime.
    #[cfg(feature = "test-support")]
    pub(crate) fn standalone_shared_trigger_repository_for_test(
        &self,
    ) -> Option<Arc<dyn ironclaw_triggers::TriggerRepository>> {
        Some(Arc::clone(&self.trigger_repository))
    }

    /// WebUI-facing `InboundAttachmentReader` view over the standalone
    /// workspace filesystem, mirroring production's `webui.rs`
    /// (`ProjectScopedAttachmentReader` construction at `webui.rs` ~line 153).
    /// Shares [`Self::standalone_workspace_attachment_reader_for_test`]'s
    /// construction recipe with [`Self::standalone_attachment_test_support_for_test`]
    /// rather than re-deriving it. Test-support only; zero bytes shipped in
    /// production builds. `None` w/o a standalone runtime.
    #[cfg(feature = "test-support")]
    pub(crate) fn standalone_inbound_attachment_reader_for_test(
        &self,
    ) -> Option<Arc<dyn ironclaw_attachments::InboundAttachmentReader>> {
        Some(self.standalone_workspace_attachment_reader_for_test()?
            as Arc<dyn ironclaw_attachments::InboundAttachmentReader>)
    }

    /// C-JOURNEY: publish a bundled first-party WASM extension package (e.g. a
    /// WASM tool extension) directly into the standalone active-extension
    /// registry + trust policy, bypassing the multi-turn
    /// `builtin.extension_install` → `builtin.extension_activate` capability
    /// handshake. Reaches the SAME `ActiveExtensionPublisher::publish` step
    /// `activate()` calls (`extension_lifecycle.rs`) — the model-visible
    /// dispatchable surface — so a harness that needs a bundled tool's
    /// capabilities reachable for dispatch without scripting install/activate
    /// turns can seed it at construction time. Returns `None` for
    /// production-profile
    /// compositions without a standalone runtime (mirrors
    /// `extension_installation_store_for_test`).
    #[cfg(feature = "test-support")]
    pub(crate) async fn publish_bundled_extension_for_test(
        &self,
        package: &ironclaw_extensions::ExtensionPackage,
        resolved: Option<&ironclaw_extensions::ResolvedExtensionManifest>,
    ) -> Option<Result<(), ironclaw_product::ProductSurfaceFailure>> {
        let extension_management = &self.extension_management;
        Some(
            extension_management
                .publish_bundled_package_for_test(package, resolved)
                .await
                .map_err(ironclaw_product::ProductSurfaceFailure::from),
        )
    }

    /// Register a static channel-egress credential mapping
    /// `(extension_id, handle) → material`, consulted ahead of the scoped
    /// secret store — the test stand-in for `[channel.config]` secret
    /// storage until the configure surface lands (P6/H). Returns `false`
    /// when this composition built no channel-egress credential bridging
    /// (no generic extension host).
    #[cfg(feature = "test-support")]
    pub(crate) fn register_static_channel_egress_credentials_for_test(
        &self,
        entries: Vec<(String, String, ironclaw_secrets::SecretMaterial)>,
    ) -> bool {
        let Some(bridges) = &self.channel_egress_credential_bridges else {
            return false;
        };
        bridges.register(Arc::new(
            ironclaw_extension_host::channel_egress::StaticChannelEgressCredentials::new(entries),
        ));
        true
    }

    /// The delivery coordinator's outbound stores — the SAME instances the
    /// factory handed the coordinator (`outbound_state`), the gate-route
    /// recorder (`delivered_gate_routes`), and the preference service
    /// (`outbound_preferences`). Integration proofs build generic
    /// run-delivery components over these so observer and coordinator share
    /// one delivery ledger. `None` without a standalone runtime.
    #[cfg(feature = "test-support")]
    #[allow(clippy::type_complexity)]
    pub(crate) fn outbound_delivery_stores_for_test(
        &self,
    ) -> Option<(
        Arc<dyn ironclaw_outbound::OutboundStateStorePort>,
        Arc<dyn ironclaw_outbound::DeliveredGateRouteStore>,
        Arc<dyn ironclaw_outbound::CommunicationPreferenceRepository>,
    )> {
        Some((
            Arc::clone(&self.outbound_state),
            Arc::clone(&self.delivered_gate_routes),
            Arc::clone(&self.outbound_preferences),
        ))
    }

    /// Test-support authority snapshot for active standalone extensions.
    ///
    /// Binary-E2E harnesses build capability ports at the host-runtime boundary
    /// instead of going through `RefreshingLoopCapabilityPortFactory`, so they need
    /// the same active-extension grants and provider trust that production
    /// standalone recomputes whenever the model-visible surface is refreshed.
    #[cfg(feature = "test-support")]
    pub(crate) async fn standalone_active_extension_authority_for_test(
        &self,
        grantee: &ExtensionId,
    ) -> Option<Result<ActiveExtensionAuthorityForTest, ironclaw_product::ProductSurfaceFailure>>
    {
        let extension_management = &self.extension_management;
        Some(active_extension_authority_for_test(extension_management, grantee).await)
    }
}

#[cfg(feature = "test-support")]
pub struct ActiveExtensionAuthorityForTest {
    pub grants: Vec<CapabilityGrant>,
    pub provider_trust: Vec<(ExtensionId, TrustDecision)>,
}

#[cfg(feature = "test-support")]
pub(crate) async fn active_extension_authority_for_test(
    extension_management: &RebornLocalExtensionManagementPort,
    grantee: &ExtensionId,
) -> Result<ActiveExtensionAuthorityForTest, ironclaw_product::ProductSurfaceFailure> {
    let active_capabilities = extension_management
        .active_model_visible_capabilities()
        .await?;
    let grants = active_capabilities
        .iter()
        .map(|capability| CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: capability.id.clone(),
            grantee: Principal::Extension(grantee.clone()),
            issued_by: Principal::HostRuntime,
            constraints: active_extension_grant_constraints_for_test(capability),
        })
        .collect();
    let mut effects_by_provider: std::collections::BTreeMap<ExtensionId, Vec<EffectKind>> =
        std::collections::BTreeMap::new();
    for capability in &active_capabilities {
        let effects = effects_by_provider
            .entry(capability.provider.clone())
            .or_default();
        for effect in &capability.effects {
            if !effects.contains(effect) {
                effects.push(*effect);
            }
        }
    }
    let provider_trust = effects_by_provider
        .into_iter()
        .map(|(provider, allowed_effects)| {
            (
                provider,
                TrustDecision {
                    effective_trust: EffectiveTrustClass::user_trusted(),
                    authority_ceiling: AuthorityCeiling {
                        allowed_effects,
                        max_resource_ceiling: None,
                    },
                    provenance: TrustProvenance::AdminConfig,
                    evaluated_at: chrono::Utc::now(),
                },
            )
        })
        .collect();
    Ok(ActiveExtensionAuthorityForTest {
        grants,
        provider_trust,
    })
}

#[cfg(feature = "test-support")]
fn active_extension_grant_constraints_for_test(
    capability: &ironclaw_extension_host::ActiveExtensionCapability,
) -> GrantConstraints {
    GrantConstraints {
        allowed_effects: capability.effects.clone(),
        mounts: MountView::default(),
        network: active_extension_network_policy_for_test(capability),
        secrets: {
            let mut handles = Vec::new();
            for credential in &capability.runtime_credentials {
                if !handles.contains(&credential.handle) {
                    handles.push(credential.handle.clone());
                }
            }
            handles
        },
        resource_ceiling: None,
        expires_at: None,
        max_invocations: None,
    }
}

#[cfg(feature = "test-support")]
fn active_extension_network_policy_for_test(
    capability: &ironclaw_extension_host::ActiveExtensionCapability,
) -> NetworkPolicy {
    // Delegate to the production manifest-egress policy builder (gsuite +
    // web-access declare their egress in their manifests now — no per-provider
    // special-case, and no first-party dependency in this test-support seam).
    ironclaw_extension_host::capability_surface::extension_network_policy(capability)
}

/// Bundle returned by [`RebornRuntimeStores::standalone_attachment_test_support_for_test`]
/// (C-ATTACH seam). Test-support only — zero bytes shipped in production builds.
#[cfg(feature = "test-support")]
#[derive(Clone)]
pub struct AttachmentTestSupport {
    pub read_port: Arc<dyn ironclaw_loop_host::LoopAttachmentReadPort>,
    pub lander: Arc<dyn ironclaw_attachments::InboundAttachmentLander>,
}

#[cfg(feature = "test-support")]
#[derive(Clone)]
pub struct RebornApprovalTestParts {
    pub approval_requests: Arc<dyn ironclaw_approvals::ApprovalRequestStorePort>,
    pub capability_leases: Arc<dyn ironclaw_authorization::CapabilityLeaseStorePort>,
    /// Durable model-visible gate-record store, shared across the group's threads
    /// so a gate raised on one thread can be read back on another.
    pub gate_record_store: Arc<dyn ironclaw_approvals::GateRecordStorePort>,
    /// Durable host-private replay-payload store (§5.3 Stage 2a-i), shared across
    /// the group's threads/turns so a gate/auth resume reconstitutes the input the
    /// original raise persisted. Backed by the same composite root as production
    /// `capability_wiring`, so the harness store round-trips identically.
    pub replay_payload_store: Arc<dyn ironclaw_capabilities::ReplayPayloadStorePort>,
}

/// Thin void wrapper over [`build_default_database_roots`] for
/// `#[cfg(feature = "test-support")]` callers that need to mount the standalone
/// database roots but don't need the opaque `DurableBackend` handle
/// (which is private to this module).
///
/// Used by `test_support::build_default_database_roots_for_test`.
#[cfg(feature = "test-support")]
pub(crate) async fn mount_default_database_roots(
    root: &Path,
    composite: &mut CompositeRootFilesystem,
) -> Result<(), RebornBuildError> {
    build_default_database_roots(root, composite)
        .await
        .map(|_| ())
}

/// Test-only (T5 restart-survival seam): open a FRESH standalone root
/// filesystem at an existing `storage_root`, for reconstructing the generic
/// channel-identity store the way production boot does
/// (`build_runtime_substrate` → `FilesystemChannelIdentityStore::new` over the
/// composed root). `libsql`-only: the `EmbeddedLibsql` non-libsql
/// arm mounts a fresh `InMemoryBackend`, which could only ever report
/// absence. Tests only; zero bytes in production.
#[cfg(feature = "test-support")]
pub(crate) async fn open_standalone_root_filesystem_for_test(
    storage_root: &Path,
) -> Result<Arc<dyn RootFilesystem>, RebornBuildError> {
    let workspace_root = storage_root.join("workspace");
    let bundle = build_filesystem(
        storage_root,
        &workspace_root,
        None,
        DurableStorageInput::EmbeddedLibsql,
    )
    .await?;
    Ok(bundle.filesystem)
}

/// Test-only (E-DURABLE seam): open a FRESH, independent
/// [`ExtensionInstallationStore`] at an existing standalone `storage_root`,
/// paralleling how `assert_reply_persists_after_reopen` opens a fresh libsql
/// handle rather than reusing the live one. Reuses the production
/// [`build_standalone_root_filesystem`] mounts and
/// [`ExtensionInstallationStore::default_state_path`] so the reopen
/// reads the exact durable `/system/extensions/.installations` state the
/// running harness wrote while extension package files still live on disk
/// (mirrors the production install-store load in [`build_runtime_substrate`],
/// above at the `extension_installation_store` binding). The store's virtual
/// state path has no identity dependency for standalone profiles, so no
/// tenant/user context is needed. Tests only; zero bytes in production builds.
#[cfg(feature = "test-support")]
pub(crate) async fn open_standalone_extension_installation_store_for_test(
    storage_root: &Path,
) -> Result<Arc<dyn ironclaw_extensions::ExtensionInstallationStorePort>, RebornBuildError> {
    let workspace_root = storage_root.join("workspace");
    let bundle = build_filesystem(
        storage_root,
        &workspace_root,
        None,
        DurableStorageInput::EmbeddedLibsql,
    )
    .await?;
    let filesystem: Arc<dyn RootFilesystem> = bundle.filesystem;
    let state_path = ExtensionInstallationStore::default_state_path().map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: format!("extension installation state path invalid: {error}"),
        }
    })?;
    let host_ports = ironclaw_host_runtime::default_host_port_catalog().map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: format!("extension host port catalog could not be loaded: {error}"),
        }
    })?;
    let host_api_contracts = product_extension_host_api_contract_registry().map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: format!("extension host API contracts could not be loaded: {error}"),
        }
    })?;
    let store =
        ExtensionInstallationStore::load_at(filesystem, state_path, host_ports, host_api_contracts)
            .await
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("extension installation state could not be reopened: {error}"),
            })?;
    Ok(Arc::new(store))
}

/// Test-only (C-DURABLE seam): open a FRESH, independent
/// [`ironclaw_approvals::ApprovalRequestStore`] at an existing standalone
/// `storage_root`, paralleling [`open_standalone_extension_installation_store_for_test`]
/// (same on-disk root; a sibling capability store). Reuses
/// [`mount_default_database_roots`] + the production [`crate::wrap_scoped`]
/// so the reopen mounts + scopes the SAME way `build_local_runtime` does when it
/// first builds `approval_requests` — the reopen path never drifts from
/// production. Tests only; zero bytes in production builds.
#[cfg(feature = "test-support")]
pub(crate) async fn open_standalone_approval_request_store_for_test(
    storage_root: &Path,
) -> Result<Arc<dyn ironclaw_approvals::ApprovalRequestStorePort>, RebornBuildError> {
    let mut composite = CompositeRootFilesystem::new();
    mount_default_database_roots(storage_root, &mut composite).await?;
    let scoped = crate::wrap_scoped(Arc::new(composite));
    Ok(Arc::new(ApprovalRequestStore::new(scoped)))
}

/// W6-COLD-SPOTS: fresh `CommunicationPreferenceRepository` reopen, mirrors
/// [`open_standalone_approval_request_store_for_test`]. Reuses
/// [`crate::outbound_store_assembly::build_outbound_stores`] — the same
/// composition-owned construction the
/// production `build_runtime_stores` path uses — so the reopen path
/// never drifts from production and needs no `disallowed_methods` exception.
/// Tests only.
#[cfg(feature = "test-support")]
pub(crate) async fn open_standalone_outbound_preferences_store_for_test(
    storage_root: &Path,
) -> Result<Arc<dyn CommunicationPreferenceRepository>, RebornBuildError> {
    let mut composite = CompositeRootFilesystem::new();
    mount_default_database_roots(storage_root, &mut composite).await?;
    Ok(
        crate::outbound_store_assembly::build_outbound_stores(Arc::new(composite))
            .outbound_preferences,
    )
}

/// Test-only (W5-WEBUI-API-1 seam): open FRESH, independent
/// [`ironclaw_approvals::ToolPermissionOverrideStore`] /
/// [`ironclaw_approvals::AutoApproveSettingStore`] /
/// [`ironclaw_approvals::PersistentApprovalPolicyStore`] handles at an
/// existing standalone `storage_root`, paralleling
/// [`open_standalone_approval_request_store_for_test`] (same on-disk root;
/// sibling capability stores). Reuses [`mount_default_database_roots`]
/// plus the production [`crate::wrap_scoped`] so the reopen mounts and scopes
/// the SAME way `build_runtime_stores` does when it first builds
/// `tool_permission_overrides` / `auto_approve_settings` /
/// `persistent_approval_policies` (above) — the reopen path never drifts from
/// production. Tests only; zero bytes in production builds.
#[cfg(feature = "test-support")]
pub(crate) async fn open_standalone_approval_settings_stores_for_test(
    storage_root: &Path,
) -> Result<
    (
        Arc<dyn ironclaw_approvals::ToolPermissionOverrideStorePort>,
        Arc<dyn ironclaw_approvals::AutoApproveSettingStorePort>,
        Arc<dyn ironclaw_approvals::PersistentApprovalPolicyStorePort>,
    ),
    RebornBuildError,
> {
    let mut composite = CompositeRootFilesystem::new();
    mount_default_database_roots(storage_root, &mut composite).await?;
    let scoped = crate::wrap_scoped(Arc::new(composite));
    let tool_permission_overrides: Arc<dyn ironclaw_approvals::ToolPermissionOverrideStorePort> =
        Arc::new(ComposedToolPermissionOverrideStore::new(Arc::clone(
            &scoped,
        )));
    let auto_approve_settings: Arc<dyn ironclaw_approvals::AutoApproveSettingStorePort> =
        Arc::new(ComposedAutoApproveSettingStore::new(Arc::clone(&scoped)));
    let persistent_approval_policies: Arc<
        dyn ironclaw_approvals::PersistentApprovalPolicyStorePort,
    > = Arc::new(PersistentApprovalPolicyStore::new(scoped));
    Ok((
        tool_permission_overrides,
        auto_approve_settings,
        persistent_approval_policies,
    ))
}

/// Test-only (C-DURABLE seam): open a FRESH, independent
/// [`ironclaw_triggers::TriggerRepository`] at an existing standalone
/// `storage_root`, paralleling [`open_standalone_extension_installation_store_for_test`].
/// Reuses [`open_standalone_libsql_database`] (the same libSQL-open sequence
/// production uses) AND delegates to [`trigger_repository_for_durable_backend`] for
/// repository construction + migrations, so the reopen path shares the SAME
/// construction code as production standalone wiring — never a second place to
/// update if trigger repository setup changes. Tests only; zero bytes in
/// production builds.
#[cfg(feature = "test-support")]
pub(crate) async fn open_standalone_trigger_repository_for_test(
    storage_root: &Path,
) -> Result<Arc<dyn TriggerRepository>, RebornBuildError> {
    let mut composite = CompositeRootFilesystem::new();
    let backend = build_default_database_roots(storage_root, &mut composite).await?;
    trigger_repository_for_durable_backend(&backend).await
}

#[cfg(all(test, feature = "test-support"))]
mod attachment_seam_tests {
    use ironclaw_host_api::ids::{AgentId, TenantId, UserId};
    use ironclaw_threads::ThreadScope;

    /// Store-level regression for the two C-ATTACH accessors
    /// (`RebornRuntimeStores::standalone_attachment_test_support_for_test` and
    /// `RebornRuntimeStores::standalone_inbound_attachment_reader_for_test`).
    /// The downstream integration harness reaches this seam through the
    /// `RebornRuntime` wrapper's same-named methods, so nothing ever drove the
    /// store-level recipe itself: a regression here — a lander built over the
    /// read-only `workspace_filesystem` handle (which fails closed with
    /// `PermissionDenied`), or a reader pointed at a different mount view than
    /// the lander wrote through — would only surface downstream. Landing real
    /// bytes and reading them back through BOTH returned read views proves the
    /// two accessors hand out usable, mutually consistent ports.
    #[tokio::test]
    async fn standalone_attachment_seams_land_and_read_back_the_same_bytes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = crate::factory::build_runtime_substrate(
            crate::deployment::local_filesystem_build_input(
                "attachment-seam-owner",
                dir.path().join("standalone"),
            ),
        )
        .await
        .expect("standalone services build");

        let support = services
            .standalone_attachment_test_support_for_test()
            .expect("a standalone composition exposes the C-ATTACH seam");
        let inbound_reader = services
            .standalone_inbound_attachment_reader_for_test()
            .expect("a standalone composition exposes the WebUI-facing inbound reader");

        let thread_scope = ThreadScope {
            tenant_id: TenantId::new("attachment-seam-tenant").expect("tenant id"),
            agent_id: AgentId::new("attachment-seam-agent").expect("agent id"),
            project_id: None,
            owner_user_id: Some(UserId::new("attachment-seam-owner").expect("user id")),
            mission_id: None,
        };

        let refs = support
            .lander
            .land(
                &thread_scope,
                "msg-attachment-seam",
                vec![ironclaw_host_api::attachment::InboundAttachment {
                    id: "att-0".to_string(),
                    mime_type: "image/png".to_string(),
                    filename: Some("seam.png".to_string()),
                    bytes: b"attachment-seam-bytes".to_vec(),
                }],
            )
            .await
            .expect("the seam's lander writes through a read-write workspace view");
        let storage_key = refs[0]
            .storage_key
            .as_deref()
            .expect("a landed attachment carries a storage_key");

        assert_eq!(
            support
                .read_port
                .read_attachment_bytes(&thread_scope.to_resource_scope(), storage_key)
                .await
                .expect("the seam's model-injection read port reads the landed bytes"),
            b"attachment-seam-bytes".to_vec(),
        );
        assert_eq!(
            inbound_reader
                .read(&thread_scope, storage_key)
                .await
                .expect("the WebUI-facing inbound reader reads the same landed bytes"),
            b"attachment-seam-bytes".to_vec(),
        );
    }

    /// Store-level regression for `RebornRuntimeStores::channel_config_service`
    /// (extension-runtime §6.4): the port the WebUI setup service and the
    /// lifecycle configure action route operator channel config through.
    ///
    /// Nothing drove this accessor -- the integration harness reaches the seam
    /// through the `RebornRuntime` wrapper's same-named method, so the store
    /// recipe (build the manager-side product port over the composed
    /// `ChannelConfigService`) was only ever compiled, never run. A regression
    /// here -- a port built over the wrong service handle, or one that panics
    /// on its first read -- would surface only downstream. Asking the returned
    /// port about an extension the composition has not installed proves it is
    /// live and answers on the contract's terms: an extension with nothing to
    /// configure projects an empty field list rather than erroring, which is
    /// what makes the WebUI setup view render for it.
    #[tokio::test]
    async fn the_channel_config_seam_hands_out_a_usable_product_port() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = crate::factory::build_runtime_substrate(
            crate::deployment::local_filesystem_build_input(
                "channel-config-seam-owner",
                dir.path().join("standalone"),
            ),
        )
        .await
        .expect("standalone services build");

        let channel_config = services
            .channel_config_service()
            .expect("a standalone composition exposes the §6.4 configure port");
        assert_eq!(
            channel_config
                .field_status(
                    &ironclaw_host_api::ids::ExtensionId::new("not-installed-extension")
                        .expect("extension id")
                )
                .await
                .expect(
                    "an uninstalled extension has nothing to configure, and that is not an error"
                ),
            Vec::new(),
        );
    }
}

/// The ambient shared workspace view a deployment carries, or `None` under a
/// per-caller policy.
///
/// Test-support only: a per-caller deployment has no shared view, and
/// production resolves its view from the run/gate scope instead. Lives here
/// rather than on `WorkspaceMountPolicy` so the production type carries no
/// test-only member.
/// The ambient shared workspace view a test runtime carries.
///
/// A free function rather than a `RebornRuntimeStores` method: the
/// struct-member ratchet
/// (`ironclaw_architecture::reborn_struct_test_support_ratchet`) counts
/// `#[cfg(test-support)]` *members* on production structs, while
/// `check_no_panics.py` requires the item-level `#[cfg]` for the `.expect()`
/// below. A gated free function satisfies both.
///
/// Panics under a per-caller policy, which has no shared view --- such a
/// deployment must be driven through the production seam instead.
#[cfg(test)]
pub(crate) fn workspace_mounts_for_test(stores: &RebornRuntimeStores) -> &MountView {
    shared_workspace_view(&stores.workspace_mounts)
        .expect("test runtime uses a shared workspace mount policy")
}

#[cfg(test)]
pub(crate) fn shared_workspace_view(
    policy: &crate::runtime_mounts::WorkspaceMountPolicy,
) -> Option<&MountView> {
    match policy {
        crate::runtime_mounts::WorkspaceMountPolicy::Shared(view) => Some(view),
        crate::runtime_mounts::WorkspaceMountPolicy::PerCaller => None,
    }
}
