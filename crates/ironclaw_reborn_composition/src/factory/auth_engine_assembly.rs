use super::*;

/// Display name sent with RFC 7591 dynamic client registration.
const DCR_CLIENT_NAME: &str = "Ironclaw";

/// The static vendor-callback base path (`{base}/{vendor}/callback`); the
/// serve layer mounts the matching `{provider}` route.
const PRODUCT_AUTH_OAUTH_ROUTE_BASE: &str = "/api/reborn/product-auth/oauth";

#[derive(Clone)]
pub(super) struct OAuthProviderComposition {
    pub(super) engine: Option<Arc<AuthEngine>>,
    pub(super) client: Option<Arc<dyn AuthProviderClient>>,
    pub(super) gate_driver: Option<Arc<OAuthGateFlowDriver>>,
}

#[derive(Clone, Default)]
pub(super) struct AdminConfigurationCredentialSlot {
    inner: Arc<std::sync::OnceLock<Arc<ComposedExtensionAdminConfigurationResolver>>>,
}

impl AdminConfigurationCredentialSlot {
    pub(super) fn fill(&self, service: Arc<ComposedExtensionAdminConfigurationResolver>) {
        let _ = self.inner.set(service);
    }

    fn get(&self) -> Option<Arc<ComposedExtensionAdminConfigurationResolver>> {
        self.inner.get().cloned()
    }
}

impl fmt::Debug for AdminConfigurationCredentialSlot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdminConfigurationCredentialSlot")
            .field("filled", &self.inner.get().is_some())
            .finish()
    }
}

#[derive(Clone, Default)]
struct CompositionClientCredentials {
    values: BTreeMap<String, SecretString>,
    admin_configuration: Option<AdminConfigurationCredentialSlot>,
}

impl CompositionClientCredentials {
    fn register_static(&mut self, handle: impl Into<String>, value: SecretString) {
        self.values.insert(handle.into(), value);
    }

    fn with_admin_configuration(&mut self, slot: AdminConfigurationCredentialSlot) {
        self.admin_configuration = Some(slot);
    }

    async fn resolve_handle(&self, handle: &str) -> Result<Option<SecretString>, AuthProductError> {
        if let Some(value) = self.values.get(handle) {
            return Ok(Some(value.clone()));
        }
        let Some(service) = self
            .admin_configuration
            .as_ref()
            .and_then(|slot| slot.get())
        else {
            return Ok(None);
        };
        service
            .credential_handle_value(handle)
            .await
            .map_err(|error| {
                tracing::warn!(
                    %error,
                    handle,
                    "administrator client-credential lookup failed"
                );
                AuthProductError::BackendUnavailable
            })
    }
}

impl fmt::Debug for CompositionClientCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompositionClientCredentials")
            .field("handles", &self.values.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[async_trait::async_trait]
impl EngineClientCredentialsSource for CompositionClientCredentials {
    async fn resolve(
        &self,
        vendor: &str,
        credentials: &RecipeClientCredentials,
    ) -> Result<EngineOAuthClientMaterial, AuthProductError> {
        use secrecy::ExposeSecret as _;

        let Some(client_id) = self
            .resolve_handle(credentials.client_id_handle.as_str())
            .await?
        else {
            tracing::debug!(
                vendor,
                handle = credentials.client_id_handle.as_str(),
                "vendor OAuth client id is not configured"
            );
            return Err(AuthProductError::MalformedConfig);
        };
        let client_secret = match &credentials.client_secret_handle {
            None => None,
            Some(handle) => self.resolve_handle(handle.as_str()).await?,
        };
        Ok(EngineOAuthClientMaterial {
            client_id: OAuthClientId::new(client_id.expose_secret())?,
            client_secret,
        })
    }
}

pub(super) fn compose_provider_client(
    configs: Vec<OAuthProviderBackendConfig>,
    dcr_callback: Option<OAuthDcrCallbackConfig>,
    secret_store: Arc<dyn SecretStorePort>,
    runtime_ports: ProductAuthProviderRuntimePorts,
    admin_configuration_credentials: AdminConfigurationCredentialSlot,
    first_party_bundles: &[ironclaw_extension_host::FirstPartyPackageBundle],
    installation_store: Arc<dyn ExtensionInstallationStorePort>,
) -> Result<OAuthProviderComposition, RebornBuildError> {
    let static_recipes = Arc::new(StaticAuthRecipeResolver::new(
        ironclaw_extension_host::AvailableExtensionCatalog::bundled_vendor_recipes(
            first_party_bundles,
        )
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("bundled vendor auth recipes could not be resolved: {error}"),
        })?,
    ));

    let mut client_credentials = CompositionClientCredentials::default();
    for config in &configs {
        register_vendor_client_config(&mut client_credentials, static_recipes.as_ref(), config);
    }
    client_credentials.with_admin_configuration(admin_configuration_credentials);
    let callback_base = dcr_callback
        .map(|dcr| {
            EngineCallbackBase::new(format!(
                "{}{PRODUCT_AUTH_OAUTH_ROUTE_BASE}",
                dcr.callback_origin.trim_end_matches('/')
            ))
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("OAuth callback origin rejected: {error}"),
            })
        })
        .transpose()?
        .or_else(|| {
            configs
                .iter()
                .find_map(|config| callback_base_from_redirect(config.client.redirect_uri.as_str()))
        });

    compose_auth_engine(
        Arc::new(CompositionAuthRecipeResolver {
            static_recipes,
            installed_recipes: ironclaw_extension_host::InstalledManifestAuthRecipeResolver::new(
                installation_store,
            ),
        }),
        client_credentials,
        callback_base,
        secret_store,
        runtime_ports,
    )
}

