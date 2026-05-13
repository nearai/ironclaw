use std::time::Instant;

use ironclaw_product_adapters::{
    AdapterInstallationId, AuthRequirement, DeclaredEgressHost, DeclaredEgressTarget,
    EgressCredentialHandle, ProductAdapterCapabilities, ProductAdapterId, ProtocolAuthEvidence,
};
use ironclaw_wasm_sandbox_core::{
    SandboxError, add_minimal_wasi_to_linker, component_engine,
    configure_store as configure_sandbox_store, elapsed_millis,
};
use serde_json::Value;
use wasmtime::component::Linker;
use wasmtime::{Engine, Store};

use crate::bindings;
use crate::bindings::exports::near::product_adapter::product_adapter;
use crate::config::{
    PRODUCT_ADAPTER_WIT_VERSION, ProductAdapterComponentLimits,
    ProductAdapterComponentRuntimeConfig,
};
use crate::egress_policy::{EgressPolicy, EgressPolicyTarget};
use crate::store::{ComponentLogRecord, StoreData};

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("failed to create WASM engine: {0}")]
    EngineCreationFailed(String),
    #[error("failed to compile WASM component: {0}")]
    CompilationFailed(String),
    #[error("failed to configure WASM store: {0}")]
    StoreConfiguration(String),
    #[error("failed to configure WASM linker: {0}")]
    LinkerConfiguration(String),
    #[error("failed to instantiate WASM component: {0}")]
    InstantiationFailed(String),
    #[error("ProductAdapter component execution failed: {message}")]
    ExecutionFailed {
        message: String,
        logs: Vec<ComponentLogRecord>,
    },
    #[error("ProductAdapter component returned invalid manifest: {0}")]
    InvalidManifest(String),
    #[error("ProductAdapter component returned invalid JSON in {field}: {message}")]
    InvalidJson {
        field: &'static str,
        message: String,
    },
}

impl From<SandboxError> for RuntimeError {
    fn from(error: SandboxError) -> Self {
        match error {
            SandboxError::EngineCreationFailed(message) => Self::EngineCreationFailed(message),
            SandboxError::StoreConfiguration(message) => Self::StoreConfiguration(message),
            SandboxError::LinkerConfiguration(message) => Self::LinkerConfiguration(message),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentManifest {
    pub adapter_id: ProductAdapterId,
    pub installation_id: AdapterInstallationId,
    pub capabilities_json: String,
    pub declared_egress_targets: Vec<DeclaredEgressTarget>,
    pub declared_auth_requirements: Vec<AuthRequirement>,
}

pub struct PreparedProductAdapterComponent {
    name: String,
    component: wasmtime::component::Component,
    limits: ProductAdapterComponentLimits,
    manifest: ComponentManifest,
    egress_policy: EgressPolicy,
}

impl std::fmt::Debug for PreparedProductAdapterComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedProductAdapterComponent")
            .field("name", &self.name)
            .field("limits", &self.limits)
            .field("manifest", &self.manifest)
            .field("egress_policy", &self.egress_policy)
            .finish_non_exhaustive()
    }
}

impl PreparedProductAdapterComponent {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn manifest(&self) -> &ComponentManifest {
        &self.manifest
    }

