use std::time::Instant;

use ironclaw_product_adapters::{
    AdapterInstallationId, AuthRequirement, DeclaredEgressHost, DeclaredEgressTarget,
    EgressCredentialHandle, EgressHeader, EgressMethod, EgressPath, EgressRequest,
    ParsedProductInbound, ProductAdapterCapabilities, ProductAdapterId, ProtocolAuthEvidence,
};

/// Maximum size of a component-returned JSON document the host will
/// deserialize. Bounded above the largest manifest/parsed-inbound payload we
/// expect in practice while staying well under the default WASM memory cap so
/// that operators raising `memory_bytes` for richer adapters do not silently
/// expand the host-side serde allocation in lockstep.
///
/// The WASM memory cap is the upper bound on what the component can return;
/// this constant is the *host*'s own ceiling before any serde parsing happens,
/// so we fail fast with `InvalidJson { message: "... exceeds N bytes" }`
/// instead of letting a multi-megabyte serde graph land in host memory.
pub(crate) const MAX_COMPONENT_JSON_BYTES: usize = 1024 * 1024;
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
    #[error("WASM component artifact is {actual} bytes; limit is {limit} bytes")]
    ComponentTooLarge { actual: usize, limit: usize },
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

/// Outcome of a successful `render-outbound` call.
///
/// `egress_request` is the host's validated, typed `EgressRequest` — the host
/// reconstructs it from the component-supplied JSON shim using the *same*
/// constructors (`EgressMethod::new`, `EgressPath::new`, `EgressHeader::new`,
/// `EgressRequest::with_body`) that the production HTTP egress path will
/// use. Callers MUST use this typed value when actually sending the request;
/// the raw component JSON is intentionally not exposed so that the only path
/// from a WASM component to the network goes through the host-side
/// validators.
#[derive(Debug, Clone)]
pub struct RenderOutboundResult {
    pub egress_request: EgressRequest,
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
        if wasm_bytes.len() > self.config.max_component_bytes {
            return Err(RuntimeError::ComponentTooLarge {
                actual: wasm_bytes.len(),
                limit: self.config.max_component_bytes,
            });
        }
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
        ensure_parsed_inbound_json(&response.parsed_json)?;
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
        let egress_request =
            validate_rendered_egress_request(prepared, &response.egress_request_json)?;
        Ok(RenderOutboundResult {
            egress_request,
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
        let manifest = adapter.call_manifest(&mut store);
        // Check the timeout BEFORE inspecting the call result so an
        // epoch-trap surfaces as a clean "deadline exceeded" error rather
        // than the raw wasmtime trap text — matches `parse_inbound` and
        // `render_outbound` ordering.
        ensure_execution_not_timed_out(&store, started)?;
        let manifest = manifest.map_err(|error| execution_failed(error.to_string(), &store))?;
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
    auth_requirement
        .validate_metadata()
        .map_err(|error| RuntimeError::InvalidManifest(error.to_string()))?;
    Ok(auth_requirement)
}

fn required_field(name: &'static str, value: Option<String>) -> Result<String, RuntimeError> {
    value.ok_or_else(|| RuntimeError::InvalidManifest(format!("missing {name}")))
}

fn ensure_capabilities_json(json: &str) -> Result<(), RuntimeError> {
    let field = "adapter-manifest.capabilities-json";
    ensure_json_within_host_budget(field, json)?;
    serde_json::from_str::<ProductAdapterCapabilities>(json)
        .map(|_| ())
        .map_err(|error| RuntimeError::InvalidJson {
            field,
            message: error.to_string(),
        })
}

fn ensure_json(field: &'static str, json: &str) -> Result<(), RuntimeError> {
    ensure_json_within_host_budget(field, json)?;
    serde_json::from_str::<Value>(json)
        .map(|_| ())
        .map_err(|error| RuntimeError::InvalidJson {
            field,
            message: error.to_string(),
        })
}

fn ensure_parsed_inbound_json(json: &str) -> Result<(), RuntimeError> {
    let field = "parsed-inbound.parsed-json";
    ensure_json_within_host_budget(field, json)?;
    serde_json::from_str::<ParsedProductInbound>(json)
        .map(|_| ())
        .map_err(|error| RuntimeError::InvalidJson {
            field,
            message: error.to_string(),
        })
}

/// Reject component-returned JSON above the host's own ceiling before letting
/// serde walk it. The WASM memory cap is the upper bound on what a component
/// can produce; this is the host's own ceiling so a future operator who
/// raises the WASM memory limit does not silently raise the host-side serde
/// allocation in lockstep.
fn ensure_json_within_host_budget(field: &'static str, json: &str) -> Result<(), RuntimeError> {
    if json.len() > MAX_COMPONENT_JSON_BYTES {
        return Err(RuntimeError::InvalidJson {
            field,
            message: format!(
                "component-returned JSON is {} bytes; host limit is {} bytes",
                json.len(),
                MAX_COMPONENT_JSON_BYTES,
            ),
        });
    }
    Ok(())
}

fn validate_rendered_egress_request(
    prepared: &PreparedProductAdapterComponent,
    json: &str,
) -> Result<EgressRequest, RuntimeError> {
    let field = "outbound-render.egress-request-json";
    ensure_json_within_host_budget(field, json)?;
    let value = serde_json::from_str::<Value>(json).map_err(|error| RuntimeError::InvalidJson {
        field,
        message: error.to_string(),
    })?;
    let object = value.as_object().ok_or_else(|| RuntimeError::InvalidJson {
        field,
        message: "must be a JSON object".to_string(),
    })?;
    let index = required_u64_field(object, field, "egress_target_index")?;
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
    // Defense-in-depth: this check is structurally a no-op today (the policy
    // is built from the same `declared_egress_targets` slice we just indexed
    // into, so the pair is always present) but the symmetry locks the
    // invariant for future readers and any future divergence between
    // `manifest` and `egress_policy`. Removing it would also remove the
    // single proof in this file that the pair authorized for egress is
    // exactly the pair declared in the manifest.
    prepared
        .egress_policy
        .check(EgressPolicyTarget {
            host: &target.host,
            credential_handle: target.credential_handle.as_ref(),
        })
        .map_err(|error| RuntimeError::InvalidJson {
            field,
            message: error.to_string(),
        })?;

    let method = EgressMethod::new(required_string_field(object, field, "method")?)
        .map_err(|error| invalid_json(field, error.to_string()))?;
    let path = EgressPath::new(required_string_field(object, field, "path")?)
        .map_err(|error| invalid_json(field, error.to_string()))?;
    let mut request = EgressRequest::new(target.host.clone(), method, path)
        .with_credential_handle(target.credential_handle.clone());
    for header in required_array_field(object, field, "headers")? {
        let header = header
            .as_object()
            .ok_or_else(|| RuntimeError::InvalidJson {
                field,
                message: "headers entries must be JSON objects".to_string(),
            })?;
        let name = required_string_field(header, field, "name")?;
        let value = required_string_field(header, field, "value")?;
        request = request.with_header(
            EgressHeader::new(name, value)
                .map_err(|error| invalid_json(field, error.to_string()))?,
        );
    }
    let body = required_array_field(object, field, "body")?
        .iter()
        .map(|byte| {
            let byte = byte.as_u64().ok_or_else(|| RuntimeError::InvalidJson {
                field,
                message: "body entries must be bytes".to_string(),
            })?;
            u8::try_from(byte).map_err(|_| RuntimeError::InvalidJson {
                field,
                message: "body entries must be bytes".to_string(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(request.with_body(body))
}

fn required_u64_field(
    object: &serde_json::Map<String, Value>,
    field: &'static str,
    name: &'static str,
) -> Result<u64, RuntimeError> {
    object
        .get(name)
        .and_then(Value::as_u64)
        .ok_or_else(|| RuntimeError::InvalidJson {
            field,
            message: format!("must include numeric {name}"),
        })
}

fn required_string_field(
    object: &serde_json::Map<String, Value>,
    field: &'static str,
    name: &'static str,
) -> Result<String, RuntimeError> {
    object
        .get(name)
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| RuntimeError::InvalidJson {
            field,
            message: format!("must include string {name}"),
        })
}

fn required_array_field<'a>(
    object: &'a serde_json::Map<String, Value>,
    field: &'static str,
    name: &'static str,
) -> Result<&'a Vec<Value>, RuntimeError> {
    object
        .get(name)
        .and_then(Value::as_array)
        .ok_or_else(|| RuntimeError::InvalidJson {
            field,
            message: format!("must include array {name}"),
        })
}

fn invalid_json(field: &'static str, message: String) -> RuntimeError {
    RuntimeError::InvalidJson { field, message }
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
    let message = if matches!(store.get_fuel(), Ok(0)) {
        format!("WASM ProductAdapter execution fuel exhausted: {message}")
    } else {
        message
    };

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

#[cfg(test)]
mod tests {
    use super::{
        PRODUCT_ADAPTER_WIT_VERSION, RuntimeError, auth_requirement_from_wit,
        classify_instantiation_error, product_adapter,
    };

    #[test]
    fn instantiation_error_classifier_pins_current_wit_import_diagnostic() {
        let err = classify_instantiation_error(
            "unknown import near:product-adapter/product-adapter-host@0.1.0".to_string(),
        );

        assert!(
            matches!(err, RuntimeError::InstantiationFailed(ref message)
                if message.contains("different WIT version")
                    && message.contains(PRODUCT_ADAPTER_WIT_VERSION)),
            "{err:?}"
        );
    }

    #[test]
    fn wit_auth_requirement_rejects_invalid_header_names() {
        let err = auth_requirement_from_wit(product_adapter::AuthRequirement {
            kind: product_adapter::AuthRequirementKind::SharedSecretHeader,
            header_name: Some("X-Foo\r\nInjected".to_string()),
            timestamp_header_name: None,
            cookie_name: None,
        })
        .expect_err("invalid header");

        assert!(
            matches!(err, RuntimeError::InvalidManifest(ref message)
                if message.contains("auth.header_name")),
            "{err:?}"
        );
    }

    #[test]
    fn wit_auth_requirement_rejects_invalid_timestamp_header_names() {
        let err = auth_requirement_from_wit(product_adapter::AuthRequirement {
            kind: product_adapter::AuthRequirementKind::RequestSignature,
            header_name: Some("X-Signature".to_string()),
            timestamp_header_name: Some("X-Time;stamp".to_string()),
            cookie_name: None,
        })
        .expect_err("invalid timestamp header");

        assert!(
            matches!(err, RuntimeError::InvalidManifest(ref message)
                if message.contains("auth.timestamp_header_name")),
            "{err:?}"
        );
    }

    #[test]
    fn wit_auth_requirement_rejects_invalid_cookie_names() {
        let err = auth_requirement_from_wit(product_adapter::AuthRequirement {
            kind: product_adapter::AuthRequirementKind::SessionCookie,
            header_name: None,
            timestamp_header_name: None,
            cookie_name: Some("session=id".to_string()),
        })
        .expect_err("invalid cookie name");

        assert!(
            matches!(err, RuntimeError::InvalidManifest(ref message)
                if message.contains("auth.name")),
            "{err:?}"
        );
    }
}