/// Routes built-in callers to bundled recipes and installed callers to their
/// own durable manifest. These paths must never fall back across the requester
/// boundary: doing so would let an installed extension borrow another recipe.
#[derive(Clone, Debug)]
struct CompositionAuthRecipeResolver {
    static_recipes: Arc<StaticAuthRecipeResolver>,
    installed_recipes: ironclaw_extension_host::InstalledManifestAuthRecipeResolver,
}

#[async_trait::async_trait]
impl AuthRecipeResolver for CompositionAuthRecipeResolver {
    async fn resolve(
        &self,
        requester_extension: Option<&ExtensionId>,
        vendor: &str,
    ) -> Option<ironclaw_auth::ResolvedVendorAuthRecipe> {
        match requester_extension {
            Some(requester_extension) => {
                self.installed_recipes
                    .resolve(Some(requester_extension), vendor)
                    .await
            }
            None => self.static_recipes.resolve(None, vendor).await,
        }
    }
}

fn register_vendor_client_config(
    credentials: &mut CompositionClientCredentials,
    recipes: &StaticAuthRecipeResolver,
    config: &OAuthProviderBackendConfig,
) {
    use secrecy::ExposeSecret as _;

    let Some(resolved) = recipes.recipe_for_vendor(&config.vendor) else {
        tracing::warn!(
            vendor = config.vendor,
            "no bundled recipe for configured OAuth vendor; client material not wired"
        );
        return;
    };
    let ironclaw_host_api::recipe::VendorAuthRecipe::Oauth2Code(recipe) = &resolved.recipe else {
        tracing::warn!(
            vendor = config.vendor,
            "configured OAuth vendor's recipe is not oauth2_code; client material not wired"
        );
        return;
    };
    let Some(handles) = &recipe.client_credentials else {
        tracing::debug!(
            vendor = config.vendor,
            "vendor recipe uses dynamic client registration; static client material ignored"
        );
        return;
    };
    credentials.register_static(
        handles.client_id_handle.as_str(),
        SecretString::from(config.client.client_id.as_str().to_string()),
    );
    if let (Some(secret_handle), Some(secret)) =
        (&handles.client_secret_handle, &config.client.client_secret)
    {
        credentials.register_static(
            secret_handle.as_str(),
            SecretString::from(secret.expose_secret().to_string()),
        );
    }
}

fn callback_base_from_redirect(redirect: &str) -> Option<EngineCallbackBase> {
    let prefix = redirect.strip_suffix("/callback")?;
    let (base, _vendor) = prefix.rsplit_once('/')?;
    EngineCallbackBase::new(base).ok()
}