    pub fn egress_policy(&self) -> &EgressPolicy {
        &self.egress_policy
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedInboundResult {
    pub parsed_json: String,
    pub logs: Vec<ComponentLogRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderOutboundResult {
    pub egress_request_json: String,
    pub logs: Vec<ComponentLogRecord>,
}

pub struct ProductAdapterComponentRuntime {
    engine: Engine,
    config: ProductAdapterComponentRuntimeConfig,
}

impl ProductAdapterComponentRuntime {
    pub fn new(config: ProductAdapterComponentRuntimeConfig) -> Result<Self, RuntimeError> {
        let engine = component_engine("reborn-product-adapter-wasm-epoch-ticker")?;

        Ok(Self { engine, config })
    }

    pub fn config(&self) -> &ProductAdapterComponentRuntimeConfig {
        &self.config
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn prepare(
        &self,
        name: &str,
        wasm_bytes: &[u8],
    ) -> Result<PreparedProductAdapterComponent, RuntimeError> {
        let component = wasmtime::component::Component::new(&self.engine, wasm_bytes)
            .map_err(|error| RuntimeError::CompilationFailed(error.to_string()))?;
        let limits = self.config.default_limits.clone();
        let manifest = self.extract_manifest(&component, &limits)?;
        let egress_policy = EgressPolicy::new(manifest.declared_egress_targets.clone());

        Ok(PreparedProductAdapterComponent {
            name: name.to_string(),
            component,
            limits,
            manifest,
            egress_policy,
        })
    }

    pub fn parse_inbound(
        &self,
        prepared: &PreparedProductAdapterComponent,
        raw_payload: &[u8],
        evidence: &ProtocolAuthEvidence,
    ) -> Result<ParsedInboundResult, RuntimeError> {
        let started = Instant::now();
        let evidence_json =
            serde_json::to_string(evidence).map_err(|error| RuntimeError::InvalidJson {
                field: "auth-evidence.evidence-json",
                message: error.to_string(),
            })?;
        let (mut store, instance) = self.instantiate(&prepared.component, &prepared.limits)?;
        let adapter = instance.near_product_adapter_product_adapter();
        let evidence = product_adapter::AuthEvidence { evidence_json };
        let response = adapter.call_parse_inbound(&mut store, raw_payload, &evidence);
        ensure_execution_not_timed_out(&store, started)?;
        let response = match response {
            Ok(Ok(response)) => response,
            Ok(Err(message)) => return Err(execution_failed(message, &store)),
            Err(error) => return Err(execution_failed(error.to_string(), &store)),
        };
        ensure_json("parsed-inbound.parsed-json", &response.parsed_json)?;
        Ok(ParsedInboundResult {
            parsed_json: response.parsed_json,
            logs: store.data().logs.clone(),
        })
    }

    pub fn render_outbound(
        &self,
        prepared: &PreparedProductAdapterComponent,
        outbound_json: &str,
    ) -> Result<RenderOutboundResult, RuntimeError> {
        let started = Instant::now();
        ensure_json("outbound-envelope.outbound-json", outbound_json)?;
        let (mut store, instance) = self.instantiate(&prepared.component, &prepared.limits)?;
        let adapter = instance.near_product_adapter_product_adapter();
        let envelope = product_adapter::OutboundEnvelope {
            outbound_json: outbound_json.to_string(),
        };
        let response = adapter.call_render_outbound(&mut store, &envelope);
        ensure_execution_not_timed_out(&store, started)?;
        let response = match response {
            Ok(Ok(response)) => response,
            Ok(Err(message)) => return Err(execution_failed(message, &store)),
            Err(error) => return Err(execution_failed(error.to_string(), &store)),
        };
        validate_rendered_egress_request(prepared, &response.egress_request_json)?;
        Ok(RenderOutboundResult {
            egress_request_json: response.egress_request_json,
            logs: store.data().logs.clone(),
        })
    }

    fn extract_manifest(
        &self,
        component: &wasmtime::component::Component,
        limits: &ProductAdapterComponentLimits,
    ) -> Result<ComponentManifest, RuntimeError> {
        let started = Instant::now();
        let (mut store, instance) = self.instantiate(component, limits)?;
        let adapter = instance.near_product_adapter_product_adapter();
        let manifest = adapter
            .call_manifest(&mut store)
            .map_err(|error| execution_failed(error.to_string(), &store))?;
        ensure_execution_not_timed_out(&store, started)?;
        component_manifest_from_wit(manifest)
    }

    fn instantiate(
        &self,
        component: &wasmtime::component::Component,
        limits: &ProductAdapterComponentLimits,
    ) -> Result<(Store<StoreData>, bindings::ProductAdapterComponent), RuntimeError> {
        let mut store = Store::new(
            &self.engine,
            StoreData::new(limits.memory_bytes, limits.timeout),
        );
        configure_store(&mut store, limits)?;
        let linker = create_linker(&self.engine)?;
        let instance =
            bindings::ProductAdapterComponent::instantiate(&mut store, component, &linker)
                .map_err(|error| classify_instantiation_error(error.to_string()))?;
        Ok((store, instance))
    }
}

impl std::fmt::Debug for ProductAdapterComponentRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProductAdapterComponentRuntime")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

fn component_manifest_from_wit(
    manifest: product_adapter::AdapterManifest,
) -> Result<ComponentManifest, RuntimeError> {
    ensure_capabilities_json(&manifest.capabilities_json)?;
    let adapter_id = ProductAdapterId::new(manifest.adapter_id)
        .map_err(|error| RuntimeError::InvalidManifest(error.to_string()))?;
    let installation_id = AdapterInstallationId::new(manifest.installation_id)
        .map_err(|error| RuntimeError::InvalidManifest(error.to_string()))?;
    let declared_egress_targets = manifest
        .declared_egress_targets
        .into_iter()
        .map(declared_egress_target_from_wit)
        .collect::<Result<Vec<_>, _>>()?;
    let declared_auth_requirements = manifest
        .declared_auth_requirements
        .into_iter()
        .map(auth_requirement_from_wit)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(ComponentManifest {
        adapter_id,
        installation_id,
        capabilities_json: manifest.capabilities_json,
        declared_egress_targets,
        declared_auth_requirements,
    })
}

fn declared_egress_target_from_wit(
    target: product_adapter::DeclaredEgressTarget,
) -> Result<DeclaredEgressTarget, RuntimeError> {
    let host = DeclaredEgressHost::new(target.host)
        .map_err(|error| RuntimeError::InvalidManifest(error.to_string()))?;
    let credential_handle = match target.credential_handle {
        Some(handle) => Some(
            EgressCredentialHandle::new(handle)
                .map_err(|error| RuntimeError::InvalidManifest(error.to_string()))?,
        ),
        None => None,
    };
    Ok(DeclaredEgressTarget::new(host, credential_handle))
}

fn auth_requirement_from_wit(
    requirement: product_adapter::AuthRequirement,
) -> Result<AuthRequirement, RuntimeError> {
    use product_adapter::AuthRequirementKind;

    let auth_requirement = match requirement.kind {
        AuthRequirementKind::RequestSignature => AuthRequirement::RequestSignature {
            header_name: required_field("header-name", requirement.header_name)?,
            timestamp_header_name: requirement.timestamp_header_name,
        },
        AuthRequirementKind::SharedSecretHeader => AuthRequirement::SharedSecretHeader {
            header_name: required_field("header-name", requirement.header_name)?,
        },
        AuthRequirementKind::SessionCookie => AuthRequirement::SessionCookie {
            name: required_field("cookie-name", requirement.cookie_name)?,
        },
        AuthRequirementKind::BearerToken => AuthRequirement::BearerToken,
    };
    Ok(auth_requirement)
}

fn required_field(name: &'static str, value: Option<String>) -> Result<String, RuntimeError> {
    value.ok_or_else(|| RuntimeError::InvalidManifest(format!("missing {name}")))
}

fn ensure_capabilities_json(json: &str) -> Result<(), RuntimeError> {
    serde_json::from_str::<ProductAdapterCapabilities>(json)
        .map(|_| ())
        .map_err(|error| RuntimeError::InvalidJson {
            field: "adapter-manifest.capabilities-json",
            message: error.to_string(),
        })
}

fn ensure_json(field: &'static str, json: &str) -> Result<(), RuntimeError> {
    serde_json::from_str::<Value>(json)
        .map(|_| ())
        .map_err(|error| RuntimeError::InvalidJson {
            field,
            message: error.to_string(),
        })
}

fn validate_rendered_egress_request(
    prepared: &PreparedProductAdapterComponent,
    json: &str,
) -> Result<(), RuntimeError> {
    let field = "outbound-render.egress-request-json";
    let value = serde_json::from_str::<Value>(json).map_err(|error| RuntimeError::InvalidJson {
        field,
        message: error.to_string(),
    })?;
    let object = value.as_object().ok_or_else(|| RuntimeError::InvalidJson {
        field,
        message: "must be a JSON object".to_string(),
    })?;
    let index = object
        .get("egress_target_index")
        .or_else(|| object.get("egress-target-index"))
        .and_then(Value::as_u64)
        .ok_or_else(|| RuntimeError::InvalidJson {
            field,
            message: "must include numeric egress_target_index".to_string(),
        })?;
    let index = usize::try_from(index).map_err(|_| RuntimeError::InvalidJson {
        field,
        message: "egress_target_index is too large".to_string(),
    })?;
    let target = prepared
        .manifest
        .declared_egress_targets
        .get(index)
        .ok_or_else(|| RuntimeError::InvalidJson {
            field,
            message: format!("egress_target_index {index} is not declared in adapter manifest"),
        })?;
    prepared
        .egress_policy
        .check(EgressPolicyTarget {
            host: &target.host,
            credential_handle: target.credential_handle.as_ref(),
        })
        .map_err(|error| RuntimeError::InvalidJson {
            field,
            message: error.to_string(),
        })
}

fn configure_store(
    store: &mut Store<StoreData>,
    limits: &ProductAdapterComponentLimits,
) -> Result<(), RuntimeError> {
    configure_sandbox_store(store, limits)?;
    Ok(())
}

fn create_linker(engine: &Engine) -> Result<Linker<StoreData>, RuntimeError> {
    let mut linker = Linker::new(engine);
    add_minimal_wasi_to_linker(&mut linker)?;
    bindings::ProductAdapterComponent::add_to_linker::<_, wasmtime::component::HasSelf<_>>(
        &mut linker,
        |state: &mut StoreData| state,
    )
    .map_err(|error| RuntimeError::LinkerConfiguration(error.to_string()))?;
    Ok(linker)
}

fn ensure_execution_not_timed_out(
    store: &Store<StoreData>,
    started: Instant,
) -> Result<(), RuntimeError> {
    if store.data().deadline_exceeded() {
        return Err(execution_failed(
            format!(
                "WASM ProductAdapter execution deadline exceeded after {}ms",
                elapsed_millis(started)
            ),
            store,
        ));
    }
    Ok(())
}

fn execution_failed(message: String, store: &Store<StoreData>) -> RuntimeError {
    RuntimeError::ExecutionFailed {
        message,
        logs: store.data().logs.clone(),
    }
}

fn classify_instantiation_error(message: String) -> RuntimeError {
    if message.contains("near:product-adapter") || message.contains("import") {
        RuntimeError::InstantiationFailed(format!(
            "{message}. This usually means the component was compiled against a different WIT version than the host supports (host: {PRODUCT_ADAPTER_WIT_VERSION})."
        ))
    } else {
        RuntimeError::InstantiationFailed(message)
    }
}