fn compose_auth_engine(
    recipes: Arc<dyn AuthRecipeResolver>,
    client_credentials: CompositionClientCredentials,
    callback_base: Option<EngineCallbackBase>,
    secret_store: Arc<dyn SecretStorePort>,
    runtime_ports: ProductAuthProviderRuntimePorts,
) -> Result<OAuthProviderComposition, RebornBuildError> {
    let Some(callback_base) = callback_base else {
        tracing::debug!("no OAuth callback base configured; auth engine not composed");
        return Ok(OAuthProviderComposition {
            engine: None,
            client: None,
            gate_driver: None,
        });
    };
    let egress: Arc<dyn RuntimeHttpEgress> = Arc::new(ObligationStagedAuthEgress::new(
        runtime_ports.runtime_http_egress(),
        runtime_ports.obligation_handler(),
    ));
    let engine = Arc::new(AuthEngine::new(AuthEngineDeps {
        recipes,
        client_credentials: Arc::new(client_credentials),
        egress,
        secret_store: Arc::clone(&secret_store),
        callback_base,
        dcr_client_name: DCR_CLIENT_NAME.to_string(),
    }));
    let gate_driver = Arc::new(OAuthGateFlowDriver::new(
        Arc::clone(&engine),
        Arc::clone(&secret_store),
    ));
    tracing::debug!("product-auth auth engine composed");
    Ok(OAuthProviderComposition {
        client: Some(Arc::clone(&engine) as Arc<dyn AuthProviderClient>),
        engine: Some(engine),
        gate_driver: Some(gate_driver),
    })
}

/// Wraps the production egress so every engine vendor call runs with its
/// request-carried network policy staged as an invoke obligation.
struct ObligationStagedAuthEgress {
    inner: Arc<dyn RuntimeHttpEgress>,
    obligations: Arc<dyn CapabilityObligationHandler>,
}

impl ObligationStagedAuthEgress {
    fn new(
        inner: Arc<dyn RuntimeHttpEgress>,
        obligations: Arc<dyn CapabilityObligationHandler>,
    ) -> Self {
        Self { inner, obligations }
    }

    async fn stage(
        &self,
        request: &RuntimeHttpEgressRequest,
    ) -> Result<(), RuntimeHttpEgressError> {
        authorize_auth_egress(
            Arc::clone(&self.obligations),
            &request.scope,
            &request.capability_id,
            &request.network_policy,
        )
        .await
        .map_err(|_| RuntimeHttpEgressError::Request {
            reason: "auth egress network policy could not be staged".to_string(),
            request_bytes: 0,
            response_bytes: 0,
        })
    }

    async fn discard(&self, request: &RuntimeHttpEgressRequest) {
        discard_auth_egress_policy(
            Arc::clone(&self.obligations),
            &request.scope,
            &request.capability_id,
            &request.network_policy,
        )
        .await;
    }
}

impl fmt::Debug for ObligationStagedAuthEgress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ObligationStagedAuthEgress")
            .finish()
    }
}

#[async_trait::async_trait]
impl RuntimeHttpEgress for ObligationStagedAuthEgress {
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        self.stage(&request).await?;
        let result = self.inner.execute(request.clone()).await;
        self.discard(&request).await;
        result
    }

    async fn execute_credential_exchange(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        self.stage(&request).await?;
        let result = self
            .inner
            .execute_credential_exchange(request.clone())
            .await;
        // Success or failure, the staged policy must not outlive the call.
        self.discard(&request).await;
        result
    }
}

async fn authorize_auth_egress(
    handler: Arc<dyn CapabilityObligationHandler>,
    scope: &ResourceScope,
    capability_id: &ironclaw_host_api::ids::CapabilityId,
    policy: &NetworkPolicy,
) -> Result<(), AuthProductError> {
    let context = auth_execution_context(scope.clone())?;
    let estimate = ResourceEstimate {
        network_egress_bytes: policy.max_egress_bytes,
        ..ResourceEstimate::default()
    };
    handler
        .satisfy(CapabilityObligationRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id,
            estimate: &estimate,
            obligations: &[Obligation::ApplyNetworkPolicy {
                policy: policy.clone(),
            }],
        })
        .await
        .map_err(|error| {
            tracing::warn!(
                target: "ironclaw::reborn::oauth",
                obligation_error = ?error,
                "auth egress network policy could not be staged"
            );
            AuthProductError::BackendUnavailable
        })
}

async fn discard_auth_egress_policy(
    handler: Arc<dyn CapabilityObligationHandler>,
    scope: &ResourceScope,
    capability_id: &ironclaw_host_api::ids::CapabilityId,
    policy: &NetworkPolicy,
) {
    let context = match auth_execution_context(scope.clone()) {
        Ok(context) => context,
        Err(error) => {
            tracing::warn!(
                target: "ironclaw::reborn::oauth",
                ?error,
                "skipped auth egress-policy discard: execution context unavailable"
            );
            return;
        }
    };
    let estimate = ResourceEstimate {
        network_egress_bytes: policy.max_egress_bytes,
        ..ResourceEstimate::default()
    };
    if let Err(error) = handler
        .abort(CapabilityObligationAbortRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id,
            estimate: &estimate,
            obligations: &[Obligation::ApplyNetworkPolicy {
                policy: policy.clone(),
            }],
            outcome: &CapabilityObligationOutcome::default(),
        })
        .await
    {
        tracing::warn!(
            obligation_error = ?error,
            "failed to discard staged auth egress policy after vendor call"
        );
    }
}

fn auth_execution_context(
    resource_scope: ResourceScope,
) -> Result<ironclaw_host_api::scope::ExecutionContext, AuthProductError> {
    let context = ironclaw_host_api::scope::ExecutionContext {
        run_id: None,
        invocation_id: resource_scope.invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: resource_scope.tenant_id.clone(),
        user_id: resource_scope.user_id.clone(),
        authenticated_actor_user_id: None,
        agent_id: resource_scope.agent_id.clone(),
        project_id: resource_scope.project_id.clone(),
        mission_id: resource_scope.mission_id.clone(),
        thread_id: resource_scope.thread_id.clone(),
        origin: None,
        extension_id: ExtensionId::new("ironclaw_auth").map_err(|error| {
            tracing::warn!(%error, "auth execution-context extension id invalid");
            AuthProductError::BackendUnavailable
        })?,
        runtime: RuntimeKind::System,
        trust: TrustClass::System,
        grants: CapabilitySet::default(),
        mounts: MountView::default(),
        resource_scope,
    };
    context.validate().map_err(|error| {
        tracing::warn!(%error, "auth execution-context validation failed");
        AuthProductError::InvalidRequest {
            reason: "auth execution context validation failed".to_string(),
        }
    })?;
    Ok(context)
}

#[derive(Clone)]
pub(super) struct ProductAuthRuntimeCredentialResolver {
    accounts: Arc<dyn RuntimeCredentialAccountSelectionService>,
    refresher: Arc<dyn RuntimeCredentialAccountRefreshService>,
}

impl ProductAuthRuntimeCredentialResolver {
    pub(super) fn new_with_refresh(
        accounts: Arc<dyn RuntimeCredentialAccountSelectionService>,
        refresher: Arc<dyn RuntimeCredentialAccountRefreshService>,
    ) -> Self {
        Self {
            accounts,
            refresher,
        }
    }
}

impl fmt::Debug for ProductAuthRuntimeCredentialResolver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProductAuthRuntimeCredentialResolver")
            .field("accounts", &"<credential_account_service>")
            .finish()
    }
}

#[async_trait::async_trait]
impl RuntimeCredentialAccountResolver for ProductAuthRuntimeCredentialResolver {
    async fn resolve_access_secret(
        &self,
        request: RuntimeCredentialAccountRequest<'_>,
    ) -> Result<RuntimeCredentialAccessSecret, CredentialStageError> {
        let selection_request = runtime_credential_account_selection_request(
            request.scope,
            request.provider,
            request.setup.clone(),
            request.provider_scopes,
            request.requester_extension,
        )?;
        let account = self
            .accounts
            .select_unique_configured_runtime_account(selection_request.clone())
            .await
            .map_err(|error| {
                tracing::debug!(
                    provider = %request.provider,
                    requester_extension = %request.requester_extension,
                    auth_error = ?error,
                    "runtime product-auth account selection failed"
                );
                map_account_error(error)
            })?;
        tracing::debug!(
            provider = %request.provider,
            requester_extension = %request.requester_extension,
            has_access_secret = account.access_secret.is_some(),
            has_refresh_secret = account.refresh_secret.is_some(),
            status = ?account.status,
            "runtime product-auth account selected"
        );
        let account = self
            .refresher
            .refresh_configured_runtime_account(selection_request, account, self.accounts.as_ref())
            .await
            .map_err(|error| {
                tracing::debug!(
                    provider = %request.provider,
                    requester_extension = %request.requester_extension,
                    auth_error = ?error,
                    "runtime product-auth account refresh failed"
                );
                map_account_error(error)
            })?;
        tracing::debug!(
            provider = %request.provider,
            requester_extension = %request.requester_extension,
            has_access_secret = account.access_secret.is_some(),
            has_refresh_secret = account.refresh_secret.is_some(),
            status = ?account.status,
            "runtime product-auth account refresh resolved"
        );
        if account.status != CredentialAccountStatus::Configured {
            return Err(CredentialStageError::AuthRequired);
        }
        let handle = account.access_secret.ok_or(CredentialStageError::Backend)?;
        Ok(RuntimeCredentialAccessSecret {
            scope: account.scope.resource,
            handle,
        })
    }
}

pub(crate) fn auth_continuation_dispatcher(
    turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator>,
    blocked_auth_gate_source: Option<
        Arc<dyn ironclaw_processes::ProcessGateQuerySource<Error = ironclaw_turns::TurnError>>,
    >,
) -> Arc<dyn RebornAuthContinuationDispatcher> {
    let single_run: Arc<dyn RebornAuthContinuationDispatcher> = Arc::new(
        ProductAuthTurnGateResumeDispatcher::new(Arc::clone(&turn_coordinator)),
    );
    match blocked_auth_gate_source {
        // Local paths fan a completed flow out to the caller's other
        // provider-blocked runs (pair/authorize once, all waiting chats
        // continue). Production-shaped builders pass None until their
        // turn-state snapshot source is wired.
        Some(gate_source) => Arc::new(crate::blocked_auth_resume::BlockedAuthResumeFanout::new(
            single_run,
            gate_source,
            turn_coordinator,
        )),
        None => single_run,
    }
}

pub(super) struct ProductAuthServicesCompositionInput {
    pub(super) ports: RebornProductAuthServicePorts,
    pub(super) turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator>,
    pub(super) blocked_auth_snapshot_source: Option<
        Arc<dyn ironclaw_processes::ProcessGateQuerySource<Error = ironclaw_turns::TurnError>>,
    >,
    pub(super) provider_composition: OAuthProviderComposition,
    pub(super) security_audit_sink: Option<Arc<dyn ironclaw_events::SecurityAuditSink>>,
    pub(super) secret_store: Arc<dyn SecretStorePort>,
    pub(super) nearai_mcp_host_managed_scope: Option<AuthProductScope>,
    pub(super) credential_account_visibility_policy:
        Option<Arc<dyn ironclaw_auth::RuntimeCredentialAccountVisibilityPolicy>>,
    /// Durable auth-flow records for composition-owned product auth.
    pub(super) flow_record_source: Option<Arc<dyn ironclaw_auth::AuthFlowRecordSource>>,
}

pub(super) fn compose_product_auth_services(
    input: ProductAuthServicesCompositionInput,
) -> Result<
    (
        RebornProductAuthServices,
        Arc<dyn RebornAuthContinuationDispatcher>,
    ),
    RebornBuildError,
> {
    let ProductAuthServicesCompositionInput {
        ports,
        turn_coordinator,
        blocked_auth_snapshot_source,
        provider_composition,
        security_audit_sink,
        secret_store,
        nearai_mcp_host_managed_scope,
        credential_account_visibility_policy,
        flow_record_source,
    } = input;
    let builder_owned_durable_auth = flow_record_source.is_some();
    let ports = match provider_composition.client {
        Some(provider_client) => ports.with_provider_client(provider_client),
        None if builder_owned_durable_auth => ports.with_current_provider_client(),
        None => ports,
    };
    let base_continuation =
        auth_continuation_dispatcher(turn_coordinator, blocked_auth_snapshot_source);
    let mut services = ports.into_services(Arc::clone(&base_continuation), secret_store);
    if let Some(sink) = security_audit_sink {
        services = services.with_security_audit_sink(sink);
    }
    if let Some(policy) = credential_account_visibility_policy {
        services = services.with_credential_account_visibility_policy(policy);
    }
    if let Some(engine) = provider_composition.engine {
        services = services.with_auth_engine(engine);
    }
    if let Some(driver) = provider_composition.gate_driver {
        services = services.with_oauth_gate_driver(driver);
    }
    if let Some(scope) = nearai_mcp_host_managed_scope {
        services = services
            .with_host_managed_nearai_credential_scope(scope)
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("host-managed NEAR AI credential scope is invalid: {error}"),
            })?;
    }
    if let Some(source) = flow_record_source {
        services = services.with_flow_record_source(source);
    }
    Ok((services, base_continuation))
}
