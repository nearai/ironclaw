//! WASM tool wrapper implementing the Tool trait.
//!
//! Uses wasmtime::component::bindgen! to generate typed bindings from the WIT
//! interface, ensuring all host functions are properly registered under the
//! correct `near:agent/host` namespace.
//!
//! Each execution creates a fresh instance (NEAR pattern) to ensure
//! isolation and deterministic behavior.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use wasmtime::Store;
use wasmtime::component::Linker;
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::auth::resolve_secret_for_runtime;
use crate::context::JobContext;
use crate::db::UserStore;
use crate::llm::recording::{HttpExchangeRequest, HttpExchangeResponse, HttpInterceptor};
use crate::secrets::SecretsStore;
use crate::secrets::host_matches_pattern;
use crate::tools::tool::{Tool, ToolDiscoverySummary, ToolError, ToolOutput};
use crate::tools::wasm::capabilities::Capabilities;
use base64::Engine as _;
use borsh::BorshSerialize;
use ed25519_dalek::Signer as _;
use hmac::{Hmac, Mac};
use k256::ecdsa::{Signature as K256Signature, SigningKey, signature::hazmat::PrehashSigner};
use rand::RngCore;
use rmp_serde::Serializer as MsgpackSerializer;
use serde::Serialize as _;
use sha2::{Digest, Sha256};
use sha3::Keccak256;

use crate::tools::wasm::credential_injector::{InjectedCredentials, inject_credential};
use crate::tools::wasm::error::WasmError;
use crate::tools::wasm::host::{HostState, LogLevel};
use crate::tools::wasm::limits::{ResourceLimits, WasmResourceLimiter};
use crate::tools::wasm::runtime::{EPOCH_TICK_INTERVAL, PreparedModule, WasmToolRuntime};
use crate::tools::wasm::{ssrf_safe_client_builder_for_target, validate_and_resolve_http_target};
use ironclaw_safety::LeakDetector;

// Generate component model bindings from the WIT file.
//
// This creates:
// - `near::agent::host::Host` trait + `add_to_linker()` for the import interface
// - `SandboxedTool` struct with `instantiate()` for the world
// - `exports::near::agent::tool::*` types for the export interface
wasmtime::component::bindgen!({
    path: "wit/tool.wit",
    world: "sandboxed-tool",
    with: {},
});

// Alias the export interface types for convenience.
use exports::near::agent::tool as wit_tool;

/// Configuration needed to refresh an expired OAuth access token.
///
/// Extracted at tool load time from the capabilities file's `auth.oauth` section.
/// Passed into `resolve_host_credentials()` so it can transparently refresh
/// tokens before WASM execution.
#[derive(Debug, Clone)]
pub struct OAuthRefreshConfig {
    /// OAuth token exchange URL (e.g., "https://oauth2.googleapis.com/token").
    pub token_url: String,
    /// OAuth client_id.
    pub client_id: String,
    /// OAuth client_secret (optional, some providers use PKCE without a secret).
    pub client_secret: Option<String>,
    /// Hosted OAuth proxy base URL (e.g., "http://host.docker.internal:8080").
    pub exchange_proxy_url: Option<String>,
    /// OAuth proxy auth token for authenticating with the hosted OAuth proxy.
    /// Kept as `gateway_token` for public API compatibility.
    pub gateway_token: Option<String>,
    /// Secret name of the access token (e.g., "google_oauth_token").
    /// The refresh token lives at `{secret_name}_refresh_token`.
    pub secret_name: String,
    /// Provider hint stored alongside the refreshed secret.
    pub provider: Option<String>,
    /// Extra form parameters appended during refresh requests.
    pub extra_refresh_params: HashMap<String, String>,
}

impl OAuthRefreshConfig {
    pub fn oauth_proxy_auth_token(&self) -> Option<&str> {
        self.gateway_token.as_deref()
    }
}

/// Pre-resolved credential for host-based injection.
///
/// Built before each WASM execution by decrypting secrets from the store.
/// Applied per-request by matching the URL host against `host_patterns`.
/// WASM tools never see the raw secret values.
///
/// **No `derive(Debug)`.** This struct holds decrypted secret material —
/// header values, query-parameter values, and the raw `secret_value` are
/// all sensitive. The hand-rolled `Debug` impl below redacts every
/// secret-bearing field so an accidental `{:?}` in a future log line, a
/// panic message, or a `dbg!()` cannot leak credentials. Do NOT add
/// `#[derive(Debug)]` here without revisiting the redaction.
struct ResolvedHostCredential {
    /// Name of the source secret. Non-sensitive metadata used only for
    /// deterministic tie-breaks when two matching credentials share the
    /// same path specificity; never rendered to logs or tool output.
    secret_name: String,
    /// Host patterns this credential applies to (e.g., "www.googleapis.com").
    host_patterns: Vec<String>,
    /// Path prefixes this credential is scoped to. Empty means all paths.
    path_patterns: Vec<String>,
    /// Headers to add to matching requests (e.g., "Authorization: Bearer ...").
    headers: HashMap<String, String>,
    /// Query parameters to add to matching requests.
    query_params: HashMap<String, String>,
    /// Raw secret value for redaction in error messages.
    secret_value: String,
    /// When set, the secret feeds a per-request signer (HMAC, EIP-712,
    /// or NEP-413). The variant carries the venue-specific output
    /// schema declared by the tool in its capabilities file.
    signing: Option<SigningSpec>,
}

#[derive(Clone)]
enum SigningSpec {
    Hmac(HmacSigning),
    Eip712(Eip712Signing),
    Nep413(Nep413Signing),
}

#[derive(Clone)]
struct HmacSigning {
    signature_header: String,
    timestamp_header: String,
}

#[derive(Clone)]
struct Eip712Signing {
    domain: crate::secrets::Eip712Domain,
    primary_type: String,
    structs: Vec<crate::secrets::Eip712StructDef>,
    output_headers: Vec<crate::secrets::HeaderOutput>,
    output_body_fields: Vec<crate::secrets::BodyJsonOutput>,
}

#[derive(Clone)]
struct Nep413Signing {
    recipient_source: crate::secrets::FieldSource,
    message_source: crate::secrets::FieldSource,
    callback_url_source: Option<crate::secrets::FieldSource>,
    output_headers: Vec<crate::secrets::HeaderOutput>,
}

impl std::fmt::Debug for ResolvedHostCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Print the structural information that's useful for debugging
        // (which hosts the credential applies to, which header / query
        // names get injected) while redacting every value that could
        // contain decrypted secret material.
        let header_keys: Vec<&String> = self.headers.keys().collect();
        let query_keys: Vec<&String> = self.query_params.keys().collect();
        let signing_summary: Option<String> = self.signing.as_ref().map(|s| match s {
            SigningSpec::Hmac(spec) => format!(
                "hmac(sig={}, ts={})",
                spec.signature_header, spec.timestamp_header
            ),
            SigningSpec::Eip712(spec) => {
                let outs: Vec<&str> = spec
                    .output_headers
                    .iter()
                    .map(|o| o.header_name.as_str())
                    .collect();
                format!(
                    "eip712(domain={}, primary={}, outputs={:?})",
                    spec.domain.name, spec.primary_type, outs
                )
            }
            SigningSpec::Nep413(spec) => {
                let outs: Vec<&str> = spec
                    .output_headers
                    .iter()
                    .map(|o| o.header_name.as_str())
                    .collect();
                format!("nep413(outputs={:?})", outs)
            }
        });
        f.debug_struct("ResolvedHostCredential")
            .field("host_patterns", &self.host_patterns)
            .field("path_patterns", &self.path_patterns)
            .field("header_names", &header_keys)
            .field("query_param_names", &query_keys)
            .field("signing", &signing_summary)
            .field("secret_value", &"[REDACTED]")
            .finish()
    }
}

/// Store data for WASM tool execution.
///
/// Contains the resource limiter, host state, WASI context, and injected
/// credentials. Fresh instance created per execution (NEAR pattern).
struct StoreData {
    limiter: WasmResourceLimiter,
    host_state: HostState,
    wasi: WasiCtx,
    table: ResourceTable,
    /// Injected credentials for URL/header placeholder substitution.
    /// Keys are placeholder names like "TELEGRAM_BOT_TOKEN".
    credentials: HashMap<String, String>,
    /// Pre-resolved credentials for automatic host-based injection.
    /// Applied by matching URL host against each credential's host_patterns.
    host_credentials: Vec<ResolvedHostCredential>,
    /// Dedicated tokio runtime for HTTP requests, lazily initialized.
    /// Reused across multiple `http_request` calls within one execution.
    http_runtime: Option<tokio::runtime::Runtime>,
    /// Optional HTTP interceptor for testing — returns canned responses
    /// instead of making real requests when set.
    http_interceptor: Option<Arc<dyn HttpInterceptor>>,
}

impl StoreData {
    fn new(
        memory_limit: u64,
        capabilities: Capabilities,
        credentials: HashMap<String, String>,
        host_credentials: Vec<ResolvedHostCredential>,
    ) -> Self {
        // Minimal WASI context: no filesystem, no env vars (security)
        let wasi = WasiCtxBuilder::new().build();

        Self {
            limiter: WasmResourceLimiter::new(memory_limit),
            host_state: HostState::new(capabilities),
            wasi,
            table: ResourceTable::new(),
            credentials,
            host_credentials,
            http_runtime: None,
            http_interceptor: None,
        }
    }

    /// Inject credentials into a string by replacing placeholders.
    ///
    /// Replaces patterns like `{GOOGLE_ACCESS_TOKEN}` with actual values.
    /// WASM tools reference credentials by placeholder, never seeing real values.
    fn inject_credentials(&self, input: &str, context: &str) -> String {
        let mut result = input.to_string();

        for (name, value) in &self.credentials {
            let placeholder = format!("{{{}}}", name);
            if result.contains(&placeholder) {
                tracing::debug!(
                    placeholder = %placeholder,
                    context = %context,
                    "Replacing credential placeholder in tool request"
                );
                result = result.replace(&placeholder, value);
            }
        }

        result
    }

    /// Replace injected credential values with `[REDACTED]` in text.
    ///
    /// Prevents credentials from leaking through error messages or logs.
    /// reqwest::Error includes the full URL in its Display output, so any
    /// error from an injected-URL request will contain the raw credential
    /// unless we scrub it.
    fn redact_credentials(&self, text: &str) -> String {
        let mut result = text.to_string();
        for (name, value) in &self.credentials {
            if !value.is_empty() {
                result = result.replace(value, &format!("[REDACTED:{}]", name));
            }
        }
        for cred in &self.host_credentials {
            if !cred.secret_value.is_empty() {
                result = result.replace(&cred.secret_value, "[REDACTED:host_credential]");
            }
        }
        result
    }

    /// Inject pre-resolved host credentials into the request.
    ///
    /// Matches the URL host against each resolved credential's host_patterns
    /// and — when declared — path_patterns. Matching credentials are ordered
    /// by ascending path specificity (longest matching prefix last), with
    /// ties broken alphabetically on `secret_name`. Last-write-wins header
    /// merging then means the most-specific mapping wins any conflict,
    /// deterministically.
    fn inject_host_credentials(
        &self,
        url_host: &str,
        method: &str,
        body: &mut Option<Vec<u8>>,
        headers: &mut HashMap<String, String>,
        url: &mut String,
    ) {
        use crate::secrets::{
            extract_url_path_for_matching, match_specificity, path_matches_prefix,
        };

        let url_path = extract_url_path_for_matching(url);

        let mut matches_for_request: Vec<&ResolvedHostCredential> = self
            .host_credentials
            .iter()
            .filter(|cred| {
                cred.host_patterns
                    .iter()
                    .any(|pattern| host_matches_pattern(url_host, pattern))
                    && (cred.path_patterns.is_empty()
                        || cred
                            .path_patterns
                            .iter()
                            .any(|prefix| path_matches_prefix(&url_path, prefix)))
            })
            .collect();

        matches_for_request.sort_by(|a, b| {
            let spec_a = match_specificity(&a.path_patterns, &url_path);
            let spec_b = match_specificity(&b.path_patterns, &url_path);
            spec_a
                .cmp(&spec_b)
                .then_with(|| a.secret_name.cmp(&b.secret_name))
        });

        let timestamp_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut nonce_bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);

        for cred in matches_for_request {
            // Merge injected headers (host credentials take precedence; the
            // most-specific match iterates last, so it wins any conflict).
            for (key, value) in &cred.headers {
                headers.insert(key.clone(), value.clone());
            }

            // Append query parameters to URL (insert before fragment if present)
            if !cred.query_params.is_empty() {
                let (base, fragment) = match url.find('#') {
                    Some(i) => (url[..i].to_string(), Some(url[i..].to_string())),
                    None => (url.clone(), None),
                };
                *url = base;

                let separator = if url.contains('?') { '&' } else { '?' };
                for (i, (name, value)) in cred.query_params.iter().enumerate() {
                    if i == 0 {
                        url.push(separator);
                    } else {
                        url.push('&');
                    }
                    url.push_str(&urlencoding::encode(name));
                    url.push('=');
                    url.push_str(&urlencoding::encode(value));
                }

                if let Some(frag) = fragment {
                    url.push_str(&frag);
                }
            }

            if let Some(signing) = &cred.signing {
                let result = match signing {
                    SigningSpec::Hmac(spec) => apply_hmac_signing(
                        spec,
                        &cred.secret_value,
                        method,
                        &url_path,
                        body.as_deref(),
                        timestamp_secs,
                    ),
                    SigningSpec::Eip712(spec) => apply_eip712_signing(
                        spec,
                        &cred.secret_value,
                        body.as_deref(),
                        timestamp_secs,
                        &nonce_bytes,
                    ),
                    SigningSpec::Nep413(spec) => apply_nep413_signing(
                        spec,
                        &cred.secret_value,
                        body.as_deref(),
                        timestamp_secs,
                        &nonce_bytes,
                    ),
                };

                match result {
                    Ok(extra) => {
                        for (k, v) in extra.headers {
                            headers.insert(k, v);
                        }
                        if let Err(error) = apply_body_mutations(body, &extra.body_mutations) {
                            tracing::warn!(
                                secret_name = %cred.secret_name,
                                error = %error,
                                "body mutation failed; signing headers applied but body left unmodified"
                            );
                        }
                    }
                    Err(error) => tracing::warn!(
                        secret_name = %cred.secret_name,
                        error = %error,
                        "signer failed; skipping signing headers"
                    ),
                }
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum SignerError {
    #[error("invalid secret material: {0}")]
    InvalidSecret(String),
    #[error("invalid signing schema: {0}")]
    InvalidSchema(String),
    #[error("invalid eip712 field: {0}")]
    InvalidField(String),
    #[error("source not supported in this position: {0}")]
    UnsupportedSource(String),
    #[error("signing failed: {0}")]
    SignFailed(String),
}

#[derive(Debug, Default)]
struct SignerOutput {
    headers: Vec<(String, String)>,
    body_mutations: Vec<(String, serde_json::Value)>,
}

fn apply_hmac_signing(
    spec: &HmacSigning,
    secret_value: &str,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
    timestamp_secs: u64,
) -> Result<SignerOutput, SignerError> {
    let key_bytes = base64::engine::general_purpose::URL_SAFE
        .decode(secret_value.as_bytes())
        .map_err(|e| SignerError::InvalidSecret(format!("hmac key not url-safe base64: {e}")))?;
    let mut mac = Hmac::<Sha256>::new_from_slice(&key_bytes)
        .map_err(|e| SignerError::SignFailed(format!("hmac key rejected: {e}")))?;
    let timestamp = timestamp_secs.to_string();
    mac.update(timestamp.as_bytes());
    mac.update(method.as_bytes());
    mac.update(path.as_bytes());
    if let Some(body_bytes) = body {
        mac.update(body_bytes);
    }
    let signature = base64::engine::general_purpose::URL_SAFE.encode(mac.finalize().into_bytes());
    Ok(SignerOutput {
        headers: vec![
            (spec.signature_header.clone(), signature),
            (spec.timestamp_header.clone(), timestamp),
        ],
        body_mutations: Vec::new(),
    })
}

fn apply_eip712_signing(
    spec: &Eip712Signing,
    secret_value: &str,
    body: Option<&[u8]>,
    timestamp_secs: u64,
    nonce_bytes: &[u8; 32],
) -> Result<SignerOutput, SignerError> {
    let signing_key = parse_secp256k1_secret(secret_value)?;
    let address_bytes = derive_eth_address(&signing_key);
    let address_hex = format!("0x{}", hex::encode(address_bytes));
    let public_key_hex = derive_secp256k1_public_key_hex(&signing_key);

    if spec.structs.len() != 1 {
        return Err(SignerError::InvalidSchema(
            "only single-struct eip712 schemas are supported in this build".into(),
        ));
    }
    let primary = spec
        .structs
        .iter()
        .find(|s| s.name == spec.primary_type)
        .ok_or_else(|| {
            SignerError::InvalidSchema(format!(
                "primary type '{}' not found in declared structs",
                spec.primary_type
            ))
        })?;

    let mut field_value_buf: Vec<u8> = Vec::with_capacity(32 + primary.fields.len() * 32);
    let type_string = encode_eip712_type_string(primary);
    let type_hash = keccak256(type_string.as_bytes());
    field_value_buf.extend_from_slice(&type_hash);
    for field in &primary.fields {
        let raw = resolve_field_source(
            &field.source,
            Some(&address_hex),
            &public_key_hex,
            body,
            timestamp_secs,
            nonce_bytes,
        )?;
        let encoded = encode_eip712_field_value(&field.type_name, &raw)?;
        field_value_buf.extend_from_slice(&encoded);
    }
    let struct_hash = keccak256(&field_value_buf);

    let domain_separator = compute_eip712_domain_separator(&spec.domain)?;

    let mut final_buf = Vec::with_capacity(2 + 32 + 32);
    final_buf.extend_from_slice(&[0x19u8, 0x01u8]);
    final_buf.extend_from_slice(&domain_separator);
    final_buf.extend_from_slice(&struct_hash);
    let final_hash = keccak256(&final_buf);

    let (sig_64, recovery_id) = sign_secp256k1_recoverable(&signing_key, &final_hash)?;
    let mut sig_with_v = Vec::with_capacity(65);
    sig_with_v.extend_from_slice(&sig_64);
    let v = recovery_id + 27;
    sig_with_v.push(v);
    let signature_hex = format!("0x{}", hex::encode(&sig_with_v));
    let signature_b64 = base64::engine::general_purpose::URL_SAFE.encode(&sig_with_v);
    let signature_r_hex = format!("0x{}", hex::encode(&sig_64[..32]));
    let signature_s_hex = format!("0x{}", hex::encode(&sig_64[32..]));

    let evaluated = EvaluatedSignature {
        signer_address: address_hex.clone(),
        signer_public_key: public_key_hex.clone(),
        signature_hex: signature_hex.clone(),
        signature_b64: signature_b64.clone(),
        signature_r_hex,
        signature_s_hex,
        signature_v: v,
        timestamp_secs,
        nonce_bytes: *nonce_bytes,
    };

    let mut headers: Vec<(String, String)> = Vec::with_capacity(spec.output_headers.len());
    for header in &spec.output_headers {
        let value = resolve_output_source(
            &header.value,
            Some(&address_hex),
            &public_key_hex,
            &signature_hex,
            &signature_b64,
            timestamp_secs,
            nonce_bytes,
        )?;
        headers.push((header.header_name.clone(), value));
    }

    let mut body_mutations: Vec<(String, serde_json::Value)> =
        Vec::with_capacity(spec.output_body_fields.len());
    for field in &spec.output_body_fields {
        body_mutations.push((field.json_path.clone(), evaluate_body_value(&field.value, &evaluated)));
    }

    Ok(SignerOutput {
        headers,
        body_mutations,
    })
}

fn apply_nep413_signing(
    spec: &Nep413Signing,
    secret_value: &str,
    body: Option<&[u8]>,
    timestamp_secs: u64,
    nonce_bytes: &[u8; 32],
) -> Result<SignerOutput, SignerError> {
    let (signing_key, verifying_key) = parse_ed25519_secret(secret_value)?;
    let public_key_b58 = format!(
        "ed25519:{}",
        bs58::encode(verifying_key.as_bytes()).into_string()
    );

    let recipient = resolve_field_source(
        &spec.recipient_source,
        None,
        &public_key_b58,
        body,
        timestamp_secs,
        nonce_bytes,
    )?;
    let message = resolve_field_source(
        &spec.message_source,
        None,
        &public_key_b58,
        body,
        timestamp_secs,
        nonce_bytes,
    )?;
    let callback_url = match &spec.callback_url_source {
        Some(src) => Some(resolve_field_source(
            src,
            None,
            &public_key_b58,
            body,
            timestamp_secs,
            nonce_bytes,
        )?),
        None => None,
    };

    let payload = Nep413Payload {
        message,
        nonce: *nonce_bytes,
        recipient,
        callback_url,
    };

    let mut serialized: Vec<u8> = Vec::new();
    let prefix: u32 = 2_147_484_061;
    BorshSerialize::serialize(&prefix, &mut serialized)
        .map_err(|e| SignerError::SignFailed(format!("borsh prefix: {e}")))?;
    BorshSerialize::serialize(&payload, &mut serialized)
        .map_err(|e| SignerError::SignFailed(format!("borsh payload: {e}")))?;

    let hash = Sha256::digest(&serialized);
    let signature = signing_key.sign(&hash);
    let signature_bytes = signature.to_bytes();
    let signature_hex = format!("0x{}", hex::encode(signature_bytes));
    let signature_b64 = base64::engine::general_purpose::URL_SAFE.encode(signature_bytes);

    let mut headers: Vec<(String, String)> = Vec::with_capacity(spec.output_headers.len());
    for header in &spec.output_headers {
        let value = resolve_output_source(
            &header.value,
            None,
            &public_key_b58,
            &signature_hex,
            &signature_b64,
            timestamp_secs,
            nonce_bytes,
        )?;
        headers.push((header.header_name.clone(), value));
    }
    Ok(SignerOutput {
        headers,
        body_mutations: Vec::new(),
    })
}

#[derive(BorshSerialize)]
struct Nep413Payload {
    message: String,
    nonce: [u8; 32],
    recipient: String,
    callback_url: Option<String>,
}

fn parse_secp256k1_secret(secret: &str) -> Result<SigningKey, SignerError> {
    let trimmed = secret.trim().trim_start_matches("0x");
    let bytes = hex::decode(trimmed)
        .map_err(|e| SignerError::InvalidSecret(format!("expected hex secp256k1 key: {e}")))?;
    if bytes.len() != 32 {
        return Err(SignerError::InvalidSecret(format!(
            "secp256k1 key must be 32 bytes, got {}",
            bytes.len()
        )));
    }
    SigningKey::from_slice(&bytes)
        .map_err(|e| SignerError::InvalidSecret(format!("invalid secp256k1 key: {e}")))
}

fn parse_ed25519_secret(
    secret: &str,
) -> Result<(ed25519_dalek::SigningKey, ed25519_dalek::VerifyingKey), SignerError> {
    let trimmed = secret.trim();
    let trimmed = trimmed.strip_prefix("ed25519:").unwrap_or(trimmed);
    let bytes = bs58::decode(trimmed)
        .into_vec()
        .map_err(|e| SignerError::InvalidSecret(format!("expected base58 ed25519 key: {e}")))?;
    let seed_bytes: [u8; 32] = match bytes.len() {
        32 => bytes
            .try_into()
            .map_err(|_| SignerError::InvalidSecret("ed25519 32-byte slice".into()))?,
        64 => bytes[..32]
            .try_into()
            .map_err(|_| SignerError::InvalidSecret("ed25519 64-byte slice prefix".into()))?,
        n => {
            return Err(SignerError::InvalidSecret(format!(
                "ed25519 key must be 32 or 64 bytes, got {n}"
            )));
        }
    };
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&seed_bytes);
    let verifying_key = signing_key.verifying_key();
    Ok((signing_key, verifying_key))
}

fn derive_eth_address(key: &SigningKey) -> [u8; 20] {
    let pubkey_point = key.verifying_key().to_encoded_point(false);
    let pubkey_bytes = &pubkey_point.as_bytes()[1..];
    let hash = keccak256(pubkey_bytes);
    let mut addr = [0u8; 20];
    addr.copy_from_slice(&hash[12..]);
    addr
}

fn derive_secp256k1_public_key_hex(key: &SigningKey) -> String {
    let pubkey_point = key.verifying_key().to_encoded_point(false);
    format!("0x{}", hex::encode(pubkey_point.as_bytes()))
}

fn keccak256(input: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(input);
    hasher.finalize().into()
}

fn encode_eip712_type_string(primary: &crate::secrets::Eip712StructDef) -> String {
    let fields: Vec<String> = primary
        .fields
        .iter()
        .map(|f| format!("{} {}", f.type_name, f.name))
        .collect();
    format!("{}({})", primary.name, fields.join(","))
}

fn compute_eip712_domain_separator(
    domain: &crate::secrets::Eip712Domain,
) -> Result<[u8; 32], SignerError> {
    let mut domain_type = String::from("EIP712Domain(string name,string version,uint256 chainId");
    if domain.verifying_contract.is_some() {
        domain_type.push_str(",address verifyingContract");
    }
    domain_type.push(')');

    let domain_type_hash = keccak256(domain_type.as_bytes());
    let name_hash = keccak256(domain.name.as_bytes());
    let version_hash = keccak256(domain.version.as_bytes());

    let mut buf = Vec::with_capacity(32 * 5);
    buf.extend_from_slice(&domain_type_hash);
    buf.extend_from_slice(&name_hash);
    buf.extend_from_slice(&version_hash);

    let mut chain_id_be = [0u8; 32];
    chain_id_be[24..].copy_from_slice(&domain.chain_id.to_be_bytes());
    buf.extend_from_slice(&chain_id_be);

    if let Some(addr) = &domain.verifying_contract {
        let addr_bytes = parse_eth_address(addr)?;
        let mut padded = [0u8; 32];
        padded[12..].copy_from_slice(&addr_bytes);
        buf.extend_from_slice(&padded);
    }

    Ok(keccak256(&buf))
}

fn encode_eip712_field_value(type_name: &str, value: &str) -> Result<[u8; 32], SignerError> {
    match type_name {
        "string" => Ok(keccak256(value.as_bytes())),
        "address" => {
            let addr = parse_eth_address(value)?;
            let mut padded = [0u8; 32];
            padded[12..].copy_from_slice(&addr);
            Ok(padded)
        }
        "uint256" => {
            let num: u128 = value
                .parse()
                .map_err(|e| SignerError::InvalidField(format!("uint256 parse: {e}")))?;
            let mut padded = [0u8; 32];
            padded[16..].copy_from_slice(&num.to_be_bytes());
            Ok(padded)
        }
        "bytes32" => {
            let trimmed = value.trim_start_matches("0x");
            let bytes = hex::decode(trimmed)
                .map_err(|e| SignerError::InvalidField(format!("bytes32 hex: {e}")))?;
            if bytes.len() != 32 {
                return Err(SignerError::InvalidField(format!(
                    "bytes32 must be 32 bytes, got {}",
                    bytes.len()
                )));
            }
            let mut padded = [0u8; 32];
            padded.copy_from_slice(&bytes);
            Ok(padded)
        }
        other => Err(SignerError::InvalidField(format!(
            "unsupported eip712 type: {other}"
        ))),
    }
}

fn parse_eth_address(value: &str) -> Result<[u8; 20], SignerError> {
    let trimmed = value.trim().trim_start_matches("0x");
    let bytes = hex::decode(trimmed)
        .map_err(|e| SignerError::InvalidField(format!("address hex: {e}")))?;
    if bytes.len() != 20 {
        return Err(SignerError::InvalidField(format!(
            "address must be 20 bytes, got {}",
            bytes.len()
        )));
    }
    let mut addr = [0u8; 20];
    addr.copy_from_slice(&bytes);
    Ok(addr)
}

fn sign_secp256k1_recoverable(
    key: &SigningKey,
    hash: &[u8; 32],
) -> Result<([u8; 64], u8), SignerError> {
    use k256::ecdsa::{RecoveryId, VerifyingKey};

    let signature: K256Signature = key
        .sign_prehash(hash)
        .map_err(|e| SignerError::SignFailed(format!("secp256k1 sign: {e}")))?;
    let signature = signature.normalize_s().unwrap_or(signature);
    let verifying_key = key.verifying_key();

    let mut recovery_id: Option<u8> = None;
    for rid in 0..2u8 {
        let id = RecoveryId::try_from(rid)
            .map_err(|e| SignerError::SignFailed(format!("recovery id: {e}")))?;
        if let Ok(recovered) = VerifyingKey::recover_from_prehash(hash, &signature, id)
            && &recovered == verifying_key
        {
            recovery_id = Some(rid);
            break;
        }
    }
    let recovery_id = recovery_id
        .ok_or_else(|| SignerError::SignFailed("could not derive recovery id".into()))?;

    let sig_bytes: [u8; 64] = signature.to_bytes().into();
    Ok((sig_bytes, recovery_id))
}

fn resolve_field_source(
    source: &crate::secrets::FieldSource,
    signer_address: Option<&str>,
    signer_public_key: &str,
    body: Option<&[u8]>,
    timestamp_secs: u64,
    nonce_bytes: &[u8; 32],
) -> Result<String, SignerError> {
    use crate::secrets::FieldSource;
    match source {
        FieldSource::Literal { value } => Ok(value.clone()),
        FieldSource::SignerAddress => signer_address.map(str::to_string).ok_or_else(|| {
            SignerError::UnsupportedSource(
                "signer_address is not produced by this signer type (use signer_public_key instead)"
                    .into(),
            )
        }),
        FieldSource::SignerPublicKey => Ok(signer_public_key.to_string()),
        FieldSource::RequestTimestampSecs => Ok(timestamp_secs.to_string()),
        FieldSource::RequestRandomNonceB64 => {
            Ok(base64::engine::general_purpose::URL_SAFE.encode(nonce_bytes))
        }
        FieldSource::RequestBody => {
            let bytes = body.ok_or_else(|| {
                SignerError::UnsupportedSource(
                    "request_body source declared but no body present on this request".into(),
                )
            })?;
            String::from_utf8(bytes.to_vec()).map_err(|e| {
                SignerError::UnsupportedSource(format!("request_body is not valid utf-8: {e}"))
            })
        }
        FieldSource::Bytes32Keccak256OfBytes { parts } => {
            let assembled = assemble_bytes_for_keccak(parts, body)?;
            let h = keccak256(&assembled);
            Ok(format!("0x{}", hex::encode(h)))
        }
    }
}

fn resolve_output_source(
    source: &crate::secrets::OutputSource,
    signer_address: Option<&str>,
    signer_public_key: &str,
    signature_hex: &str,
    signature_b64: &str,
    timestamp_secs: u64,
    nonce_bytes: &[u8; 32],
) -> Result<String, SignerError> {
    use crate::secrets::OutputSource;
    match source {
        OutputSource::Literal { value } => Ok(value.clone()),
        OutputSource::SignerAddress => signer_address.map(str::to_string).ok_or_else(|| {
            SignerError::UnsupportedSource(
                "signer_address is not produced by this signer type (use signer_public_key instead)"
                    .into(),
            )
        }),
        OutputSource::SignerPublicKey => Ok(signer_public_key.to_string()),
        OutputSource::RequestTimestampSecs => Ok(timestamp_secs.to_string()),
        OutputSource::RequestRandomNonceB64 => {
            Ok(base64::engine::general_purpose::URL_SAFE.encode(nonce_bytes))
        }
        OutputSource::SignatureHex => Ok(signature_hex.to_string()),
        OutputSource::SignatureBase64 => Ok(signature_b64.to_string()),
    }
}

struct EvaluatedSignature {
    signer_address: String,
    signer_public_key: String,
    signature_hex: String,
    signature_b64: String,
    signature_r_hex: String,
    signature_s_hex: String,
    signature_v: u8,
    timestamp_secs: u64,
    nonce_bytes: [u8; 32],
}

fn evaluate_body_value(
    value: &crate::secrets::BodyValue,
    sig: &EvaluatedSignature,
) -> serde_json::Value {
    use crate::secrets::BodyValue;
    use serde_json::json;
    match value {
        BodyValue::SignerAddress => json!(sig.signer_address),
        BodyValue::SignerPublicKey => json!(sig.signer_public_key),
        BodyValue::SignatureHex => json!(sig.signature_hex),
        BodyValue::SignatureBase64 => json!(sig.signature_b64),
        BodyValue::SignatureRHex => json!(sig.signature_r_hex),
        BodyValue::SignatureSHex => json!(sig.signature_s_hex),
        BodyValue::SignatureV => json!(sig.signature_v),
        BodyValue::RequestTimestampSecs => json!(sig.timestamp_secs),
        BodyValue::RequestRandomNonceB64 => json!(
            base64::engine::general_purpose::URL_SAFE.encode(sig.nonce_bytes)
        ),
        BodyValue::LiteralString { value } => json!(value),
        BodyValue::LiteralNumber { value } => json!(value),
    }
}

fn json_path_get<'a>(root: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut current = root;
    for part in path.split('.') {
        current = current.as_object()?.get(part)?;
    }
    Some(current)
}

fn json_path_set(
    root: &mut serde_json::Value,
    path: &str,
    value: serde_json::Value,
) -> Result<(), SignerError> {
    if path.is_empty() || path.split('.').any(str::is_empty) {
        return Err(SignerError::InvalidSchema(format!(
            "invalid json path '{path}'"
        )));
    }
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = root;
    for (idx, part) in parts.iter().enumerate() {
        let is_last = idx + 1 == parts.len();
        let map = match current {
            serde_json::Value::Object(m) => m,
            _ => {
                return Err(SignerError::SignFailed(format!(
                    "json path '{path}' descends into a non-object"
                )));
            }
        };
        if is_last {
            map.insert((*part).to_string(), value);
            return Ok(());
        }
        if !map.contains_key(*part) {
            map.insert((*part).to_string(), serde_json::Value::Object(serde_json::Map::new()));
        }
        current = map
            .get_mut(*part)
            .ok_or_else(|| SignerError::SignFailed(format!("json path '{path}' walk failed")))?;
    }
    Ok(())
}

fn apply_body_mutations(
    body: &mut Option<Vec<u8>>,
    mutations: &[(String, serde_json::Value)],
) -> Result<(), SignerError> {
    if mutations.is_empty() {
        return Ok(());
    }
    let bytes = body.as_deref().ok_or_else(|| {
        SignerError::UnsupportedSource(
            "signing declares output_body_fields but no request body provided".into(),
        )
    })?;
    let mut json: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| SignerError::SignFailed(format!("request body is not json: {e}")))?;
    for (path, value) in mutations {
        json_path_set(&mut json, path, value.clone())?;
    }
    let new_bytes = serde_json::to_vec(&json)
        .map_err(|e| SignerError::SignFailed(format!("re-serialize body: {e}")))?;
    *body = Some(new_bytes);
    Ok(())
}

fn assemble_bytes_for_keccak(
    parts: &[crate::secrets::BytesPart],
    body: Option<&[u8]>,
) -> Result<Vec<u8>, SignerError> {
    use crate::secrets::BytesPart;
    let needs_body = parts
        .iter()
        .any(|p| !matches!(p, BytesPart::LiteralHex { .. }));
    let body_json = if needs_body {
        let bytes = body.ok_or_else(|| {
            SignerError::UnsupportedSource(
                "body-derived bytes_part declared but no request body provided".into(),
            )
        })?;
        Some(
            serde_json::from_slice::<serde_json::Value>(bytes)
                .map_err(|e| SignerError::InvalidField(format!("body not json: {e}")))?,
        )
    } else {
        None
    };

    let mut out: Vec<u8> = Vec::new();
    for part in parts {
        let bytes = evaluate_bytes_part(part, body_json.as_ref())?;
        out.extend_from_slice(&bytes);
    }
    Ok(out)
}

fn evaluate_bytes_part(
    part: &crate::secrets::BytesPart,
    body_json: Option<&serde_json::Value>,
) -> Result<Vec<u8>, SignerError> {
    use crate::secrets::BytesPart;
    match part {
        BytesPart::LiteralHex { hex } => {
            let trimmed = hex.trim_start_matches("0x");
            hex::decode(trimmed)
                .map_err(|e| SignerError::InvalidField(format!("literal_hex: {e}")))
        }
        BytesPart::BodyFieldMsgpack { path } => {
            let json = body_json.ok_or_else(|| {
                SignerError::UnsupportedSource(
                    "body_field_msgpack source needs request body".into(),
                )
            })?;
            let field = json_path_get(json, path).ok_or_else(|| {
                SignerError::InvalidField(format!("body field '{path}' not found"))
            })?;
            let mut out: Vec<u8> = Vec::new();
            field
                .serialize(&mut MsgpackSerializer::new(&mut out).with_struct_map())
                .map_err(|e| SignerError::InvalidField(format!("msgpack encode: {e}")))?;
            Ok(out)
        }
        BytesPart::BodyFieldBeU64 { path } => {
            let json = body_json.ok_or_else(|| {
                SignerError::UnsupportedSource(
                    "body_field_be_u64 source needs request body".into(),
                )
            })?;
            let field = json_path_get(json, path).ok_or_else(|| {
                SignerError::InvalidField(format!("body field '{path}' not found"))
            })?;
            let n = field.as_u64().ok_or_else(|| {
                SignerError::InvalidField(format!(
                    "body field '{path}' must be a non-negative integer"
                ))
            })?;
            Ok(n.to_be_bytes().to_vec())
        }
        BytesPart::BodyFieldEthAddressOptionalPrefixed { path } => {
            let json = body_json.ok_or_else(|| {
                SignerError::UnsupportedSource(
                    "body_field_eth_address_optional_prefixed source needs request body".into(),
                )
            })?;
            match json_path_get(json, path) {
                None | Some(serde_json::Value::Null) => Ok(vec![0x00]),
                Some(serde_json::Value::String(s)) => {
                    let addr = parse_eth_address(s)?;
                    let mut out = Vec::with_capacity(21);
                    out.push(0x01);
                    out.extend_from_slice(&addr);
                    Ok(out)
                }
                Some(_) => Err(SignerError::InvalidField(format!(
                    "body field '{path}' must be a string or null"
                ))),
            }
        }
    }
}

// Provide WASI context for the WASM component.
// Required because tools are compiled with wasm32-wasip2 target.
impl WasiView for StoreData {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

// Implement the generated Host trait from bindgen.
//
// This registers all 6 host functions under the `near:agent/host` namespace:
// log, now-millis, workspace-read, http-request, secret-exists, tool-invoke
impl near::agent::host::Host for StoreData {
    fn log(&mut self, level: near::agent::host::LogLevel, message: String) {
        let log_level = match level {
            near::agent::host::LogLevel::Trace => LogLevel::Trace,
            near::agent::host::LogLevel::Debug => LogLevel::Debug,
            near::agent::host::LogLevel::Info => LogLevel::Info,
            near::agent::host::LogLevel::Warn => LogLevel::Warn,
            near::agent::host::LogLevel::Error => LogLevel::Error,
        };
        let _ = self.host_state.log(log_level, message);
    }

    fn now_millis(&mut self) -> u64 {
        self.host_state.now_millis()
    }

    fn workspace_read(&mut self, path: String) -> Option<String> {
        self.host_state.workspace_read(&path).ok().flatten()
    }

    fn http_request(
        &mut self,
        method: String,
        url: String,
        headers_json: String,
        mut body: Option<Vec<u8>>,
        timeout_ms: Option<u32>,
    ) -> Result<near::agent::host::HttpResponse, String> {
        // Inject credentials into URL (e.g., replace {TELEGRAM_BOT_TOKEN})
        let injected_url = self.inject_credentials(&url, "url");

        // Check HTTP allowlist
        self.host_state
            .check_http_allowed(&injected_url, &method)
            .map_err(|e| format!("HTTP not allowed: {}", e))?;

        // Record for rate limiting
        self.host_state
            .record_http_request()
            .map_err(|e| format!("Rate limit exceeded: {}", e))?;

        // Parse headers and inject credentials into header values
        let raw_headers: HashMap<String, String> =
            serde_json::from_str(&headers_json).unwrap_or_default();

        // Leak scan runs on WASM-provided values BEFORE host credential injection.
        // This prevents false positives where the host-injected Bearer token
        // (e.g., xoxb- Slack token) triggers the leak detector — WASM never saw
        // the real value, so scanning the pre-injection state is correct.
        // Inline the scan to avoid allocating a Vec of cloned headers.
        let leak_detector = LeakDetector::new();
        leak_detector
            .scan_and_clean(&injected_url)
            .map_err(|e| format!("Potential secret leak in URL blocked: {}", e))?;
        for (name, value) in &raw_headers {
            leak_detector.scan_and_clean(value).map_err(|e| {
                format!("Potential secret leak in header '{}' blocked: {}", name, e)
            })?;
        }
        if let Some(body_bytes) = body.as_deref() {
            let body_str = String::from_utf8_lossy(body_bytes);
            leak_detector
                .scan_and_clean(&body_str)
                .map_err(|e| format!("Potential secret leak in body blocked: {}", e))?;
        }

        let mut headers: HashMap<String, String> = raw_headers
            .into_iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    self.inject_credentials(&v, &format!("header:{}", k)),
                )
            })
            .collect();

        let mut url = injected_url;

        // Inject pre-resolved host credentials (Bearer tokens, API keys, etc.)
        // based on the request's target host.
        if let Some(host) = extract_host_from_url(&url) {
            self.inject_host_credentials(&host, &method, &mut body, &mut headers, &mut url);
        }

        // Get the max response size from capabilities (default 10MB).
        let max_response_bytes = self
            .host_state
            .capabilities()
            .http
            .as_ref()
            .map(|h| h.max_response_bytes)
            .unwrap_or(10 * 1024 * 1024);

        // Make HTTP request using a dedicated single-threaded runtime.
        // We're inside spawn_blocking, so we can't rely on the main runtime's
        // I/O driver (it may be busy with WASM compilation or other startup work).
        // A dedicated runtime gives us our own I/O driver and avoids contention.
        // The runtime is lazily created and reused across calls within one execution.
        if self.http_runtime.is_none() {
            self.http_runtime = Some(
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| format!("Failed to create HTTP runtime: {e}"))?,
            );
        }
        let rt = self.http_runtime.as_ref().expect("just initialized"); // safety: is_none branch above guarantees Some

        // Resolve the destination once, reject private/internal addresses, and
        // reuse the validated addresses in reqwest so there is no second DNS
        // lookup window for rebinding between validation and connect.
        let resolved_target = rt.block_on(validate_and_resolve_http_target(&url))?;

        // If an HTTP interceptor is set (testing), short-circuit with a canned response.
        if let Some(interceptor) = &self.http_interceptor {
            let interceptor = Arc::clone(interceptor);
            let intercept_url = url.clone();
            let intercept_method = method.clone();
            let mut intercept_headers: Vec<(String, String)> = headers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            intercept_headers.sort_by(|a, b| a.0.cmp(&b.0));
            let intercept_body = body
                .as_ref()
                .map(|b| String::from_utf8_lossy(b).to_string());
            let intercepted = rt.block_on(async {
                let req = HttpExchangeRequest {
                    method: intercept_method,
                    url: intercept_url,
                    headers: intercept_headers,
                    body: intercept_body,
                };
                interceptor.before_request(&req).await
            });
            if let Some(resp) = intercepted {
                let resp_headers: HashMap<String, String> = resp
                    .headers
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                let resp_headers_json =
                    serde_json::to_string(&resp_headers).unwrap_or_else(|_| "{}".to_string());
                return Ok(near::agent::host::HttpResponse {
                    status: resp.status,
                    headers_json: resp_headers_json,
                    body: resp.body.into_bytes(),
                });
            }
        }

        // Capture request metadata before headers/body are consumed by the reqwest
        // builder. Used for after_response callback when a recording interceptor is set.
        let interceptor_req = self.http_interceptor.as_ref().map(|_| HttpExchangeRequest {
            method: method.clone(),
            url: url.clone(),
            headers: headers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            body: body
                .as_ref()
                .map(|b| String::from_utf8_lossy(b).to_string()),
        });

        let result = rt.block_on(async {
            let client = ssrf_safe_client_builder_for_target(&resolved_target)
                .connect_timeout(Duration::from_secs(10))
                .build()
                .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

            let mut request = match method.to_uppercase().as_str() {
                "GET" => client.get(&url),
                "POST" => client.post(&url),
                "PUT" => client.put(&url),
                "DELETE" => client.delete(&url),
                "PATCH" => client.patch(&url),
                "HEAD" => client.head(&url),
                _ => return Err(format!("Unsupported HTTP method: {}", method)),
            };

            for (key, value) in &headers {
                request = request.header(key, value);
            }

            if let Some(body_bytes) = body {
                request = request.body(body_bytes);
            } else if needs_content_length_zero(&method, &headers) {
                request = request.header("content-length", "0");
            }

            // Caller-specified timeout (default 30s, max 5min)
            let timeout_ms = timeout_ms.unwrap_or(30_000).min(300_000) as u64;
            let timeout = Duration::from_millis(timeout_ms);
            let response = request.timeout(timeout).send().await.map_err(|e| {
                // Walk the full error chain for the actual root cause
                let mut chain = format!("HTTP request failed: {}", e);
                let mut source = std::error::Error::source(&e);
                while let Some(cause) = source {
                    chain.push_str(&format!(" -> {}", cause));
                    source = cause.source();
                }
                chain
            })?;

            let status = response.status().as_u16();
            let response_headers: HashMap<String, String> = response
                .headers()
                .iter()
                .filter_map(|(k, v)| {
                    v.to_str()
                        .ok()
                        .map(|v| (k.as_str().to_string(), v.to_string()))
                })
                .collect();
            let headers_json = serde_json::to_string(&response_headers).unwrap_or_default();

            // Check Content-Length header for early rejection of oversized responses.
            let max_response = max_response_bytes;
            if let Some(cl) = response.content_length()
                && cl as usize > max_response
            {
                return Err(format!(
                    "Response body too large: {} bytes exceeds limit of {} bytes",
                    cl, max_response
                ));
            }

            // Read body with a size cap to prevent memory exhaustion.
            let body = response
                .bytes()
                .await
                .map_err(|e| format!("Failed to read response body: {}", e))?;
            if body.len() > max_response {
                return Err(format!(
                    "Response body too large: {} bytes exceeds limit of {} bytes",
                    body.len(),
                    max_response
                ));
            }
            let body = body.to_vec();

            // Leak detection on response body
            if let Ok(body_str) = std::str::from_utf8(&body) {
                leak_detector
                    .scan_and_clean(body_str)
                    .map_err(|e| format!("Potential secret leak in response: {}", e))?;
            }

            Ok(near::agent::host::HttpResponse {
                status,
                headers_json,
                body,
            })
        });

        // Notify the interceptor about the completed response (recording mode).
        // RecordingHttpInterceptor returns None from before_request and captures
        // exchanges via after_response, so this path is exercised during trace recording.
        if let (Some(interceptor), Some(req), Ok(resp)) =
            (&self.http_interceptor, &interceptor_req, &result)
        {
            let interceptor = Arc::clone(interceptor);

            // Redact credentials from request before passing to the interceptor
            // to prevent credential leakage into recorded traces.
            let mut redacted_req = req.clone();
            redacted_req.url = self.redact_credentials(&redacted_req.url);
            redacted_req.headers = redacted_req
                .headers
                .into_iter()
                .map(|(k, v)| (k, self.redact_credentials(&v)))
                .collect();
            redacted_req.body = redacted_req.body.map(|b| self.redact_credentials(&b));

            let resp_headers: Vec<(String, String)> =
                serde_json::from_str::<HashMap<String, String>>(&resp.headers_json)
                    .unwrap_or_default()
                    .into_iter()
                    .collect();
            let resp_body = String::from_utf8_lossy(&resp.body).to_string();

            // Redact credentials from response as well
            let redacted_headers: Vec<(String, String)> = resp_headers
                .into_iter()
                .map(|(k, v)| (k, self.redact_credentials(&v)))
                .collect();
            let redacted_body = self.redact_credentials(&resp_body);

            let exchange_resp = HttpExchangeResponse {
                status: resp.status,
                headers: redacted_headers,
                body: redacted_body,
            };
            rt.block_on(async {
                interceptor
                    .after_response(&redacted_req, &exchange_resp)
                    .await;
            });
        }

        // Redact credentials from error messages before returning to WASM
        result.map_err(|e| self.redact_credentials(&e))
    }

    fn tool_invoke(&mut self, alias: String, _params_json: String) -> Result<String, String> {
        // Validate capability and resolve alias
        let _real_name = self.host_state.check_tool_invoke_allowed(&alias)?;
        self.host_state.record_tool_invoke()?;

        // Tool invocation requires async context and access to the tool registry,
        // which aren't available inside a synchronous WASM callback.
        Err("Tool invocation from WASM tools is not yet supported".to_string())
    }

    fn secret_exists(&mut self, name: String) -> bool {
        self.host_state.secret_exists(&name)
    }
}

/// A Tool implementation backed by a WASM component.
///
/// Each call to `execute` creates a fresh instance for isolation.
pub struct WasmToolWrapper {
    /// Runtime for engine access.
    runtime: Arc<WasmToolRuntime>,
    /// Prepared module with compiled component.
    prepared: Arc<PreparedModule>,
    /// Capabilities to grant to this tool.
    capabilities: Capabilities,
    /// Cached description (from PreparedModule or override).
    /// Stored without any tool_info hints — hints are composed at display time.
    description: String,
    /// Compact and discovery schemas for this tool.
    schemas: WasmToolSchemas,
    /// Optional curated discovery guidance surfaced by `tool_info`.
    discovery_summary: Option<ToolDiscoverySummary>,
    /// Injected credentials for HTTP requests (e.g., OAuth tokens).
    /// Keys are placeholder names like "GOOGLE_ACCESS_TOKEN".
    credentials: HashMap<String, String>,
    /// Secrets store for resolving host-based credential injection.
    /// Used in execute() to pre-decrypt secrets before WASM runs.
    secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
    /// Database for role-aware legacy default-scope credential fallback.
    role_lookup: Option<Arc<dyn UserStore>>,
    /// OAuth refresh configuration for auto-refreshing expired tokens.
    oauth_refresh: Option<OAuthRefreshConfig>,
    /// Optional HTTP interceptor for testing — returns canned responses
    /// instead of making real requests when set.
    http_interceptor: Option<Arc<dyn HttpInterceptor>>,
}

#[derive(Debug, Clone)]
struct WasmToolSchemas {
    /// Compact schema advertised in the main tools array.
    ///
    /// This stays permissive by default to avoid serializing full exported
    /// WASM schemas on every LLM call. Sidecars can override it explicitly.
    advertised: serde_json::Value,
    /// Full schema available for discovery and runtime parameter preparation.
    ///
    /// Seeded from the WASM `schema()` export at registration time, unless a
    /// sidecar explicitly overrides it.
    discovery: serde_json::Value,
}

impl WasmToolSchemas {
    fn permissive_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "additionalProperties": true
        })
    }

    fn is_permissive_schema(schema: &serde_json::Value) -> bool {
        crate::tools::schema_metrics::is_permissive_schema(schema)
    }

    fn typed_property_count(schema: &serde_json::Value) -> usize {
        crate::tools::schema_metrics::typed_property_count(schema)
    }

    fn new(discovery: serde_json::Value) -> Self {
        let advertised = Self::compact_schema(&discovery);
        Self {
            advertised,
            discovery,
        }
    }

    /// Derive a compact advertised schema from the full discovery schema.
    ///
    /// Two distinct shapes are handled:
    ///
    /// 1. **Tagged enum / `oneOf` shape** (e.g. WASM tools whose action
    ///    enum is exposed via `schemars::JsonSchema`, or hand-written
    ///    `oneOf` schemas like `github`'s). The `oneOf` structure is
    ///    *preserved* — including each variant's `properties` and
    ///    `required` array — so the LLM can see "field X is required
    ///    when action == Y" before constructing a call. This is
    ///    critical: previously these arrays were stripped out and the
    ///    LLM would happily call `{"action":"get_file"}` without
    ///    `file_id`, getting a runtime serde error. We strip
    ///    `description`, `default`, `title`, `format`, `examples`, and
    ///    `$schema` from each variant to save tokens — the contract
    ///    (types + required) survives, the prose doesn't.
    ///
    /// 2. **Flat shape** (no `oneOf`/`anyOf`/`allOf`). Keeps top-level
    ///    properties that are either in `required` or carry an
    ///    `enum`/`const` constraint, with descriptions stripped. If
    ///    nothing survives the filter, falls back to all typed properties
    ///    or to a permissive `{}` schema.
    ///
    /// At most `MAX_COMPACT_VARIANTS` variants and
    /// `MAX_COMPACT_PROPERTIES` flat properties are kept to bound
    /// allocations from adversarial schemas.
    fn compact_schema(discovery: &serde_json::Value) -> serde_json::Value {
        const MAX_COMPACT_PROPERTIES: usize = 100;
        const MAX_COMPACT_VARIANTS: usize = 50;

        // Shape 1: tagged enum / oneOf schema. Preserve the structure so
        // the LLM sees per-variant required arrays.
        for combinator in ["oneOf", "anyOf", "allOf"] {
            if let Some(variants) = discovery.get(combinator).and_then(|v| v.as_array())
                && !variants.is_empty()
            {
                let compact_variants: Vec<serde_json::Value> = variants
                    .iter()
                    .take(MAX_COMPACT_VARIANTS)
                    .map(strip_schema_metadata)
                    .collect();
                let mut result = serde_json::Map::new();
                result.insert(
                    "type".to_string(),
                    serde_json::Value::String("object".to_string()),
                );
                if let Some(top_required) = discovery.get("required") {
                    result.insert("required".to_string(), top_required.clone());
                }
                // Carry through any top-level properties (rare with
                // schemars-derived schemas, but possible with hybrid
                // hand-written ones).
                if let Some(top_props) = discovery.get("properties") {
                    result.insert("properties".to_string(), strip_props_metadata(top_props));
                }
                result.insert(
                    combinator.to_string(),
                    serde_json::Value::Array(compact_variants),
                );
                result.insert(
                    "additionalProperties".to_string(),
                    serde_json::Value::Bool(true),
                );
                return serde_json::Value::Object(result);
            }
        }

        // Shape 2: flat schema. Keep required + enum/const-bearing
        // properties, drop the rest.
        let required: std::collections::HashSet<String> = discovery
            .get("required")
            .and_then(|r| r.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let mut all_properties = serde_json::Map::new();
        if let Some(props) = discovery.get("properties").and_then(|p| p.as_object()) {
            for (k, v) in props {
                if all_properties.len() >= MAX_COMPACT_PROPERTIES {
                    break;
                }
                all_properties.insert(k.clone(), strip_schema_metadata(v));
            }
        }

        if all_properties.is_empty() {
            return Self::permissive_schema();
        }

        let kept: serde_json::Map<String, serde_json::Value> = all_properties
            .iter()
            .filter(|(name, prop)| {
                required.contains(name.as_str())
                    || prop.get("enum").is_some()
                    || prop.get("const").is_some()
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        if kept.is_empty() {
            // When the schema has typed properties but none survived the
            // required/enum filter, include all typed properties so the LLM
            // sees meaningful parameter hints instead of permissive `{}`.
            let typed: serde_json::Map<String, serde_json::Value> = all_properties
                .into_iter()
                .filter(|(_, prop)| schema_is_typed_property(prop))
                .collect();
            if typed.is_empty() {
                return Self::permissive_schema();
            }
            return serde_json::json!({
                "type": "object",
                "properties": typed,
                "additionalProperties": true,
            });
        }

        let kept_required: Vec<serde_json::Value> = required
            .iter()
            .filter(|name| kept.contains_key(name.as_str()))
            .map(|name| serde_json::Value::String(name.clone()))
            .collect();

        let mut result = serde_json::json!({
            "type": "object",
            "properties": kept,
            "additionalProperties": true,
        });
        if !kept_required.is_empty() {
            result["required"] = serde_json::Value::Array(kept_required);
        }

        result
    }

    fn with_override(&self, schema: serde_json::Value) -> Self {
        Self {
            advertised: schema.clone(),
            discovery: schema,
        }
    }

    fn is_advertised_permissive(&self) -> bool {
        Self::is_permissive_schema(&self.advertised)
    }

    fn advertised(&self) -> serde_json::Value {
        self.advertised.clone()
    }

    fn discovery(&self) -> serde_json::Value {
        self.discovery.clone()
    }
}

impl WasmToolWrapper {
    /// Create a new WASM tool wrapper.
    pub fn new(
        runtime: Arc<WasmToolRuntime>,
        prepared: Arc<PreparedModule>,
        capabilities: Capabilities,
    ) -> Self {
        Self {
            description: prepared.description.clone(),
            schemas: WasmToolSchemas::new(prepared.schema.clone()),
            discovery_summary: None,
            runtime,
            prepared,
            capabilities,
            credentials: HashMap::new(),
            secrets_store: None,
            role_lookup: None,
            oauth_refresh: None,
            http_interceptor: None,
        }
    }

    /// Set an HTTP interceptor for testing.
    ///
    /// When set, WASM tool HTTP requests are routed through the interceptor
    /// instead of making real network calls. This allows tests to verify the
    /// exact HTTP requests a WASM tool constructs.
    pub fn with_http_interceptor(mut self, interceptor: Arc<dyn HttpInterceptor>) -> Self {
        self.http_interceptor = Some(interceptor);
        self
    }

    /// Override the tool description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Override the parameter schema.
    pub fn with_schema(mut self, schema: serde_json::Value) -> Self {
        let override_typed = WasmToolSchemas::typed_property_count(&schema);
        let prepared_typed = WasmToolSchemas::typed_property_count(&self.prepared.schema);

        if override_typed == 0 && prepared_typed > 0 {
            tracing::warn!(
                tool = %self.prepared.name,
                "Ignoring untyped schema override for discovery/runtime preparation and preserving extracted WASM schema"
            );
            self.schemas = WasmToolSchemas {
                advertised: schema,
                discovery: self.prepared.schema.clone(),
            };
        } else {
            self.schemas = self.schemas.with_override(schema);
        }
        self
    }

    /// Override the curated discovery summary.
    pub fn with_discovery_summary(mut self, summary: ToolDiscoverySummary) -> Self {
        self.discovery_summary = Some(summary);
        self
    }

    /// Set credentials for HTTP request placeholder injection.
    pub fn with_credentials(mut self, credentials: HashMap<String, String>) -> Self {
        self.credentials = credentials;
        self
    }

    /// Set the secrets store for host-based credential injection.
    ///
    /// When set, credentials declared in the tool's capabilities are
    /// automatically decrypted and injected into HTTP requests based
    /// on the target host (e.g., Bearer token for www.googleapis.com).
    pub fn with_secrets_store(mut self, store: Arc<dyn SecretsStore + Send + Sync>) -> Self {
        self.secrets_store = Some(store);
        self
    }

    /// Set the role lookup for admin-only legacy default-scope fallback.
    pub fn with_role_lookup(mut self, role_lookup: Arc<dyn UserStore>) -> Self {
        self.role_lookup = Some(role_lookup);
        self
    }

    /// Set OAuth refresh configuration for auto-refreshing expired tokens.
    ///
    /// When set, `execute()` checks the access token's `expires_at` before
    /// each call and silently refreshes it using the stored refresh token.
    pub fn with_oauth_refresh(mut self, config: OAuthRefreshConfig) -> Self {
        self.oauth_refresh = Some(config);
        self
    }

    /// Get the resource limits for this tool.
    pub fn limits(&self) -> &ResourceLimits {
        &self.prepared.limits
    }

    /// Add all host functions to the linker using generated bindings.
    ///
    /// Uses the bindgen-generated `add_to_linker` function to properly register
    /// all host functions with correct component model signatures under the
    /// `near:agent/host` namespace.
    fn add_host_functions(linker: &mut Linker<StoreData>) -> Result<(), WasmError> {
        // Add WASI support (required by components built with wasm32-wasip2)
        wasmtime_wasi::p2::add_to_linker_sync(linker)
            .map_err(|e| WasmError::ConfigError(format!("Failed to add WASI functions: {}", e)))?;

        // Add our custom host interface using the generated add_to_linker
        SandboxedTool::add_to_linker::<_, wasmtime::component::HasSelf<_>>(
            linker,
            |state: &mut StoreData| state,
        )
        .map_err(|e| WasmError::ConfigError(format!("Failed to add host functions: {}", e)))?;

        Ok(())
    }

    /// Execute the WASM tool synchronously (called from spawn_blocking).
    fn execute_sync(
        &self,
        params: serde_json::Value,
        context_json: Option<String>,
        host_credentials: Vec<ResolvedHostCredential>,
    ) -> Result<(String, Vec<crate::tools::wasm::host::LogEntry>), WasmError> {
        let engine = self.runtime.engine();
        let limits = &self.prepared.limits;

        // Create store with fresh state (NEAR pattern: fresh instance per call)
        let mut store_data = StoreData::new(
            limits.memory_bytes,
            self.capabilities.clone(),
            self.credentials.clone(),
            host_credentials,
        );
        store_data.http_interceptor = self.http_interceptor.clone();
        let mut store = Store::new(engine, store_data);

        // Configure fuel if enabled
        if self.runtime.config().fuel_config.enabled {
            store
                .set_fuel(limits.fuel)
                .map_err(|e| WasmError::ConfigError(format!("Failed to set fuel: {}", e)))?;
        }

        // Configure epoch deadline as a hard timeout backup.
        // The epoch ticker thread increments the engine epoch every EPOCH_TICK_INTERVAL.
        // Setting deadline to N means "trap after N ticks", so we compute the number
        // of ticks that fit in the tool's timeout. Minimum 1 to always have a backstop.
        store.epoch_deadline_trap();
        let ticks = (limits.timeout.as_millis() / EPOCH_TICK_INTERVAL.as_millis()).max(1) as u64;
        store.set_epoch_deadline(ticks);

        // Set up resource limiter
        store.limiter(|data| &mut data.limiter);

        // Use the pre-compiled component (no recompilation needed)
        let component = self.prepared.component().clone();

        // Create linker with all host functions properly namespaced
        let mut linker = Linker::new(engine);
        Self::add_host_functions(&mut linker)?;

        // Instantiate using the generated bindings
        let instance =
            SandboxedTool::instantiate(&mut store, &component, &linker).map_err(|e| {
                let msg = e.to_string();
                if msg.contains("near:agent") || msg.contains("import") {
                    WasmError::InstantiationFailed(format!(
                        "{msg}. This usually means the extension was compiled against \
                         a different WIT version than the host supports. \
                         Rebuild the extension against the current WIT (host: {}).",
                        crate::tools::wasm::WIT_TOOL_VERSION
                    ))
                } else {
                    WasmError::InstantiationFailed(msg)
                }
            })?;

        // Get typed interface — used for execute.
        let tool_iface = instance.near_agent_tool();

        // Prepare the request
        let params_json = serde_json::to_string(&params)
            .map_err(|e| WasmError::InvalidResponseJson(e.to_string()))?;

        let request = wit_tool::Request {
            params: params_json,
            context: context_json,
        };

        // Call execute using the generated typed interface
        let response = tool_iface
            .call_execute(&mut store, &request)
            .map_err(|e| classify_trap_error(e, limits))?;

        // Log fuel consumption for diagnostics
        if self.runtime.config().fuel_config.enabled
            && let Ok(remaining) = store.get_fuel()
        {
            let consumed = limits.fuel.saturating_sub(remaining);
            let pct = (consumed as f64 / limits.fuel as f64) * 100.0;
            tracing::debug!(
                tool = %self.prepared.name,
                fuel_consumed = consumed,
                fuel_remaining = remaining,
                fuel_limit = limits.fuel,
                fuel_pct = format!("{pct:.1}%"),
                "WASM fuel consumption"
            );
        }

        // Get logs from host state
        let logs = store.data_mut().host_state.take_logs();

        // Check for tool-level error — point the LLM to tool_info for the
        // full schema instead of dumping ~3.5KB inline.
        if let Some(err) = response.error {
            let hint = build_tool_usage_hint(&self.prepared.name, &self.schemas.discovery());
            return Err(WasmError::ToolReturnedError { message: err, hint });
        }

        // Return result (or empty string if none)
        Ok((response.output.unwrap_or_default(), logs))
    }
}

/// Classify a wasmtime execution error into the appropriate `WasmError` variant.
///
/// Prefers structured `Trap` downcast (version-proof) when the error type
/// exposes a `wasmtime::Trap` directly. Falls back to string matching on the
/// full error chain for cases where component-model glue or host wrappers
/// bury the trap inside a nested cause (the `downcast_ref` on the outer
/// error misses it, but the trap's diagnostic string still appears in the
/// `Display` chain). The string fallback covers `OutOfFuel` and
/// `unreachable` — the two traps that have distinct `WasmError` variants —
/// and is forward-compatible with future wasmtime versions that might rename
/// or restructure the type hierarchy.
///
/// Takes `wasmtime::Error` directly (not `anyhow::Error`) because that's
/// what `call_execute` returns. wasmtime 43+ has its own `Error` type
/// distinct from `anyhow::Error`; accepting it natively avoids a lossy
/// `.into()` conversion that could strip type information needed for the
/// downcast.
fn classify_trap_error(error: wasmtime::Error, limits: &ResourceLimits) -> WasmError {
    // Try structured downcast first (avoids string-matching drift across
    // wasmtime versions). `wasmtime::Error::downcast_ref` walks the error
    // chain internally, so traps wrapped by component-model glue are found.
    if let Some(trap) = error.downcast_ref::<wasmtime::Trap>() {
        return match trap {
            wasmtime::Trap::OutOfFuel => WasmError::FuelExhausted { limit: limits.fuel },
            wasmtime::Trap::StackOverflow => WasmError::Trapped(
                "stack overflow: the tool's call stack exceeded the WASM stack limit. \
                 This often happens when parsing very large JSON responses."
                    .to_string(),
            ),
            wasmtime::Trap::UnreachableCodeReached => {
                WasmError::Trapped("unreachable code executed".to_string())
            }
            // Everything else: include trap kind + full chain for diagnosis
            other => WasmError::Trapped(format!("{other}: {error:#}")),
        };
    }

    // Fallback: string matching on the full error chain. The downcast can
    // miss when the trap is wrapped in layers of component-model or host
    // glue that don't preserve the Trap type. The Display chain still
    // contains the diagnostic string, so we check for the two traps that
    // have distinct WasmError variants.
    let error_str = format!("{error:#}");
    if error_str.contains("all fuel consumed")
        || error_str.contains("out of fuel")
        || error_str.contains("OutOfFuel")
    {
        return WasmError::FuelExhausted { limit: limits.fuel };
    }
    // Match wasmtime's actual Display string for UnreachableCodeReached.
    // A bare `contains("unreachable")` would false-positive on HTTP errors
    // like "endpoint was unreachable" or "server unreachable: connection
    // refused", replacing the real diagnostic with a misleading generic
    // "unreachable code executed" message.
    if error_str.contains("unreachable code")
        || error_str.contains("UnreachableCodeReached")
        || error_str.contains("wasm trap: unreachable")
    {
        return WasmError::Trapped("unreachable code executed".to_string());
    }

    // Unrecognized: full chain for diagnosis
    WasmError::Trapped(error_str)
}

/// Extract metadata (description + schema) from a WASM tool by briefly
/// instantiating it and calling its `description()` and `schema()` exports.
/// Analogous to MCP's `list_tools()` — discovers tool capabilities at load time.
///
/// Falls back to generic description and permissive schema on failure.
pub(super) fn extract_wasm_metadata(
    engine: &wasmtime::Engine,
    component: &wasmtime::component::Component,
    limits: &ResourceLimits,
) -> Result<(String, serde_json::Value), WasmError> {
    let store_data = StoreData::new(
        limits.memory_bytes,
        Capabilities::default(),
        HashMap::new(),
        vec![],
    );
    let mut store = Store::new(engine, store_data);

    // Configure fuel + epoch deadline so extraction can't hang
    if let Err(e) = store.set_fuel(limits.fuel) {
        tracing::debug!("Fuel not enabled for metadata extraction: {e}");
    }
    store.epoch_deadline_trap();
    let ticks = (limits.timeout.as_millis() / EPOCH_TICK_INTERVAL.as_millis()).max(1) as u64;
    store.set_epoch_deadline(ticks);
    store.limiter(|data| &mut data.limiter);

    // Instantiate with minimal linker
    let mut linker = Linker::new(engine);
    WasmToolWrapper::add_host_functions(&mut linker)?;
    let instance = SandboxedTool::instantiate(&mut store, component, &linker)
        .map_err(|e| WasmError::InstantiationFailed(e.to_string()))?;
    let tool_iface = instance.near_agent_tool();

    // Extract description (fall back to generic)
    let description = tool_iface
        .call_description(&mut store)
        .unwrap_or_else(|_| "WASM sandboxed tool".to_string());

    // Extract and parse schema (fall back to permissive)
    let schema = tool_iface
        .call_schema(&mut store)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .unwrap_or_else(|| {
            serde_json::json!({"type": "object", "properties": {}, "additionalProperties": true})
        });

    Ok((description, schema))
}

#[async_trait]
impl Tool for WasmToolWrapper {
    fn name(&self) -> &str {
        &self.prepared.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        self.schemas.advertised()
    }

    fn discovery_schema(&self) -> serde_json::Value {
        self.schemas.discovery()
    }

    fn discovery_summary(&self) -> Option<ToolDiscoverySummary> {
        self.discovery_summary.clone()
    }

    fn provider_extension(&self) -> Option<&str> {
        Some(&self.prepared.name)
    }

    /// Compose the tool schema for LLM function calling.
    ///
    /// When the advertised schema is permissive (no typed properties), appends
    /// a hint to the description directing the LLM to call `tool_info` for the
    /// full parameter schema. This keeps the raw description clean while still
    /// guiding the LLM.
    fn schema(&self) -> crate::tools::tool::ToolSchema {
        let description = if self.schemas.is_advertised_permissive() {
            format!(
                "{} (call tool_info(name: \"{}\", include_schema: true) for parameter schema)",
                self.description, self.prepared.name
            )
        } else {
            self.description.clone()
        };
        crate::tools::tool::ToolSchema {
            name: self.prepared.name.clone(),
            description,
            parameters: self.schemas.advertised(),
        }
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let timeout = self.prepared.limits.timeout;

        // Pre-resolve host credentials from secrets store (async, before blocking task).
        // This decrypts the secrets once so the sync http_request() host function
        // can inject them without needing async access.
        let credential_user_id = ctx.user_id.clone();
        let resolution = resolve_host_credentials(
            &self.capabilities,
            self.secrets_store.as_deref(),
            &credential_user_id,
            self.role_lookup.as_deref(),
            self.oauth_refresh.as_ref(),
        )
        .await;

        // Fail closed: if any *required* credential is missing, refuse to
        // execute the tool. The previous behavior of silently dropping
        // unresolved credentials let a malicious or misconfigured tool
        // issue requests without the credentials it declared, which can
        // exfiltrate user context to an unauthenticated endpoint.
        // Tools that genuinely want graceful degradation must mark the
        // mapping `optional = true` in their capabilities manifest.
        if !resolution.missing_required.is_empty() {
            return Err(ToolError::ExecutionFailed(format!(
                "WASM tool '{}' requires credentials that are not configured for user '{}': {}. \
                 Configure the missing credentials with `ironclaw secrets set` and re-run the tool.",
                self.name(),
                credential_user_id,
                resolution.missing_required.join(", ")
            )));
        }
        let host_credentials = resolution.resolved;

        // Serialize context for WASM
        let context_json = serde_json::to_string(ctx).ok();

        // Clone what we need for the blocking task
        let runtime = Arc::clone(&self.runtime);
        let prepared = Arc::clone(&self.prepared);
        let capabilities = self.capabilities.clone();
        let description = self.description.clone();
        let schemas = self.schemas.clone();
        let discovery_summary = self.discovery_summary.clone();
        let credentials = self.credentials.clone();

        // Execute in blocking task with timeout
        let result = tokio::time::timeout(timeout, async move {
            let wrapper = WasmToolWrapper {
                runtime,
                prepared,
                capabilities,
                description,
                schemas,
                discovery_summary,
                credentials,
                secrets_store: None, // Not needed in blocking task
                role_lookup: None,
                oauth_refresh: None, // Already used above for pre-refresh
                http_interceptor: self.http_interceptor.clone(),
            };

            tokio::task::spawn_blocking(move || {
                wrapper.execute_sync(params, context_json, host_credentials)
            })
            .await
            .map_err(|e| WasmError::ExecutionPanicked(e.to_string()))?
        })
        .await;

        let duration = start.elapsed();

        match result {
            Ok(Ok((result_json, logs))) => {
                // Emit collected logs
                for log in logs {
                    match log.level {
                        LogLevel::Trace => tracing::trace!(target: "wasm_tool", "{}", log.message),
                        LogLevel::Debug => tracing::debug!(target: "wasm_tool", "{}", log.message),
                        LogLevel::Info => tracing::info!(target: "wasm_tool", "{}", log.message),
                        LogLevel::Warn => tracing::warn!(target: "wasm_tool", "{}", log.message),
                        LogLevel::Error => tracing::error!(target: "wasm_tool", "{}", log.message),
                    }
                }

                // Parse result JSON
                let result: serde_json::Value = serde_json::from_str(&result_json)
                    .unwrap_or(serde_json::Value::String(result_json));

                Ok(ToolOutput::success(result, duration))
            }
            Ok(Err(wasm_err)) => Err(wasm_err.into()),
            Err(_) => Err(WasmError::Timeout(timeout).into()),
        }
    }

    fn requires_sanitization(&self) -> bool {
        // WASM tools always require sanitization, they're untrusted by definition
        true
    }

    fn estimated_duration(&self, _params: &serde_json::Value) -> Option<Duration> {
        // Use the timeout as a conservative estimate
        Some(self.prepared.limits.timeout)
    }

    fn webhook_capability(&self) -> Option<crate::tools::wasm::WebhookCapability> {
        self.capabilities.webhook.clone()
    }
}

impl std::fmt::Debug for WasmToolWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmToolWrapper")
            .field("name", &self.prepared.name)
            .field("description", &self.description)
            .field("limits", &self.prepared.limits)
            .finish()
    }
}

/// Pre-resolved credentials returned from [`resolve_host_credentials`].
///
/// Resolution runs once per tool execution in an async context, before the
/// synchronous WASM host function spawns. Optional mappings may be skipped
/// when unavailable; required mappings that fail to resolve are tracked in
/// `missing_required` so the caller can fail closed before execution starts.
/// Failing closed prevents a malicious or misconfigured tool from issuing
/// requests that silently borrow another scope's secrets or exfiltrate user
/// context to an unauthenticated endpoint.
struct HostCredentialsResolution {
    resolved: Vec<ResolvedHostCredential>,
    missing_required: Vec<String>,
}

#[cfg(test)]
impl HostCredentialsResolution {
    fn is_empty(&self) -> bool {
        self.resolved.is_empty()
    }

    fn len(&self) -> usize {
        self.resolved.len()
    }
}

#[cfg(test)]
impl std::ops::Index<usize> for HostCredentialsResolution {
    type Output = ResolvedHostCredential;
    fn index(&self, idx: usize) -> &Self::Output {
        &self.resolved[idx]
    }
}

async fn resolve_host_credentials(
    capabilities: &Capabilities,
    store: Option<&(dyn SecretsStore + Send + Sync)>,
    user_id: &str,
    role_lookup: Option<&dyn UserStore>,
    oauth_refresh: Option<&OAuthRefreshConfig>,
) -> HostCredentialsResolution {
    let mut missing_required: Vec<String> = Vec::new();

    let store = match store {
        Some(s) => s,
        None => {
            // If tool requires credentials but has no secrets store, every
            // declared *required* credential is unresolvable. Return them
            // as missing so the caller can refuse the execution rather
            // than silently dropping into unauthenticated mode.
            if let Some(http_cap) = &capabilities.http
                && !http_cap.credentials.is_empty()
            {
                tracing::warn!(
                    user_id = %user_id,
                    "WASM tool requires credentials but secrets_store is not configured"
                );
                for mapping in http_cap.credentials.values() {
                    if !mapping.optional {
                        missing_required.push(mapping.secret_name.clone());
                    }
                }
            }
            return HostCredentialsResolution {
                resolved: Vec::new(),
                missing_required,
            };
        }
    };

    let http_cap = match &capabilities.http {
        Some(cap) => cap,
        None => {
            return HostCredentialsResolution {
                resolved: Vec::new(),
                missing_required,
            };
        }
    };

    if http_cap.credentials.is_empty() {
        return HostCredentialsResolution {
            resolved: Vec::new(),
            missing_required,
        };
    }

    let mut resolved = Vec::new();

    for mapping in http_cap.credentials.values() {
        // Skip UrlPath credentials, they're handled by placeholder substitution
        if matches!(
            mapping.location,
            crate::secrets::CredentialLocation::UrlPath { .. }
        ) {
            continue;
        }

        let secret = match resolve_secret_for_runtime(
            store,
            user_id,
            &mapping.secret_name,
            role_lookup,
            oauth_refresh.filter(|config| config.secret_name == mapping.secret_name),
            crate::auth::DefaultFallback::Denied,
        )
        .await
        {
            Ok(secret) => secret,
            Err(error) => {
                tracing::warn!(
                    secret_name = %mapping.secret_name,
                    user_id = %user_id,
                    error = ?error,
                    optional = mapping.optional,
                    "Could not resolve credential for WASM tool"
                );
                if !mapping.optional {
                    missing_required.push(mapping.secret_name.clone());
                }
                continue;
            }
        };

        let mut injected = InjectedCredentials::empty();
        inject_credential(&mut injected, &mapping.location, &secret);

        let signing = match &mapping.location {
            crate::secrets::CredentialLocation::HmacSignedHeader {
                signature_header,
                timestamp_header,
            } => Some(SigningSpec::Hmac(HmacSigning {
                signature_header: signature_header.clone(),
                timestamp_header: timestamp_header.clone(),
            })),
            crate::secrets::CredentialLocation::Eip712SignedHeader {
                domain,
                primary_type,
                structs,
                output_headers,
                output_body_fields,
            } => Some(SigningSpec::Eip712(Eip712Signing {
                domain: domain.clone(),
                primary_type: primary_type.clone(),
                structs: structs.clone(),
                output_headers: output_headers.clone(),
                output_body_fields: output_body_fields.clone(),
            })),
            crate::secrets::CredentialLocation::Nep413SignedHeader {
                recipient_source,
                message_source,
                callback_url_source,
                output_headers,
            } => Some(SigningSpec::Nep413(Nep413Signing {
                recipient_source: recipient_source.clone(),
                message_source: message_source.clone(),
                callback_url_source: callback_url_source.clone(),
                output_headers: output_headers.clone(),
            })),
            _ => None,
        };

        if injected.is_empty() && signing.is_none() {
            continue;
        }

        resolved.push(ResolvedHostCredential {
            secret_name: mapping.secret_name.clone(),
            host_patterns: mapping.host_patterns.clone(),
            path_patterns: mapping.path_patterns.clone(),
            headers: injected.headers,
            query_params: injected.query_params,
            secret_value: secret.expose().to_string(),
            signing,
        });
    }

    if !resolved.is_empty() {
        tracing::debug!(
            count = resolved.len(),
            "Pre-resolved host credentials for WASM tool execution"
        );
    }

    HostCredentialsResolution {
        resolved,
        missing_required,
    }
}

/// Extract the hostname from a URL string.
///
/// Handles `https://host:port/path`, stripping scheme, port, and path.
/// Also handles IPv6 bracket notation like `http://[::1]:8080/path`.
/// Returns None for malformed URLs.
fn extract_host_from_url(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    parsed.host_str().map(|h| {
        h.strip_prefix('[')
            .and_then(|v| v.strip_suffix(']'))
            .unwrap_or(h)
            .to_lowercase()
    })
}

#[cfg(test)]
fn reject_private_ip(url: &str) -> Result<(), String> {
    crate::tools::wasm::reject_private_ip(url)
}

#[cfg(test)]
fn is_private_ip(ip: std::net::IpAddr) -> bool {
    crate::tools::wasm::is_private_ip(ip)
}

fn schema_contains_container_properties(schema: &serde_json::Value) -> bool {
    let has_container = |props: &serde_json::Map<String, serde_json::Value>| {
        props
            .values()
            .any(|prop| schema_declares_type(prop, "array") || schema_declares_type(prop, "object"))
    };

    if schema
        .get("properties")
        .and_then(|p| p.as_object())
        .is_some_and(has_container)
    {
        return true;
    }

    for key in ["allOf", "oneOf", "anyOf"] {
        if let Some(variants) = schema.get(key).and_then(|v| v.as_array())
            && variants.iter().any(|v| {
                v.get("properties")
                    .and_then(|p| p.as_object())
                    .is_some_and(has_container)
            })
        {
            return true;
        }
    }

    false
}

fn schema_declares_type(schema: &serde_json::Value, expected: &str) -> bool {
    match schema.get("type") {
        Some(serde_json::Value::String(t)) => t == expected,
        Some(serde_json::Value::Array(types)) => types.iter().any(|t| t.as_str() == Some(expected)),
        _ => match expected {
            "object" => {
                schema
                    .get("properties")
                    .and_then(|p| p.as_object())
                    .is_some()
                    || schema
                        .get("additionalProperties")
                        .is_some_and(serde_json::Value::is_object)
            }
            "array" => schema.get("items").is_some(),
            _ => false,
        },
    }
}

/// Recursively strip prose-only metadata fields from a schema value.
///
/// Preserves the contract (`type`, `enum`, `const`, `required`,
/// `properties`, `items`, `oneOf`/`anyOf`/`allOf`, `additionalProperties`,
/// `minimum`/`maximum`, etc.) and drops fields that only matter for
/// human consumption (`description`, `title`, `default`, `examples`,
/// `$schema`, `$id`, `$comment`, `format`). The result is the smallest
/// faithful representation of the type contract — useful for embedding
/// schemas in LLM tool definitions where every token costs.
fn strip_schema_metadata(value: &serde_json::Value) -> serde_json::Value {
    const STRIP: &[&str] = &[
        "description",
        "title",
        "default",
        "examples",
        "$schema",
        "$id",
        "$comment",
        "format",
        "deprecated",
        "readOnly",
        "writeOnly",
    ];

    match value {
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                if STRIP.contains(&k.as_str()) {
                    continue;
                }
                out.insert(k.clone(), strip_schema_metadata(v));
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(strip_schema_metadata).collect())
        }
        other => other.clone(),
    }
}

/// Strip metadata from every property value in a `properties` object.
/// Returns the input unchanged if it isn't an object map.
fn strip_props_metadata(value: &serde_json::Value) -> serde_json::Value {
    match value.as_object() {
        Some(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                out.insert(k.clone(), strip_schema_metadata(v));
            }
            serde_json::Value::Object(out)
        }
        None => value.clone(),
    }
}

fn schema_is_typed_property(schema: &serde_json::Value) -> bool {
    matches!(
        schema.get("type"),
        Some(serde_json::Value::String(_)) | Some(serde_json::Value::Array(_))
    ) || schema.get("$ref").is_some()
        || schema.get("anyOf").is_some()
        || schema.get("oneOf").is_some()
        || schema.get("allOf").is_some()
        || schema.get("items").is_some()
        || schema
            .get("properties")
            .and_then(|p| p.as_object())
            .is_some()
        || schema
            .get("additionalProperties")
            .is_some_and(serde_json::Value::is_object)
}

/// Build a hint to attach to a WASM tool error so the LLM can correct
/// its next call without an extra round trip.
///
/// The previous version emitted only `Tip: call tool_info(...)`, which
/// forced the agent to spend an entire turn fetching the schema it
/// already had access to. The agent would read the error, call
/// `tool_info`, get the schema back, and only then retry — burning
/// two iterations to recover from one bad parameter. This version
/// inlines the relevant schema info directly:
///
/// 1. **Tagged-enum / `oneOf` schemas**: extract a compact
///    `action -> [required fields]` map. For google-drive that's
///    ~400 chars / 100 tokens, vs. the ~$0.005-0.01 cost of an
///    extra LLM turn. Tells the LLM exactly which fields it forgot
///    for which action.
/// 2. **Flat schemas**: dump the compact JSON inline if it's under
///    `MAX_INLINE_SCHEMA_BYTES`, otherwise fall through to the old
///    `tool_info` tip as a last-resort fallback for adversarial
///    cases.
///
/// Container hints (arrays/objects need to be JSON literals, not
/// quoted strings) are appended in either case — that's a separate
/// LLM mistake mode that the schema alone doesn't surface.
fn build_tool_usage_hint(tool_name: &str, schema: &serde_json::Value) -> String {
    const MAX_INLINE_SCHEMA_BYTES: usize = 4_000;

    let mut hint = String::new();

    if let Some(map) = extract_action_required_map(schema) {
        hint.push_str(&format!(
            "Required fields per action for {tool_name}: {map}"
        ));
    } else {
        match serde_json::to_string(schema) {
            Ok(json) if json.len() <= MAX_INLINE_SCHEMA_BYTES => {
                hint.push_str(&format!("Schema for {tool_name}: {json}"));
            }
            _ => {
                hint.push_str(&format!(
                    "Tip: call tool_info(name: \"{tool_name}\", include_schema: true) \
                     for the full parameter schema (it was too large to inline)."
                ));
            }
        }
    }

    if schema_contains_container_properties(schema) {
        hint.push_str(
            " For array/object fields, pass native JSON arrays/objects, not quoted JSON strings.",
        );
    }

    hint
}

/// Extract a compact `action -> [required fields]` map from a tagged
/// enum / `oneOf` schema. Returns `None` for schemas without a
/// recognisable `oneOf` of action-discriminated variants.
///
/// Output format: `list_files=[], get_file=[file_id], share_file=[file_id,email]`
///
/// Each variant must have a `properties.action.const` value (the
/// discriminator) and may have a `required` array. The discriminator
/// itself is filtered out of the per-action required list since
/// it's always implicit.
fn extract_action_required_map(schema: &serde_json::Value) -> Option<String> {
    let one_of = schema.get("oneOf")?.as_array()?;
    if one_of.is_empty() {
        return None;
    }

    let mut entries: Vec<String> = Vec::new();
    for variant in one_of {
        let action = variant
            .get("properties")
            .and_then(|p| p.get("action"))
            .and_then(|a| a.get("const"))
            .and_then(|c| c.as_str())?;

        let required: Vec<&str> = variant
            .get("required")
            .and_then(|r| r.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .filter(|s| *s != "action")
                    .collect()
            })
            .unwrap_or_default();

        entries.push(format!("{action}=[{}]", required.join(",")));
    }

    Some(entries.join(", "))
}

/// Methods with side effects require `Content-Length` even when no body is
/// sent — some APIs (e.g. Gmail) return 411 without it. Returns `true` when
/// the host should inject a `Content-Length: 0` header.
fn needs_content_length_zero(method: &str, headers: &HashMap<String, String>) -> bool {
    let mutating = method.eq_ignore_ascii_case("POST")
        || method.eq_ignore_ascii_case("PUT")
        || method.eq_ignore_ascii_case("PATCH")
        || method.eq_ignore_ascii_case("DELETE");
    mutating
        && !headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-length"))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use base64::Engine as _;
    use hmac::Mac as _;
    use sha2::Digest as _;
    use axum::extract::{Form, State};
    use axum::http::HeaderMap;
    use axum::routing::post;
    use axum::{Json, Router};
    use serde_json::json;
    use tokio::net::TcpListener;
    use tokio::sync::{Mutex as AsyncMutex, oneshot};
    use uuid::Uuid;

    use crate::context::JobContext;
    #[cfg(feature = "libsql")]
    use crate::db::{Database, UserRecord, UserStore};
    use crate::secrets::{
        CreateSecretParams, DecryptedSecret, InMemorySecretsStore, Secret, SecretError, SecretRef,
        SecretsStore,
    };

    use crate::testing::credentials::{
        TEST_BEARER_TOKEN_123, TEST_GOOGLE_OAUTH_FRESH, TEST_GOOGLE_OAUTH_LEGACY,
        TEST_GOOGLE_OAUTH_TOKEN, TEST_OAUTH_CLIENT_ID, TEST_OAUTH_CLIENT_SECRET,
        test_secrets_store,
    };
    use crate::tools::tool::Tool;
    use crate::tools::wasm::capabilities::Capabilities;
    use crate::tools::wasm::runtime::{WasmRuntimeConfig, WasmToolRuntime};

    struct RecordingSecretsStore {
        inner: InMemorySecretsStore,
        get_decrypted_lookups: Mutex<Vec<(String, String)>>,
    }

    impl RecordingSecretsStore {
        fn new() -> Self {
            Self {
                inner: test_secrets_store(),
                get_decrypted_lookups: Mutex::new(Vec::new()),
            }
        }

        fn decrypted_lookups(&self) -> Vec<(String, String)> {
            self.get_decrypted_lookups.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl SecretsStore for RecordingSecretsStore {
        async fn create(
            &self,
            user_id: &str,
            params: CreateSecretParams,
        ) -> Result<Secret, SecretError> {
            self.inner.create(user_id, params).await
        }

        async fn get(&self, user_id: &str, name: &str) -> Result<Secret, SecretError> {
            self.inner.get(user_id, name).await
        }

        async fn get_decrypted(
            &self,
            user_id: &str,
            name: &str,
        ) -> Result<DecryptedSecret, SecretError> {
            self.get_decrypted_lookups
                .lock()
                .unwrap()
                .push((user_id.to_string(), name.to_string()));
            self.inner.get_decrypted(user_id, name).await
        }

        async fn exists(&self, user_id: &str, name: &str) -> Result<bool, SecretError> {
            self.inner.exists(user_id, name).await
        }

        async fn any_exist(&self) -> Result<bool, SecretError> {
            self.inner.any_exist().await
        }

        async fn list(&self, user_id: &str) -> Result<Vec<SecretRef>, SecretError> {
            self.inner.list(user_id).await
        }

        async fn delete(&self, user_id: &str, name: &str) -> Result<bool, SecretError> {
            self.inner.delete(user_id, name).await
        }

        async fn record_usage(&self, secret_id: Uuid) -> Result<(), SecretError> {
            self.inner.record_usage(secret_id).await
        }

        async fn is_accessible(
            &self,
            user_id: &str,
            secret_name: &str,
            allowed_secrets: &[String],
        ) -> Result<bool, SecretError> {
            self.inner
                .is_accessible(user_id, secret_name, allowed_secrets)
                .await
        }
    }

    #[cfg(feature = "libsql")]
    async fn test_user_db(user_id: &str, role: &str) -> Arc<dyn Database> {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("admin_fallback_test.db");
        let db = crate::db::libsql::LibSqlBackend::new_local(&db_path)
            .await
            .expect("local libsql db");
        db.run_migrations().await.expect("run migrations");
        db.create_user(&UserRecord {
            id: user_id.to_string(),
            email: None,
            display_name: user_id.to_string(),
            status: "active".to_string(),
            role: role.to_string(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            last_login_at: None,
            created_by: None,
            metadata: serde_json::Value::Null,
        })
        .await
        .expect("create user");
        std::mem::forget(dir);
        let db: Arc<dyn Database> = Arc::new(db);
        db
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct RecordedProxyRequest {
        authorization: Option<String>,
        form: HashMap<String, String>,
    }

    struct MockProxyServer {
        addr: SocketAddr,
        requests: Arc<AsyncMutex<Vec<RecordedProxyRequest>>>,
        shutdown_tx: Option<oneshot::Sender<()>>,
        server_task: Option<tokio::task::JoinHandle<()>>,
    }

    impl MockProxyServer {
        async fn start() -> Self {
            async fn refresh_handler(
                State(requests): State<Arc<AsyncMutex<Vec<RecordedProxyRequest>>>>,
                headers: HeaderMap,
                Form(form): Form<HashMap<String, String>>,
            ) -> Json<serde_json::Value> {
                requests.lock().await.push(RecordedProxyRequest {
                    authorization: headers
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_string),
                    form,
                });
                Json(json!({
                    "access_token": "mock-refreshed-access-token",
                    "refresh_token": "mock-rotated-refresh-token",
                    "expires_in": 3600
                }))
            }

            let requests = Arc::new(AsyncMutex::new(Vec::new()));
            let app = Router::new()
                .route("/oauth/refresh", post(refresh_handler))
                .with_state(Arc::clone(&requests));

            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind mock proxy");
            let addr = listener.local_addr().expect("read mock proxy addr");
            let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
            let server_task = tokio::spawn(async move {
                let _ = axum::serve(listener, app)
                    .with_graceful_shutdown(async {
                        let _ = shutdown_rx.await;
                    })
                    .await;
            });

            Self {
                addr,
                requests,
                shutdown_tx: Some(shutdown_tx),
                server_task: Some(server_task),
            }
        }

        fn base_url(&self) -> String {
            format!("http://{}", self.addr)
        }

        async fn requests(&self) -> Vec<RecordedProxyRequest> {
            self.requests.lock().await.clone()
        }

        async fn shutdown(mut self) {
            if let Some(tx) = self.shutdown_tx.take() {
                let _ = tx.send(());
            }
            if let Some(task) = self.server_task.take() {
                let _ = task.await;
            }
        }
    }

    impl Drop for MockProxyServer {
        fn drop(&mut self) {
            if let Some(tx) = self.shutdown_tx.take() {
                let _ = tx.send(());
            }
            if let Some(task) = self.server_task.take() {
                task.abort();
            }
        }
    }

    #[test]
    fn test_wrapper_creation() {
        // This test verifies the runtime can be created
        // Actual execution tests require a valid WASM component
        let config = WasmRuntimeConfig::for_testing();
        let runtime = Arc::new(WasmToolRuntime::new(config).unwrap());

        // Runtime was created successfully
        assert!(runtime.config().fuel_config.enabled);
    }

    #[tokio::test]
    async fn test_advertised_schema_auto_compacted_from_discovery() {
        let discovery_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "limit": { "type": "integer" }
            },
            "required": ["query"]
        });

        let runtime = Arc::new(WasmToolRuntime::new(WasmRuntimeConfig::for_testing()).unwrap());
        let prepared = runtime
            .prepare("search", b"\0asm\x0d\0\x01\0", None)
            .await
            .unwrap();
        let mut wrapper =
            super::WasmToolWrapper::new(Arc::clone(&runtime), prepared, Capabilities::default());
        wrapper.schemas = super::WasmToolSchemas::new(discovery_schema.clone());
        wrapper.description = "Search documents".to_string();

        // Advertised schema is auto-compacted: keeps required props, drops optional
        assert_eq!(
            wrapper.parameters_schema(),
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                },
                "required": ["query"],
                "additionalProperties": true
            })
        );
        // Discovery retains the full schema
        assert_eq!(wrapper.discovery_schema(), discovery_schema);

        // Compacted schema has typed properties, so no tool_info hint needed
        let schema = wrapper.schema();
        assert!(
            !schema.description.contains("tool_info"),
            "schema().description should not contain tool_info hint when auto-compacted: {}",
            schema.description
        );
    }

    #[tokio::test]
    async fn test_typed_schema_without_required_is_advertised() {
        // Regression test for #1303: when a WASM tool exports a typed schema
        // with no required/enum fields, the advertised schema should still
        // contain the typed properties instead of falling back to permissive {}.
        let discovery_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "limit": { "type": "integer" }
            }
        });

        let runtime = Arc::new(WasmToolRuntime::new(WasmRuntimeConfig::for_testing()).unwrap());
        let prepared = runtime
            .prepare("typed_search", b"\0asm\x0d\0\x01\0", None)
            .await
            .unwrap();
        let mut wrapper =
            super::WasmToolWrapper::new(Arc::clone(&runtime), prepared, Capabilities::default());
        wrapper.schemas = super::WasmToolSchemas::new(discovery_schema.clone());
        wrapper.description = "Typed search tool".to_string();

        let advertised = wrapper.parameters_schema();
        let props = advertised["properties"].as_object().unwrap();

        // Both typed properties should be preserved in the advertised schema
        assert!(
            props.contains_key("query"),
            "advertised schema should contain 'query' property"
        );
        assert!(
            props.contains_key("limit"),
            "advertised schema should contain 'limit' property"
        );
        assert_eq!(props.len(), 2);

        // The schema should NOT be permissive
        assert!(
            !super::WasmToolSchemas::is_permissive_schema(&advertised),
            "advertised schema should not be permissive when typed properties exist"
        );

        // No tool_info hint needed since typed properties are visible
        let schema = wrapper.schema();
        assert!(
            !schema.description.contains("tool_info"),
            "description should not contain tool_info hint: {}",
            schema.description
        );
    }

    #[test]
    fn test_compact_schema_keeps_required_and_enum_properties() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "get", "create"],
                    "description": "The operation"
                },
                "query": { "type": "string" },
                "limit": { "type": "integer" },
                "format": {
                    "type": "string",
                    "enum": ["json", "csv"]
                }
            },
            "required": ["action"]
        });

        let compacted = super::WasmToolSchemas::compact_schema(&schema);
        let props = compacted["properties"].as_object().unwrap();

        // action: required + enum → kept
        assert!(props.contains_key("action"));
        // format: has enum → kept
        assert!(props.contains_key("format"));
        // query: not required, no enum → dropped
        assert!(!props.contains_key("query"));
        // limit: not required, no enum → dropped
        assert!(!props.contains_key("limit"));
        // additionalProperties lets the LLM still pass dropped props
        assert_eq!(compacted["additionalProperties"], true);
        assert_eq!(compacted["required"], serde_json::json!(["action"]));
    }

    #[test]
    fn test_compact_schema_preserves_typed_properties_when_no_required() {
        // No required, no enum, but typed properties → keep all typed props
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "limit": { "type": "integer" }
            }
        });

        let compacted = super::WasmToolSchemas::compact_schema(&schema);
        let props = compacted["properties"].as_object().unwrap();
        assert_eq!(props.len(), 2);
        assert!(props.contains_key("query"));
        assert!(props.contains_key("limit"));
        assert_eq!(compacted["additionalProperties"], true);
    }

    #[test]
    fn test_compact_schema_falls_back_to_permissive_when_no_typed_properties() {
        // Properties with no type info → permissive fallback
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "data": {}
            }
        });

        let compacted = super::WasmToolSchemas::compact_schema(&schema);
        assert!(compacted["properties"].as_object().unwrap().is_empty());
    }

    #[test]
    fn test_compact_schema_handles_no_properties() {
        let schema = serde_json::json!({ "type": "object" });
        let compacted = super::WasmToolSchemas::compact_schema(&schema);
        assert!(compacted["properties"].as_object().unwrap().is_empty());
    }

    /// Regression test: a tagged-enum / `oneOf` schema must preserve
    /// each variant's `required` array so the LLM knows which fields
    /// are mandatory for each `action`. Earlier versions of
    /// `compact_schema` flattened the schema and dropped per-variant
    /// required fields, causing the LLM to construct calls like
    /// `{"action":"get_file"}` without `file_id`, which serde then
    /// rejected at runtime. The current contract: keep `oneOf`,
    /// keep each variant's properties + required, strip prose
    /// metadata (description/default/title) to save tokens.
    #[test]
    fn test_compact_schema_preserves_oneof_variants_and_required() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["action"],
            "oneOf": [
                {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "const": "get_repo" },
                        "owner": { "type": "string", "description": "Repo owner" },
                        "repo": { "type": "string", "description": "Repo name" }
                    },
                    "required": ["action", "owner", "repo"]
                },
                {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "const": "list_issues" },
                        "owner": { "type": "string" },
                        "repo": { "type": "string" },
                        "state": {
                            "type": "string",
                            "enum": ["open", "closed", "all"],
                            "default": "open"
                        }
                    },
                    "required": ["action", "owner", "repo"]
                }
            ]
        });

        let compacted = super::WasmToolSchemas::compact_schema(&schema);

        // The top-level `oneOf` MUST survive — that's the whole point.
        let one_of = compacted["oneOf"]
            .as_array()
            .expect("oneOf should be preserved on the compact schema");
        assert_eq!(one_of.len(), 2);

        // Variant 0 (get_repo) must keep `owner`/`repo` in its required array.
        let v0 = &one_of[0];
        let v0_required: Vec<&str> = v0["required"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(v0_required.contains(&"action"));
        assert!(
            v0_required.contains(&"owner"),
            "variant required array must survive compaction"
        );
        assert!(v0_required.contains(&"repo"));

        // Variant 0 properties must still include owner/repo (typed).
        let v0_props = v0["properties"].as_object().unwrap();
        assert!(v0_props.contains_key("owner"));
        assert!(v0_props.contains_key("repo"));
        // Description must be stripped to save tokens.
        let owner = &v0_props["owner"];
        assert!(
            owner.get("description").is_none(),
            "description should be stripped to save tokens, got: {owner}"
        );
        // But the type must survive.
        assert_eq!(owner["type"], "string");

        // Variant 1 (list_issues) must also keep its required + types.
        let v1 = &one_of[1];
        let v1_required: Vec<&str> = v1["required"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(v1_required.contains(&"owner"));
        assert!(v1_required.contains(&"repo"));

        // The default on `state` should be stripped, but the enum survives.
        let state = &v1["properties"]["state"];
        assert!(state.get("default").is_none(), "default should be stripped");
        assert!(state.get("enum").is_some(), "enum must survive");

        // Top-level required and additionalProperties carry through.
        assert_eq!(compacted["required"], serde_json::json!(["action"]));
        assert_eq!(compacted["additionalProperties"], true);
    }

    /// Specific repro for the google-drive bug: a schemars-derived
    /// `oneOf` schema with one variant that has `file_id` as required.
    /// After compaction, the `file_id` requirement must still be visible
    /// to the LLM, otherwise it will call `{"action":"get_file"}` and
    /// serde will reject it.
    #[test]
    fn test_compact_schema_preserves_file_id_required_for_get_file() {
        let schema = serde_json::json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": "GoogleDriveAction",
            "oneOf": [
                {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "const": "list_files" },
                        "query": { "type": ["string", "null"], "default": null },
                        "page_size": { "type": "integer", "default": 25 }
                    },
                    "required": ["action"]
                },
                {
                    "type": "object",
                    "description": "Get file metadata.",
                    "properties": {
                        "action": { "type": "string", "const": "get_file" },
                        "file_id": { "description": "The file ID.", "type": "string" }
                    },
                    "required": ["action", "file_id"]
                }
            ]
        });

        let compacted = super::WasmToolSchemas::compact_schema(&schema);
        let one_of = compacted["oneOf"].as_array().unwrap();

        // Find the get_file variant.
        let get_file = one_of
            .iter()
            .find(|v| {
                v["properties"]
                    .get("action")
                    .and_then(|a| a.get("const"))
                    .and_then(|c| c.as_str())
                    == Some("get_file")
            })
            .expect("compact schema should still contain get_file variant");

        let required: Vec<&str> = get_file["required"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(
            required.contains(&"file_id"),
            "get_file's required array MUST still contain file_id after compaction; \
             without this the LLM constructs malformed calls — got required={:?}",
            required
        );

        // The `$schema` and `title` should be dropped from the top-level
        // (they're noise to the LLM).
        assert!(compacted.get("$schema").is_none());
        assert!(compacted.get("title").is_none());

        // And the per-variant `description` should also be stripped.
        assert!(
            get_file.get("description").is_none(),
            "variant-level description should be stripped"
        );
    }

    #[test]
    fn test_capabilities_default() {
        let caps = Capabilities::default();
        assert!(caps.workspace_read.is_none());
        assert!(caps.http.is_none());
        assert!(caps.tool_invoke.is_none());
        assert!(caps.secrets.is_none());
    }

    #[test]
    fn test_extract_host_from_url() {
        use crate::tools::wasm::wrapper::extract_host_from_url;

        assert_eq!(
            extract_host_from_url("https://www.googleapis.com/calendar/v3/events"),
            Some("www.googleapis.com".to_string())
        );
        assert_eq!(
            extract_host_from_url("https://api.example.com:443/v1/foo"),
            Some("api.example.com".to_string())
        );
        assert_eq!(
            extract_host_from_url("http://localhost:8080/test?q=1"),
            Some("localhost".to_string())
        );
        assert_eq!(
            extract_host_from_url("https://user:pass@host.com/path"),
            Some("host.com".to_string())
        );
        assert_eq!(extract_host_from_url("ftp://bad.com"), None);
        assert_eq!(extract_host_from_url("not a url"), None);
        // IPv6
        assert_eq!(
            extract_host_from_url("http://[::1]:8080/test"),
            Some("::1".to_string())
        );
        assert_eq!(
            extract_host_from_url("https://[2001:db8::1]/path"),
            Some("2001:db8::1".to_string())
        );
    }

    #[test]
    fn test_inject_host_credentials_bearer() {
        use crate::tools::wasm::wrapper::{ResolvedHostCredential, StoreData};
        use std::collections::HashMap;

        let host_credentials = vec![ResolvedHostCredential {
            secret_name: "google_token".to_string(),
            host_patterns: vec!["www.googleapis.com".to_string()],
            path_patterns: vec![],
            headers: {
                let mut h = HashMap::new();
                h.insert(
                    "Authorization".to_string(),
                    format!("Bearer {TEST_BEARER_TOKEN_123}"),
                );
                h
            },
            query_params: HashMap::new(),
            secret_value: TEST_BEARER_TOKEN_123.to_string(),
            signing: None,
        }];

        let store_data = StoreData::new(
            1024 * 1024,
            Capabilities::default(),
            HashMap::new(),
            host_credentials,
        );

        // Should inject for matching host
        let mut headers = HashMap::new();
        let mut url = "https://www.googleapis.com/calendar/v3/events".to_string();
        store_data.inject_host_credentials("www.googleapis.com", "GET", &mut None, &mut headers, &mut url);
        assert_eq!(
            headers.get("Authorization"),
            Some(&format!("Bearer {TEST_BEARER_TOKEN_123}"))
        );

        // Should not inject for non-matching host
        let mut headers2 = HashMap::new();
        let mut url2 = "https://other.com/api".to_string();
        store_data.inject_host_credentials("other.com", "GET", &mut None, &mut headers2, &mut url2);
        assert!(!headers2.contains_key("Authorization"));
    }

    #[test]
    fn test_inject_host_credentials_path_scoped() {
        use crate::tools::wasm::wrapper::{ResolvedHostCredential, StoreData};
        use std::collections::HashMap;

        let host_credentials = vec![ResolvedHostCredential {
            secret_name: "scoped_token".to_string(),
            host_patterns: vec!["api.example.com".to_string()],
            path_patterns: vec!["/api/v1".to_string()],
            headers: {
                let mut h = HashMap::new();
                h.insert(
                    "Authorization".to_string(),
                    "Bearer scoped-token".to_string(),
                );
                h
            },
            query_params: HashMap::new(),
            secret_value: "scoped-token".to_string(),
            signing: None,
        }];

        let store_data = StoreData::new(
            1024 * 1024,
            Capabilities::default(),
            HashMap::new(),
            host_credentials,
        );

        // Should inject for matching host + matching path
        let mut headers = HashMap::new();
        let mut url = "https://api.example.com/api/v1/users".to_string();
        store_data.inject_host_credentials("api.example.com", "GET", &mut None, &mut headers, &mut url);
        assert_eq!(
            headers.get("Authorization"),
            Some(&"Bearer scoped-token".to_string())
        );

        // Should NOT inject for matching host + non-matching path
        let mut headers2 = HashMap::new();
        let mut url2 = "https://api.example.com/other/endpoint".to_string();
        store_data.inject_host_credentials("api.example.com", "GET", &mut None, &mut headers2, &mut url2);
        assert!(!headers2.contains_key("Authorization"));

        // Should NOT inject for matching host + prefix-boundary attack
        let mut headers3 = HashMap::new();
        let mut url3 = "https://api.example.com/api/v1-malicious".to_string();
        store_data.inject_host_credentials("api.example.com", "GET", &mut None, &mut headers3, &mut url3);
        assert!(!headers3.contains_key("Authorization"));
    }

    #[test]
    fn test_inject_host_credentials_different_paths_same_host() {
        use crate::tools::wasm::wrapper::{ResolvedHostCredential, StoreData};
        use std::collections::HashMap;

        let host_credentials = vec![
            ResolvedHostCredential {
                secret_name: "v1_token".to_string(),
                host_patterns: vec!["api.example.com".to_string()],
                path_patterns: vec!["/api/v1".to_string()],
                headers: {
                    let mut h = HashMap::new();
                    h.insert("Authorization".to_string(), "Bearer v1-token".to_string());
                    h
                },
                query_params: HashMap::new(),
                secret_value: "v1-token".to_string(),
                signing: None,
            },
            ResolvedHostCredential {
                secret_name: "v2_token".to_string(),
                host_patterns: vec!["api.example.com".to_string()],
                path_patterns: vec!["/api/v2".to_string()],
                headers: {
                    let mut h = HashMap::new();
                    h.insert("Authorization".to_string(), "Bearer v2-token".to_string());
                    h
                },
                query_params: HashMap::new(),
                secret_value: "v2-token".to_string(),
                signing: None,
            },
        ];

        let store_data = StoreData::new(
            1024 * 1024,
            Capabilities::default(),
            HashMap::new(),
            host_credentials,
        );

        // /api/v1 path gets v1 token
        let mut headers = HashMap::new();
        let mut url = "https://api.example.com/api/v1/users".to_string();
        store_data.inject_host_credentials("api.example.com", "GET", &mut None, &mut headers, &mut url);
        assert_eq!(
            headers.get("Authorization"),
            Some(&"Bearer v1-token".to_string())
        );

        // /api/v2 path gets v2 token
        let mut headers2 = HashMap::new();
        let mut url2 = "https://api.example.com/api/v2/data".to_string();
        store_data.inject_host_credentials("api.example.com", "GET", &mut None, &mut headers2, &mut url2);
        assert_eq!(
            headers2.get("Authorization"),
            Some(&"Bearer v2-token".to_string())
        );

        // Unscoped path gets neither
        let mut headers3 = HashMap::new();
        let mut url3 = "https://api.example.com/other".to_string();
        store_data.inject_host_credentials("api.example.com", "GET", &mut None, &mut headers3, &mut url3);
        assert!(!headers3.contains_key("Authorization"));
    }

    #[test]
    fn test_inject_host_credentials_most_specific_path_wins() {
        // Regression for Firat round-4 (#3125963270): overlapping mappings
        // had no deterministic precedence and the HashMap-backed WASM
        // credential source could pick the wrong winner. With ordered
        // specificity, the longest matching path prefix must always win any
        // conflicting header, regardless of insertion order.
        use crate::tools::wasm::wrapper::{ResolvedHostCredential, StoreData};
        use std::collections::HashMap;

        fn global() -> ResolvedHostCredential {
            ResolvedHostCredential {
                secret_name: "global_token".to_string(),
                host_patterns: vec!["api.example.com".to_string()],
                path_patterns: vec![],
                headers: {
                    let mut h = HashMap::new();
                    h.insert("Authorization".to_string(), "Bearer GLOBAL".to_string());
                    h
                },
                query_params: HashMap::new(),
                secret_value: "GLOBAL".to_string(),
                signing: None,
            }
        }
        fn scoped() -> ResolvedHostCredential {
            ResolvedHostCredential {
                secret_name: "write_token".to_string(),
                host_patterns: vec!["api.example.com".to_string()],
                path_patterns: vec!["/api/v1/write".to_string()],
                headers: {
                    let mut h = HashMap::new();
                    h.insert("Authorization".to_string(), "Bearer WRITE".to_string());
                    h
                },
                query_params: HashMap::new(),
                secret_value: "WRITE".to_string(),
                signing: None,
            }
        }

        for creds in [
            // "wrong" order: global first, specific second
            vec![global(), scoped()],
            // reverse order: specific first, global second — must still
            // yield the same WRITE winner under specificity sort
            vec![scoped(), global()],
        ] {
            let store = StoreData::new(1024 * 1024, Capabilities::default(), HashMap::new(), creds);
            let mut headers = HashMap::new();
            let mut url = "https://api.example.com/api/v1/write/foo".to_string();
            store.inject_host_credentials("api.example.com", "GET", &mut None, &mut headers, &mut url);
            assert_eq!(
                headers.get("Authorization"),
                Some(&"Bearer WRITE".to_string()),
                "most-specific path_patterns (/api/v1/write) must win over global credential"
            );
        }
    }

    #[test]
    fn test_inject_host_credentials_query_params() {
        use crate::tools::wasm::wrapper::{ResolvedHostCredential, StoreData};
        use std::collections::HashMap;

        let host_credentials = vec![ResolvedHostCredential {
            secret_name: "api_key".to_string(),
            host_patterns: vec!["api.example.com".to_string()],
            path_patterns: vec![],
            headers: HashMap::new(),
            query_params: {
                let mut q = HashMap::new();
                q.insert("api_key".to_string(), "secret123".to_string());
                q
            },
            secret_value: "secret123".to_string(),
            signing: None,
        }];

        let store_data = StoreData::new(
            1024 * 1024,
            Capabilities::default(),
            HashMap::new(),
            host_credentials,
        );

        let mut headers = HashMap::new();
        let mut url = "https://api.example.com/v1/data".to_string();
        store_data.inject_host_credentials("api.example.com", "GET", &mut None, &mut headers, &mut url);
        assert!(url.contains("api_key=secret123"));
        assert!(url.contains('?'));
    }

    #[test]
    fn test_redact_credentials_includes_host_credentials() {
        use crate::tools::wasm::wrapper::{ResolvedHostCredential, StoreData};
        use std::collections::HashMap;

        let host_credentials = vec![ResolvedHostCredential {
            secret_name: "super_secret".to_string(),
            host_patterns: vec!["api.example.com".to_string()],
            path_patterns: vec![],
            headers: HashMap::new(),
            query_params: HashMap::new(),
            secret_value: "super-secret-token".to_string(),
            signing: None,
        }];

        let store_data = StoreData::new(
            1024 * 1024,
            Capabilities::default(),
            HashMap::new(),
            host_credentials,
        );

        let text = "Error: request to https://api.example.com?key=super-secret-token failed";
        let redacted = store_data.redact_credentials(text);
        assert!(!redacted.contains("super-secret-token"));
        assert!(redacted.contains("[REDACTED:host_credential]"));
    }

    fn expected_hmac_signature(
        base64_secret: &str,
        timestamp: &str,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
    ) -> String {
        let key = base64::engine::general_purpose::URL_SAFE
            .decode(base64_secret)
            .expect("test secret decodes as url-safe base64");
        let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(&key)
            .expect("hmac accepts arbitrary key length");
        mac.update(timestamp.as_bytes());
        mac.update(method.as_bytes());
        mac.update(path.as_bytes());
        if let Some(b) = body {
            mac.update(b);
        }
        base64::engine::general_purpose::URL_SAFE.encode(mac.finalize().into_bytes())
    }

    const TEST_HMAC_SECRET_B64: &str = "MDEyMzQ1Njc4OWFiY2RlZg==";

    fn hmac_credential(
        host: &str,
        path_patterns: Vec<String>,
        sig_header: &str,
        ts_header: &str,
    ) -> super::ResolvedHostCredential {
        super::ResolvedHostCredential {
            secret_name: "polymarket_api_secret".to_string(),
            host_patterns: vec![host.to_string()],
            path_patterns,
            headers: HashMap::new(),
            query_params: HashMap::new(),
            secret_value: TEST_HMAC_SECRET_B64.to_string(),
            signing: Some(super::SigningSpec::Hmac(super::HmacSigning {
                signature_header: sig_header.to_string(),
                timestamp_header: ts_header.to_string(),
            })),
        }
    }

    #[test]
    fn test_inject_host_credentials_hmac_basic_get() {
        use crate::tools::wasm::wrapper::StoreData;

        let creds = vec![hmac_credential(
            "clob.polymarket.com",
            vec![],
            "POLY-SIGNATURE",
            "POLY-TIMESTAMP",
        )];
        let store_data =
            StoreData::new(1024 * 1024, Capabilities::default(), HashMap::new(), creds);

        let mut headers = HashMap::new();
        let mut url = "https://clob.polymarket.com/orders".to_string();
        store_data.inject_host_credentials(
            "clob.polymarket.com",
            "GET",
            &mut None,
            &mut headers,
            &mut url,
        );

        let timestamp = headers
            .get("POLY-TIMESTAMP")
            .expect("timestamp header was emitted")
            .clone();
        assert!(timestamp.parse::<u64>().is_ok(), "timestamp must be unix seconds");

        let signature = headers
            .get("POLY-SIGNATURE")
            .expect("signature header was emitted");
        let expected = expected_hmac_signature(
            TEST_HMAC_SECRET_B64,
            &timestamp,
            "GET",
            "/orders",
            None,
        );
        assert_eq!(signature, &expected);
    }

    #[test]
    fn test_inject_host_credentials_hmac_with_body() {
        use crate::tools::wasm::wrapper::StoreData;

        let creds = vec![hmac_credential(
            "clob.polymarket.com",
            vec![],
            "POLY-SIGNATURE",
            "POLY-TIMESTAMP",
        )];
        let store_data =
            StoreData::new(1024 * 1024, Capabilities::default(), HashMap::new(), creds);

        let body = br#"{"price":"0.5","side":"BUY","size":"10"}"#;
        let mut headers = HashMap::new();
        let mut url = "https://clob.polymarket.com/order".to_string();
        let mut body_slot: Option<Vec<u8>> = Some(body.to_vec());
        store_data.inject_host_credentials(
            "clob.polymarket.com",
            "POST",
            &mut body_slot,
            &mut headers,
            &mut url,
        );

        let timestamp = headers.get("POLY-TIMESTAMP").unwrap().clone();
        let signature = headers.get("POLY-SIGNATURE").unwrap();
        let expected = expected_hmac_signature(
            TEST_HMAC_SECRET_B64,
            &timestamp,
            "POST",
            "/order",
            Some(body),
        );
        assert_eq!(signature, &expected);

        let signature_no_body = expected_hmac_signature(
            TEST_HMAC_SECRET_B64,
            &timestamp,
            "POST",
            "/order",
            None,
        );
        assert_ne!(
            signature, &signature_no_body,
            "body bytes must change the resulting signature"
        );
    }

    #[test]
    fn test_inject_host_credentials_hmac_path_scoped_match() {
        use crate::tools::wasm::wrapper::StoreData;

        let creds = vec![hmac_credential(
            "api.example.com",
            vec!["/v1/private".to_string()],
            "X-SIG",
            "X-TS",
        )];
        let store_data =
            StoreData::new(1024 * 1024, Capabilities::default(), HashMap::new(), creds);

        let mut headers = HashMap::new();
        let mut url = "https://api.example.com/v1/private/orders".to_string();
        store_data.inject_host_credentials(
            "api.example.com",
            "GET",
            &mut None,
            &mut headers,
            &mut url,
        );

        assert!(headers.contains_key("X-SIG"));
        assert!(headers.contains_key("X-TS"));
    }

    #[test]
    fn test_inject_host_credentials_hmac_path_scoped_miss() {
        use crate::tools::wasm::wrapper::StoreData;

        let creds = vec![hmac_credential(
            "api.example.com",
            vec!["/v1/private".to_string()],
            "X-SIG",
            "X-TS",
        )];
        let store_data =
            StoreData::new(1024 * 1024, Capabilities::default(), HashMap::new(), creds);

        let mut headers = HashMap::new();
        let mut url = "https://api.example.com/v1/public/orders".to_string();
        store_data.inject_host_credentials(
            "api.example.com",
            "GET",
            &mut None,
            &mut headers,
            &mut url,
        );

        assert!(!headers.contains_key("X-SIG"));
        assert!(!headers.contains_key("X-TS"));
    }

    #[test]
    fn test_inject_host_credentials_hmac_host_mismatch() {
        use crate::tools::wasm::wrapper::StoreData;

        let creds = vec![hmac_credential(
            "api.example.com",
            vec![],
            "X-SIG",
            "X-TS",
        )];
        let store_data =
            StoreData::new(1024 * 1024, Capabilities::default(), HashMap::new(), creds);

        let mut headers = HashMap::new();
        let mut url = "https://other.com/anything".to_string();
        store_data.inject_host_credentials(
            "other.com",
            "GET",
            &mut None,
            &mut headers,
            &mut url,
        );

        assert!(headers.is_empty());
    }

    #[test]
    fn test_inject_host_credentials_hmac_invalid_secret_skips() {
        use crate::tools::wasm::wrapper::{ResolvedHostCredential, StoreData};

        let creds = vec![ResolvedHostCredential {
            secret_name: "polymarket_api_secret".to_string(),
            host_patterns: vec!["clob.polymarket.com".to_string()],
            path_patterns: vec![],
            headers: HashMap::new(),
            query_params: HashMap::new(),
            secret_value: "this is not valid url-safe base64!!!".to_string(),
            signing: Some(super::SigningSpec::Hmac(super::HmacSigning {
                signature_header: "POLY-SIGNATURE".to_string(),
                timestamp_header: "POLY-TIMESTAMP".to_string(),
            })),
        }];
        let store_data =
            StoreData::new(1024 * 1024, Capabilities::default(), HashMap::new(), creds);

        let mut headers = HashMap::new();
        let mut url = "https://clob.polymarket.com/orders".to_string();
        store_data.inject_host_credentials(
            "clob.polymarket.com",
            "GET",
            &mut None,
            &mut headers,
            &mut url,
        );

        assert!(!headers.contains_key("POLY-SIGNATURE"));
        assert!(!headers.contains_key("POLY-TIMESTAMP"));
    }

    #[test]
    fn test_inject_host_credentials_hmac_combined_with_static_bearer() {
        use crate::tools::wasm::wrapper::{ResolvedHostCredential, StoreData};

        let api_key_cred = ResolvedHostCredential {
            secret_name: "polymarket_api_key".to_string(),
            host_patterns: vec!["clob.polymarket.com".to_string()],
            path_patterns: vec![],
            headers: {
                let mut h = HashMap::new();
                h.insert("POLY-API-KEY".to_string(), "the-api-key-id".to_string());
                h
            },
            query_params: HashMap::new(),
            secret_value: "the-api-key-id".to_string(),
            signing: None,
        };

        let creds = vec![
            api_key_cred,
            hmac_credential(
                "clob.polymarket.com",
                vec![],
                "POLY-SIGNATURE",
                "POLY-TIMESTAMP",
            ),
        ];
        let store_data =
            StoreData::new(1024 * 1024, Capabilities::default(), HashMap::new(), creds);

        let mut headers = HashMap::new();
        let mut url = "https://clob.polymarket.com/orders".to_string();
        store_data.inject_host_credentials(
            "clob.polymarket.com",
            "GET",
            &mut None,
            &mut headers,
            &mut url,
        );

        assert_eq!(
            headers.get("POLY-API-KEY"),
            Some(&"the-api-key-id".to_string())
        );
        assert!(headers.contains_key("POLY-SIGNATURE"));
        assert!(headers.contains_key("POLY-TIMESTAMP"));
    }

    #[test]
    fn test_resolved_host_credential_debug_redacts_hmac_secret() {
        use crate::tools::wasm::wrapper::ResolvedHostCredential;

        let cred = ResolvedHostCredential {
            secret_name: "polymarket_api_secret".to_string(),
            host_patterns: vec!["clob.polymarket.com".to_string()],
            path_patterns: vec![],
            headers: HashMap::new(),
            query_params: HashMap::new(),
            secret_value: TEST_HMAC_SECRET_B64.to_string(),
            signing: Some(super::SigningSpec::Hmac(super::HmacSigning {
                signature_header: "POLY-SIGNATURE".to_string(),
                timestamp_header: "POLY-TIMESTAMP".to_string(),
            })),
        };

        let dbg = format!("{cred:?}");
        assert!(dbg.contains("POLY-SIGNATURE"));
        assert!(dbg.contains("POLY-TIMESTAMP"));
        assert!(dbg.contains("[REDACTED]"));
        assert!(!dbg.contains(TEST_HMAC_SECRET_B64));
    }

    #[tokio::test]
    async fn test_resolve_host_credentials_hmac_creates_signing_field() {
        use crate::secrets::{
            CreateSecretParams, CredentialLocation, CredentialMapping, SecretsStore,
        };
        use crate::tools::wasm::capabilities::HttpCapability;
        use crate::tools::wasm::wrapper::resolve_host_credentials;

        let store = test_secrets_store();
        store
            .create(
                "user1",
                CreateSecretParams::new("polymarket_api_secret", TEST_HMAC_SECRET_B64),
            )
            .await
            .unwrap();

        let mut credentials = HashMap::new();
        credentials.insert(
            "polymarket_api_secret".to_string(),
            CredentialMapping {
                secret_name: "polymarket_api_secret".to_string(),
                location: CredentialLocation::HmacSignedHeader {
                    signature_header: "POLY-SIGNATURE".to_string(),
                    timestamp_header: "POLY-TIMESTAMP".to_string(),
                },
                host_patterns: vec!["clob.polymarket.com".to_string()],
                path_patterns: Vec::new(),
                optional: false,
            },
        );

        let caps = Capabilities {
            http: Some(HttpCapability {
                credentials,
                ..Default::default()
            }),
            ..Default::default()
        };

        let result = resolve_host_credentials(&caps, Some(&store), "user1", None, None).await;
        assert_eq!(result.len(), 1);
        assert!(result[0].headers.is_empty());
        assert!(result[0].query_params.is_empty());
        let signing = result[0]
            .signing
            .as_ref()
            .expect("signing field populated for HmacSignedHeader location");
        match signing {
            super::SigningSpec::Hmac(spec) => {
                assert_eq!(spec.signature_header, "POLY-SIGNATURE");
                assert_eq!(spec.timestamp_header, "POLY-TIMESTAMP");
            }
            other => panic!("expected SigningSpec::Hmac, got {:?}", std::mem::discriminant(other)),
        }
    }

    const TEST_ETH_PRIVATE_KEY: &str =
        "0x0123456789012345678901234567890123456789012345678901234567890123";
    const TEST_NEAR_ED25519_SEED: &str =
        "11111111111111111111111111111111";

    fn polymarket_clobauth_struct() -> crate::secrets::Eip712StructDef {
        use crate::secrets::{Eip712StructDef, Eip712TypedField, FieldSource};
        Eip712StructDef {
            name: "ClobAuth".to_string(),
            fields: vec![
                Eip712TypedField {
                    name: "address".to_string(),
                    type_name: "address".to_string(),
                    source: FieldSource::SignerAddress,
                },
                Eip712TypedField {
                    name: "timestamp".to_string(),
                    type_name: "string".to_string(),
                    source: FieldSource::RequestTimestampSecs,
                },
                Eip712TypedField {
                    name: "nonce".to_string(),
                    type_name: "uint256".to_string(),
                    source: FieldSource::Literal {
                        value: "0".to_string(),
                    },
                },
                Eip712TypedField {
                    name: "message".to_string(),
                    type_name: "string".to_string(),
                    source: FieldSource::Literal {
                        value: "This message attests that I control the given wallet"
                            .to_string(),
                    },
                },
            ],
        }
    }

    fn polymarket_clobauth_signing() -> super::Eip712Signing {
        use crate::secrets::{Eip712Domain, HeaderOutput, OutputSource};
        super::Eip712Signing {
            domain: Eip712Domain {
                name: "ClobAuthDomain".to_string(),
                version: "1".to_string(),
                chain_id: 137,
                verifying_contract: None,
            },
            primary_type: "ClobAuth".to_string(),
            structs: vec![polymarket_clobauth_struct()],
            output_headers: vec![
                HeaderOutput {
                    header_name: "POLY_ADDRESS".to_string(),
                    value: OutputSource::SignerAddress,
                },
                HeaderOutput {
                    header_name: "POLY_SIGNATURE".to_string(),
                    value: OutputSource::SignatureHex,
                },
                HeaderOutput {
                    header_name: "POLY_TIMESTAMP".to_string(),
                    value: OutputSource::RequestTimestampSecs,
                },
                HeaderOutput {
                    header_name: "POLY_NONCE".to_string(),
                    value: OutputSource::Literal {
                        value: "0".to_string(),
                    },
                },
            ],
            output_body_fields: vec![],
        }
    }

    #[test]
    fn test_inject_host_credentials_eip712_polymarket_clobauth_recovers_signer() {
        use crate::tools::wasm::wrapper::{ResolvedHostCredential, StoreData};
        use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};

        let signing_key = super::parse_secp256k1_secret(TEST_ETH_PRIVATE_KEY)
            .expect("test private key parses");
        let expected_address = super::derive_eth_address(&signing_key);
        let expected_address_hex = format!("0x{}", hex::encode(expected_address));

        let cred = ResolvedHostCredential {
            secret_name: "polymarket_eth_key".to_string(),
            host_patterns: vec!["clob.polymarket.com".to_string()],
            path_patterns: vec![],
            headers: HashMap::new(),
            query_params: HashMap::new(),
            secret_value: TEST_ETH_PRIVATE_KEY.to_string(),
            signing: Some(super::SigningSpec::Eip712(polymarket_clobauth_signing())),
        };

        let store_data = StoreData::new(
            1024 * 1024,
            Capabilities::default(),
            HashMap::new(),
            vec![cred],
        );

        let mut headers = HashMap::new();
        let mut url = "https://clob.polymarket.com/auth-api-key".to_string();
        store_data.inject_host_credentials(
            "clob.polymarket.com",
            "POST",
            &mut None,
            &mut headers,
            &mut url,
        );

        assert_eq!(headers.get("POLY_ADDRESS"), Some(&expected_address_hex));
        assert_eq!(headers.get("POLY_NONCE"), Some(&"0".to_string()));

        let timestamp_str = headers
            .get("POLY_TIMESTAMP")
            .expect("timestamp header emitted")
            .clone();
        timestamp_str
            .parse::<u64>()
            .expect("timestamp must be unix seconds");

        let signature_hex = headers
            .get("POLY_SIGNATURE")
            .expect("signature header emitted")
            .clone();
        let sig_bytes = hex::decode(signature_hex.trim_start_matches("0x"))
            .expect("signature is hex");
        assert_eq!(sig_bytes.len(), 65, "signature must be 65 bytes (r||s||v)");
        let v = sig_bytes[64];
        assert!(v == 27 || v == 28, "v must be 27 or 28, got {v}");

        let signature = Signature::from_slice(&sig_bytes[..64]).expect("valid signature");
        let recovery_id = RecoveryId::try_from(v - 27).expect("valid recovery id");

        let domain_sep =
            super::compute_eip712_domain_separator(&polymarket_clobauth_signing().domain)
                .expect("domain separator");
        let primary = polymarket_clobauth_struct();
        let type_hash =
            super::keccak256(super::encode_eip712_type_string(&primary).as_bytes());
        let mut struct_buf: Vec<u8> = Vec::new();
        struct_buf.extend_from_slice(&type_hash);
        struct_buf
            .extend_from_slice(&super::encode_eip712_field_value("address", &expected_address_hex).unwrap());
        struct_buf
            .extend_from_slice(&super::encode_eip712_field_value("string", &timestamp_str).unwrap());
        struct_buf.extend_from_slice(&super::encode_eip712_field_value("uint256", "0").unwrap());
        struct_buf.extend_from_slice(
            &super::encode_eip712_field_value(
                "string",
                "This message attests that I control the given wallet",
            )
            .unwrap(),
        );
        let struct_hash = super::keccak256(&struct_buf);

        let mut final_buf: Vec<u8> = Vec::with_capacity(2 + 32 + 32);
        final_buf.extend_from_slice(&[0x19u8, 0x01u8]);
        final_buf.extend_from_slice(&domain_sep);
        final_buf.extend_from_slice(&struct_hash);
        let final_hash = super::keccak256(&final_buf);

        let recovered_pk = VerifyingKey::recover_from_prehash(&final_hash, &signature, recovery_id)
            .expect("recover from signature");
        let recovered_point = recovered_pk.to_encoded_point(false);
        let recovered_pk_bytes = &recovered_point.as_bytes()[1..];
        let h = super::keccak256(recovered_pk_bytes);
        let mut recovered_addr = [0u8; 20];
        recovered_addr.copy_from_slice(&h[12..]);
        assert_eq!(
            recovered_addr, expected_address,
            "signature must recover to the signer address"
        );
    }

    #[test]
    fn test_inject_host_credentials_eip712_invalid_secret_skips() {
        use crate::tools::wasm::wrapper::{ResolvedHostCredential, StoreData};

        let cred = ResolvedHostCredential {
            secret_name: "polymarket_eth_key".to_string(),
            host_patterns: vec!["clob.polymarket.com".to_string()],
            path_patterns: vec![],
            headers: HashMap::new(),
            query_params: HashMap::new(),
            secret_value: "not a real hex private key".to_string(),
            signing: Some(super::SigningSpec::Eip712(polymarket_clobauth_signing())),
        };
        let store_data = StoreData::new(
            1024 * 1024,
            Capabilities::default(),
            HashMap::new(),
            vec![cred],
        );

        let mut headers = HashMap::new();
        let mut url = "https://clob.polymarket.com/auth-api-key".to_string();
        store_data.inject_host_credentials(
            "clob.polymarket.com",
            "POST",
            &mut None,
            &mut headers,
            &mut url,
        );

        assert!(!headers.contains_key("POLY_ADDRESS"));
        assert!(!headers.contains_key("POLY_SIGNATURE"));
        assert!(!headers.contains_key("POLY_TIMESTAMP"));
    }

    #[test]
    fn test_inject_host_credentials_eip712_multi_struct_schema_skips() {
        use crate::secrets::{Eip712StructDef, Eip712TypedField, FieldSource};
        use crate::tools::wasm::wrapper::{ResolvedHostCredential, StoreData};

        let mut spec = polymarket_clobauth_signing();
        spec.structs.push(Eip712StructDef {
            name: "Other".to_string(),
            fields: vec![Eip712TypedField {
                name: "x".to_string(),
                type_name: "uint256".to_string(),
                source: FieldSource::Literal {
                    value: "1".to_string(),
                },
            }],
        });

        let cred = ResolvedHostCredential {
            secret_name: "polymarket_eth_key".to_string(),
            host_patterns: vec!["clob.polymarket.com".to_string()],
            path_patterns: vec![],
            headers: HashMap::new(),
            query_params: HashMap::new(),
            secret_value: TEST_ETH_PRIVATE_KEY.to_string(),
            signing: Some(super::SigningSpec::Eip712(spec)),
        };
        let store_data = StoreData::new(
            1024 * 1024,
            Capabilities::default(),
            HashMap::new(),
            vec![cred],
        );

        let mut headers = HashMap::new();
        let mut url = "https://clob.polymarket.com/auth-api-key".to_string();
        store_data.inject_host_credentials(
            "clob.polymarket.com",
            "POST",
            &mut None,
            &mut headers,
            &mut url,
        );

        assert!(headers.is_empty());
    }

    #[test]
    fn test_nep413_borsh_prefix_matches_spec() {
        let prefix: u32 = 2_147_484_061;
        let mut buf: Vec<u8> = Vec::new();
        borsh::BorshSerialize::serialize(&prefix, &mut buf).unwrap();
        assert_eq!(buf, vec![0x9D, 0x01, 0x00, 0x80]);
    }

    fn nep413_signing(recipient: &str, message: &str) -> super::Nep413Signing {
        use crate::secrets::{FieldSource, HeaderOutput, OutputSource};
        super::Nep413Signing {
            recipient_source: FieldSource::Literal {
                value: recipient.to_string(),
            },
            message_source: FieldSource::Literal {
                value: message.to_string(),
            },
            callback_url_source: None,
            output_headers: vec![
                HeaderOutput {
                    header_name: "X-NEAR-PUBLIC-KEY".to_string(),
                    value: OutputSource::SignerPublicKey,
                },
                HeaderOutput {
                    header_name: "X-NEAR-SIGNATURE".to_string(),
                    value: OutputSource::SignatureBase64,
                },
                HeaderOutput {
                    header_name: "X-NEAR-NONCE".to_string(),
                    value: OutputSource::RequestRandomNonceB64,
                },
            ],
        }
    }

    #[test]
    fn test_inject_host_credentials_nep413_signature_verifies() {
        use crate::tools::wasm::wrapper::{ResolvedHostCredential, StoreData};
        use ed25519_dalek::Verifier;

        let cred = ResolvedHostCredential {
            secret_name: "near_account_key".to_string(),
            host_patterns: vec!["api.trezu.example".to_string()],
            path_patterns: vec![],
            headers: HashMap::new(),
            query_params: HashMap::new(),
            secret_value: TEST_NEAR_ED25519_SEED.to_string(),
            signing: Some(super::SigningSpec::Nep413(nep413_signing(
                "trezu.near",
                "auth-challenge-2026",
            ))),
        };
        let store_data = StoreData::new(
            1024 * 1024,
            Capabilities::default(),
            HashMap::new(),
            vec![cred],
        );

        let mut headers = HashMap::new();
        let mut url = "https://api.trezu.example/login".to_string();
        store_data.inject_host_credentials(
            "api.trezu.example",
            "POST",
            &mut None,
            &mut headers,
            &mut url,
        );

        let pk_str = headers
            .get("X-NEAR-PUBLIC-KEY")
            .expect("public key header emitted")
            .clone();
        assert!(pk_str.starts_with("ed25519:"));
        let pk_b58 = pk_str.trim_start_matches("ed25519:");
        let pk_bytes = bs58::decode(pk_b58).into_vec().unwrap();
        let verifying_key =
            ed25519_dalek::VerifyingKey::from_bytes(pk_bytes.as_slice().try_into().unwrap())
                .unwrap();

        let sig_b64 = headers
            .get("X-NEAR-SIGNATURE")
            .expect("signature header emitted")
            .clone();
        let sig_bytes = base64::engine::general_purpose::URL_SAFE
            .decode(sig_b64.as_bytes())
            .expect("signature is url-safe base64");
        let signature =
            ed25519_dalek::Signature::from_bytes(sig_bytes.as_slice().try_into().unwrap());

        let nonce_b64 = headers
            .get("X-NEAR-NONCE")
            .expect("nonce header emitted")
            .clone();
        let nonce_bytes_vec = base64::engine::general_purpose::URL_SAFE
            .decode(nonce_b64.as_bytes())
            .unwrap();
        let nonce_bytes: [u8; 32] = nonce_bytes_vec.as_slice().try_into().unwrap();

        let payload = super::Nep413Payload {
            message: "auth-challenge-2026".to_string(),
            nonce: nonce_bytes,
            recipient: "trezu.near".to_string(),
            callback_url: None,
        };

        let mut serialized: Vec<u8> = Vec::new();
        let prefix: u32 = 2_147_484_061;
        borsh::BorshSerialize::serialize(&prefix, &mut serialized).unwrap();
        borsh::BorshSerialize::serialize(&payload, &mut serialized).unwrap();
        let hash = sha2::Sha256::digest(&serialized);

        verifying_key
            .verify(&hash, &signature)
            .expect("signature must verify against the published public key");
    }

    #[test]
    fn test_inject_host_credentials_nep413_invalid_secret_skips() {
        use crate::tools::wasm::wrapper::{ResolvedHostCredential, StoreData};

        let cred = ResolvedHostCredential {
            secret_name: "near_account_key".to_string(),
            host_patterns: vec!["api.trezu.example".to_string()],
            path_patterns: vec![],
            headers: HashMap::new(),
            query_params: HashMap::new(),
            secret_value: "definitely not a base58 ed25519 key !@#".to_string(),
            signing: Some(super::SigningSpec::Nep413(nep413_signing(
                "trezu.near",
                "challenge",
            ))),
        };
        let store_data = StoreData::new(
            1024 * 1024,
            Capabilities::default(),
            HashMap::new(),
            vec![cred],
        );

        let mut headers = HashMap::new();
        let mut url = "https://api.trezu.example/login".to_string();
        store_data.inject_host_credentials(
            "api.trezu.example",
            "POST",
            &mut None,
            &mut headers,
            &mut url,
        );

        assert!(!headers.contains_key("X-NEAR-PUBLIC-KEY"));
        assert!(!headers.contains_key("X-NEAR-SIGNATURE"));
        assert!(!headers.contains_key("X-NEAR-NONCE"));
    }

    fn hyperliquid_agent_signing() -> super::Eip712Signing {
        use crate::secrets::{
            BodyJsonOutput, BodyValue, BytesPart, Eip712Domain, Eip712StructDef,
            Eip712TypedField, FieldSource, HeaderOutput, OutputSource,
        };
        super::Eip712Signing {
            domain: Eip712Domain {
                name: "Exchange".to_string(),
                version: "1".to_string(),
                chain_id: 1337,
                verifying_contract: Some("0x0000000000000000000000000000000000000000".to_string()),
            },
            primary_type: "Agent".to_string(),
            structs: vec![Eip712StructDef {
                name: "Agent".to_string(),
                fields: vec![
                    Eip712TypedField {
                        name: "source".to_string(),
                        type_name: "string".to_string(),
                        source: FieldSource::Literal {
                            value: "a".to_string(),
                        },
                    },
                    Eip712TypedField {
                        name: "connectionId".to_string(),
                        type_name: "bytes32".to_string(),
                        source: FieldSource::Bytes32Keccak256OfBytes {
                            parts: vec![
                                BytesPart::BodyFieldMsgpack {
                                    path: "action".to_string(),
                                },
                                BytesPart::BodyFieldBeU64 {
                                    path: "nonce".to_string(),
                                },
                                BytesPart::BodyFieldEthAddressOptionalPrefixed {
                                    path: "vaultAddress".to_string(),
                                },
                            ],
                        },
                    },
                ],
            }],
            output_headers: vec![HeaderOutput {
                header_name: "X-WALLET-ADDRESS".to_string(),
                value: OutputSource::SignerAddress,
            }],
            output_body_fields: vec![
                BodyJsonOutput {
                    json_path: "signature.r".to_string(),
                    value: BodyValue::SignatureRHex,
                },
                BodyJsonOutput {
                    json_path: "signature.s".to_string(),
                    value: BodyValue::SignatureSHex,
                },
                BodyJsonOutput {
                    json_path: "signature.v".to_string(),
                    value: BodyValue::SignatureV,
                },
            ],
        }
    }

    #[test]
    fn test_inject_host_credentials_eip712_hyperliquid_signature_in_body_recovers_signer() {
        use crate::tools::wasm::wrapper::{ResolvedHostCredential, StoreData};
        use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
        use serde::Serialize as _;

        let signing_key = super::parse_secp256k1_secret(TEST_ETH_PRIVATE_KEY)
            .expect("test private key parses");
        let expected_address = super::derive_eth_address(&signing_key);
        let expected_address_hex = format!("0x{}", hex::encode(expected_address));

        let body_json = serde_json::json!({
            "action": { "type": "order", "grouping": "na" },
            "nonce": 1_700_000_000_000u64,
            "vaultAddress": serde_json::Value::Null,
        });
        let body_bytes = serde_json::to_vec(&body_json).unwrap();

        let cred = ResolvedHostCredential {
            secret_name: "hyperliquid_eth_key".to_string(),
            host_patterns: vec!["api.hyperliquid.xyz".to_string()],
            path_patterns: vec![],
            headers: HashMap::new(),
            query_params: HashMap::new(),
            secret_value: TEST_ETH_PRIVATE_KEY.to_string(),
            signing: Some(super::SigningSpec::Eip712(hyperliquid_agent_signing())),
        };
        let store_data = StoreData::new(
            1024 * 1024,
            Capabilities::default(),
            HashMap::new(),
            vec![cred],
        );

        let mut headers = HashMap::new();
        let mut url = "https://api.hyperliquid.xyz/exchange".to_string();
        let mut body_slot: Option<Vec<u8>> = Some(body_bytes.clone());
        store_data.inject_host_credentials(
            "api.hyperliquid.xyz",
            "POST",
            &mut body_slot,
            &mut headers,
            &mut url,
        );

        assert_eq!(headers.get("X-WALLET-ADDRESS"), Some(&expected_address_hex));

        let mutated_body = body_slot.as_ref().expect("body retained after signing");
        let mutated_json: serde_json::Value =
            serde_json::from_slice(mutated_body).expect("body still valid json");

        let r_hex = mutated_json["signature"]["r"]
            .as_str()
            .expect("signature.r is a string")
            .to_string();
        let s_hex = mutated_json["signature"]["s"]
            .as_str()
            .expect("signature.s is a string")
            .to_string();
        let v_num = mutated_json["signature"]["v"]
            .as_u64()
            .expect("signature.v is a number") as u8;
        assert!(v_num == 27 || v_num == 28, "v must be 27 or 28, got {v_num}");

        let r_bytes = hex::decode(r_hex.trim_start_matches("0x")).unwrap();
        let s_bytes = hex::decode(s_hex.trim_start_matches("0x")).unwrap();
        assert_eq!(r_bytes.len(), 32);
        assert_eq!(s_bytes.len(), 32);

        let mut sig_64 = [0u8; 64];
        sig_64[..32].copy_from_slice(&r_bytes);
        sig_64[32..].copy_from_slice(&s_bytes);
        let signature = Signature::from_slice(&sig_64).unwrap();
        let recovery_id = RecoveryId::try_from(v_num - 27).unwrap();

        let mut connection_input: Vec<u8> = Vec::new();
        let action_value = &body_json["action"];
        action_value
            .serialize(
                &mut rmp_serde::Serializer::new(&mut connection_input).with_struct_map(),
            )
            .unwrap();
        connection_input.extend_from_slice(&1_700_000_000_000u64.to_be_bytes());
        connection_input.push(0x00);
        let connection_id_bytes = super::keccak256(&connection_input);
        let connection_id_hex = format!("0x{}", hex::encode(connection_id_bytes));

        let primary = &hyperliquid_agent_signing().structs[0].clone();
        let type_string = super::encode_eip712_type_string(primary);
        let type_hash = super::keccak256(type_string.as_bytes());

        let mut struct_buf: Vec<u8> = Vec::new();
        struct_buf.extend_from_slice(&type_hash);
        struct_buf.extend_from_slice(&super::encode_eip712_field_value("string", "a").unwrap());
        struct_buf.extend_from_slice(
            &super::encode_eip712_field_value("bytes32", &connection_id_hex).unwrap(),
        );
        let struct_hash = super::keccak256(&struct_buf);

        let domain_separator =
            super::compute_eip712_domain_separator(&hyperliquid_agent_signing().domain).unwrap();
        let mut final_buf: Vec<u8> = Vec::new();
        final_buf.extend_from_slice(&[0x19u8, 0x01u8]);
        final_buf.extend_from_slice(&domain_separator);
        final_buf.extend_from_slice(&struct_hash);
        let final_hash = super::keccak256(&final_buf);

        let recovered_pk =
            VerifyingKey::recover_from_prehash(&final_hash, &signature, recovery_id)
                .expect("recover from signature");
        let recovered_point = recovered_pk.to_encoded_point(false);
        let recovered_pk_bytes = &recovered_point.as_bytes()[1..];
        let h = super::keccak256(recovered_pk_bytes);
        let mut recovered_addr = [0u8; 20];
        recovered_addr.copy_from_slice(&h[12..]);
        assert_eq!(
            recovered_addr, expected_address,
            "Hyperliquid signature must recover to the signer address"
        );

        let original_action = body_json.get("action").unwrap();
        assert_eq!(mutated_json.get("action"), Some(original_action),
            "the action field must not be modified by signing");
    }

    #[test]
    fn test_inject_host_credentials_eip712_body_field_msgpack_no_body_skips() {
        use crate::secrets::{
            BodyJsonOutput, BodyValue, BytesPart, Eip712Domain, Eip712StructDef,
            Eip712TypedField, FieldSource, HeaderOutput, OutputSource,
        };
        use crate::tools::wasm::wrapper::{ResolvedHostCredential, StoreData};

        let cred = ResolvedHostCredential {
            secret_name: "hyperliquid_eth_key".to_string(),
            host_patterns: vec!["api.hyperliquid.xyz".to_string()],
            path_patterns: vec![],
            headers: HashMap::new(),
            query_params: HashMap::new(),
            secret_value: TEST_ETH_PRIVATE_KEY.to_string(),
            signing: Some(super::SigningSpec::Eip712(super::Eip712Signing {
                domain: Eip712Domain {
                    name: "Exchange".to_string(),
                    version: "1".to_string(),
                    chain_id: 1337,
                    verifying_contract: None,
                },
                primary_type: "Agent".to_string(),
                structs: vec![Eip712StructDef {
                    name: "Agent".to_string(),
                    fields: vec![Eip712TypedField {
                        name: "connectionId".to_string(),
                        type_name: "bytes32".to_string(),
                        source: FieldSource::Bytes32Keccak256OfBytes {
                            parts: vec![BytesPart::BodyFieldMsgpack {
                                path: "action".to_string(),
                            }],
                        },
                    }],
                }],
                output_headers: vec![HeaderOutput {
                    header_name: "X-WALLET-ADDRESS".to_string(),
                    value: OutputSource::SignerAddress,
                }],
                output_body_fields: vec![BodyJsonOutput {
                    json_path: "signature".to_string(),
                    value: BodyValue::SignatureHex,
                }],
            })),
        };
        let store_data = StoreData::new(
            1024 * 1024,
            Capabilities::default(),
            HashMap::new(),
            vec![cred],
        );

        let mut headers = HashMap::new();
        let mut url = "https://api.hyperliquid.xyz/exchange".to_string();
        store_data.inject_host_credentials(
            "api.hyperliquid.xyz",
            "POST",
            &mut None,
            &mut headers,
            &mut url,
        );

        assert!(!headers.contains_key("X-WALLET-ADDRESS"));
    }

    #[tokio::test]
    async fn test_resolve_host_credentials_eip712_creates_signing_field() {
        use crate::secrets::{
            CreateSecretParams, CredentialLocation, CredentialMapping, Eip712Domain,
            Eip712StructDef, Eip712TypedField, FieldSource, HeaderOutput, OutputSource,
            SecretsStore,
        };
        use crate::tools::wasm::capabilities::HttpCapability;
        use crate::tools::wasm::wrapper::resolve_host_credentials;

        let store = test_secrets_store();
        store
            .create(
                "user1",
                CreateSecretParams::new("polymarket_eth_key", TEST_ETH_PRIVATE_KEY),
            )
            .await
            .unwrap();

        let mut credentials = HashMap::new();
        credentials.insert(
            "polymarket_eth_key".to_string(),
            CredentialMapping {
                secret_name: "polymarket_eth_key".to_string(),
                location: CredentialLocation::Eip712SignedHeader {
                    domain: Eip712Domain {
                        name: "ClobAuthDomain".to_string(),
                        version: "1".to_string(),
                        chain_id: 137,
                        verifying_contract: None,
                    },
                    primary_type: "ClobAuth".to_string(),
                    structs: vec![Eip712StructDef {
                        name: "ClobAuth".to_string(),
                        fields: vec![Eip712TypedField {
                            name: "address".to_string(),
                            type_name: "address".to_string(),
                            source: FieldSource::SignerAddress,
                        }],
                    }],
                    output_headers: vec![HeaderOutput {
                        header_name: "POLY_ADDRESS".to_string(),
                        value: OutputSource::SignerAddress,
                    }],
                    output_body_fields: vec![],
                },
                host_patterns: vec!["clob.polymarket.com".to_string()],
                path_patterns: Vec::new(),
                optional: false,
            },
        );

        let caps = Capabilities {
            http: Some(HttpCapability {
                credentials,
                ..Default::default()
            }),
            ..Default::default()
        };

        let result = resolve_host_credentials(&caps, Some(&store), "user1", None, None).await;
        assert_eq!(result.len(), 1);
        assert!(result[0].headers.is_empty());
        match result[0].signing.as_ref().unwrap() {
            super::SigningSpec::Eip712(spec) => {
                assert_eq!(spec.domain.name, "ClobAuthDomain");
                assert_eq!(spec.primary_type, "ClobAuth");
            }
            _ => panic!("expected SigningSpec::Eip712"),
        }
    }

    #[tokio::test]
    async fn test_resolve_host_credentials_nep413_creates_signing_field() {
        use crate::secrets::{
            CreateSecretParams, CredentialLocation, CredentialMapping, FieldSource, HeaderOutput,
            OutputSource, SecretsStore,
        };
        use crate::tools::wasm::capabilities::HttpCapability;
        use crate::tools::wasm::wrapper::resolve_host_credentials;

        let store = test_secrets_store();
        store
            .create(
                "user1",
                CreateSecretParams::new("near_account_key", TEST_NEAR_ED25519_SEED),
            )
            .await
            .unwrap();

        let mut credentials = HashMap::new();
        credentials.insert(
            "near_account_key".to_string(),
            CredentialMapping {
                secret_name: "near_account_key".to_string(),
                location: CredentialLocation::Nep413SignedHeader {
                    recipient_source: FieldSource::Literal {
                        value: "trezu.near".to_string(),
                    },
                    message_source: FieldSource::Literal {
                        value: "auth".to_string(),
                    },
                    callback_url_source: None,
                    output_headers: vec![HeaderOutput {
                        header_name: "X-NEAR-PUBLIC-KEY".to_string(),
                        value: OutputSource::SignerPublicKey,
                    }],
                },
                host_patterns: vec!["api.trezu.example".to_string()],
                path_patterns: Vec::new(),
                optional: false,
            },
        );

        let caps = Capabilities {
            http: Some(HttpCapability {
                credentials,
                ..Default::default()
            }),
            ..Default::default()
        };

        let result = resolve_host_credentials(&caps, Some(&store), "user1", None, None).await;
        assert_eq!(result.len(), 1);
        match result[0].signing.as_ref().unwrap() {
            super::SigningSpec::Nep413(spec) => {
                assert_eq!(spec.output_headers.len(), 1);
                assert_eq!(spec.output_headers[0].header_name, "X-NEAR-PUBLIC-KEY");
            }
            _ => panic!("expected SigningSpec::Nep413"),
        }
    }

    const POLYMARKET_CLOB_CAPABILITIES_JSON: &str = r#"{
      "version": "0.1.0",
      "wit_version": "0.3.0",
      "description": "Polymarket signed trading.",
      "http": {
        "allowlist": [
          { "host": "clob.polymarket.com", "path_prefix": "/", "methods": ["GET","POST","DELETE"] }
        ],
        "credentials": {
          "polymarket_l1_clobauth": {
            "secret_name": "polymarket_l1_private_key",
            "host_patterns": ["clob.polymarket.com"],
            "path_patterns": ["/auth/api-key"],
            "location": {
              "type": "eip712_signed_header",
              "domain": { "name": "ClobAuthDomain", "version": "1", "chain_id": 137 },
              "primary_type": "ClobAuth",
              "structs": [
                {
                  "name": "ClobAuth",
                  "fields": [
                    { "name": "address", "type": "address",
                      "source": { "source": "signer_address" } },
                    { "name": "timestamp", "type": "string",
                      "source": { "source": "request_timestamp_secs" } },
                    { "name": "nonce", "type": "uint256",
                      "source": { "source": "literal", "value": "0" } },
                    { "name": "message", "type": "string",
                      "source": { "source": "literal", "value": "This message attests that I control the given wallet" } }
                  ]
                }
              ],
              "output_headers": [
                { "header_name": "POLY_ADDRESS",   "value": { "source": "signer_address" } },
                { "header_name": "POLY_SIGNATURE", "value": { "source": "signature_hex" } },
                { "header_name": "POLY_TIMESTAMP", "value": { "source": "request_timestamp_secs" } },
                { "header_name": "POLY_NONCE",     "value": { "source": "literal", "value": "0" } }
              ]
            }
          },
          "polymarket_l2_hmac": {
            "secret_name": "polymarket_l2_secret",
            "host_patterns": ["clob.polymarket.com"],
            "location": {
              "type": "hmac_signed_header",
              "signature_header": "POLY_SIGNATURE",
              "timestamp_header": "POLY_TIMESTAMP"
            }
          },
          "polymarket_l2_api_key": {
            "secret_name": "polymarket_l2_api_key",
            "host_patterns": ["clob.polymarket.com"],
            "location": { "type": "header", "name": "POLY_API_KEY" }
          },
          "polymarket_l2_passphrase": {
            "secret_name": "polymarket_l2_passphrase",
            "host_patterns": ["clob.polymarket.com"],
            "location": { "type": "header", "name": "POLY_PASSPHRASE" }
          },
          "polymarket_l2_address": {
            "secret_name": "polymarket_l1_address",
            "host_patterns": ["clob.polymarket.com"],
            "location": { "type": "header", "name": "POLY_ADDRESS" }
          }
        },
        "rate_limit": { "requests_per_minute": 120, "requests_per_hour": 3600 },
        "timeout_secs": 30
      },
      "secrets": {
        "allowed_names": [
          "polymarket_l1_private_key", "polymarket_l1_address",
          "polymarket_l2_api_key", "polymarket_l2_secret", "polymarket_l2_passphrase"
        ]
      }
    }"#;

    async fn polymarket_dummy_secrets_store() -> (
        crate::secrets::InMemorySecretsStore,
        String,
        String,
    ) {
        use crate::secrets::{CreateSecretParams, SecretsStore};

        let signing_key = super::parse_secp256k1_secret(TEST_ETH_PRIVATE_KEY).unwrap();
        let derived_address = super::derive_eth_address(&signing_key);
        let derived_address_hex = format!("0x{}", hex::encode(derived_address));
        let l2_secret_b64 = "dGVzdC1obWFjLXNlY3JldA==";

        let store = test_secrets_store();
        let user_id = "default";
        for (name, value) in [
            ("polymarket_l1_private_key", TEST_ETH_PRIVATE_KEY),
            ("polymarket_l1_address", derived_address_hex.as_str()),
            ("polymarket_l2_api_key", "dummy-api-key"),
            ("polymarket_l2_secret", l2_secret_b64),
            ("polymarket_l2_passphrase", "dummy-passphrase"),
        ] {
            store
                .create(user_id, CreateSecretParams::new(name, value))
                .await
                .unwrap();
        }

        (store, derived_address_hex, l2_secret_b64.to_string())
    }

    #[tokio::test]
    async fn polymarket_clob_l2_request_assembles_with_signed_headers_end_to_end() {
        use crate::tools::wasm::capabilities_schema::CapabilitiesFile;
        use crate::tools::wasm::wrapper::{StoreData, resolve_host_credentials};

        let caps_file = CapabilitiesFile::from_json(POLYMARKET_CLOB_CAPABILITIES_JSON)
            .expect("polymarket-clob capabilities parse");
        let runtime_caps = caps_file.to_capabilities();
        let (store, expected_address, l2_secret_b64) = polymarket_dummy_secrets_store().await;

        let resolved =
            resolve_host_credentials(&runtime_caps, Some(&store), "default", None, None).await;
        assert_eq!(resolved.len(), 5, "five credential mappings should resolve");

        let store_data = StoreData::new(
            1024 * 1024,
            runtime_caps,
            HashMap::new(),
            resolved.resolved,
        );

        let mut headers = HashMap::new();
        let mut url = "https://clob.polymarket.com/data/orders".to_string();
        let mut body: Option<Vec<u8>> = None;
        store_data.inject_host_credentials(
            "clob.polymarket.com",
            "GET",
            &mut body,
            &mut headers,
            &mut url,
        );

        assert_eq!(headers.get("POLY_API_KEY").map(String::as_str), Some("dummy-api-key"));
        assert_eq!(
            headers.get("POLY_PASSPHRASE").map(String::as_str),
            Some("dummy-passphrase")
        );
        assert_eq!(
            headers.get("POLY_ADDRESS"),
            Some(&expected_address),
            "POLY_ADDRESS must match the derived wallet address"
        );

        let timestamp = headers.get("POLY_TIMESTAMP").expect("timestamp emitted").clone();
        timestamp.parse::<u64>().expect("timestamp is unix seconds");

        let signature = headers.get("POLY_SIGNATURE").expect("signature emitted");
        let expected_sig = expected_hmac_signature(
            &l2_secret_b64,
            &timestamp,
            "GET",
            "/data/orders",
            None,
        );
        assert_eq!(
            signature, &expected_sig,
            "L2 HMAC signature must match independent recomputation"
        );

        assert!(
            !headers.contains_key("POLY_NONCE"),
            "L2 path must not emit POLY_NONCE (that is L1 only)"
        );
    }

    #[tokio::test]
    async fn polymarket_clob_l1_register_key_request_assembles_with_eip712_headers_end_to_end() {
        use crate::tools::wasm::capabilities_schema::CapabilitiesFile;
        use crate::tools::wasm::wrapper::{StoreData, resolve_host_credentials};
        use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};

        let caps_file = CapabilitiesFile::from_json(POLYMARKET_CLOB_CAPABILITIES_JSON).unwrap();
        let runtime_caps = caps_file.to_capabilities();
        let (store, expected_address_hex, _) = polymarket_dummy_secrets_store().await;

        let resolved =
            resolve_host_credentials(&runtime_caps, Some(&store), "default", None, None).await;

        let store_data = StoreData::new(
            1024 * 1024,
            runtime_caps,
            HashMap::new(),
            resolved.resolved,
        );

        let mut headers = HashMap::new();
        let mut url = "https://clob.polymarket.com/auth/api-key".to_string();
        let mut body: Option<Vec<u8>> = Some(b"{}".to_vec());
        store_data.inject_host_credentials(
            "clob.polymarket.com",
            "POST",
            &mut body,
            &mut headers,
            &mut url,
        );

        assert_eq!(headers.get("POLY_NONCE").map(String::as_str), Some("0"));
        assert_eq!(
            headers.get("POLY_ADDRESS"),
            Some(&expected_address_hex),
            "POLY_ADDRESS on L1 must match the EIP-712 signer-derived address"
        );

        let timestamp = headers.get("POLY_TIMESTAMP").unwrap().clone();
        timestamp.parse::<u64>().expect("timestamp is unix seconds");
        let signature_hex = headers.get("POLY_SIGNATURE").expect("signature emitted").clone();

        let sig_bytes = hex::decode(signature_hex.trim_start_matches("0x")).unwrap();
        assert_eq!(sig_bytes.len(), 65, "EIP-712 signature must be 65 bytes (r||s||v)");
        let v = sig_bytes[64];
        let signature = Signature::from_slice(&sig_bytes[..64]).unwrap();
        let recovery_id = RecoveryId::try_from(v - 27).unwrap();

        let domain = crate::secrets::Eip712Domain {
            name: "ClobAuthDomain".to_string(),
            version: "1".to_string(),
            chain_id: 137,
            verifying_contract: None,
        };
        let domain_sep = super::compute_eip712_domain_separator(&domain).unwrap();

        let primary = crate::secrets::Eip712StructDef {
            name: "ClobAuth".to_string(),
            fields: vec![
                crate::secrets::Eip712TypedField {
                    name: "address".to_string(),
                    type_name: "address".to_string(),
                    source: crate::secrets::FieldSource::SignerAddress,
                },
                crate::secrets::Eip712TypedField {
                    name: "timestamp".to_string(),
                    type_name: "string".to_string(),
                    source: crate::secrets::FieldSource::RequestTimestampSecs,
                },
                crate::secrets::Eip712TypedField {
                    name: "nonce".to_string(),
                    type_name: "uint256".to_string(),
                    source: crate::secrets::FieldSource::Literal {
                        value: "0".to_string(),
                    },
                },
                crate::secrets::Eip712TypedField {
                    name: "message".to_string(),
                    type_name: "string".to_string(),
                    source: crate::secrets::FieldSource::Literal {
                        value: "This message attests that I control the given wallet".to_string(),
                    },
                },
            ],
        };
        let type_hash = super::keccak256(super::encode_eip712_type_string(&primary).as_bytes());
        let mut struct_buf: Vec<u8> = Vec::new();
        struct_buf.extend_from_slice(&type_hash);
        struct_buf.extend_from_slice(&super::encode_eip712_field_value("address", &expected_address_hex).unwrap());
        struct_buf.extend_from_slice(&super::encode_eip712_field_value("string", &timestamp).unwrap());
        struct_buf.extend_from_slice(&super::encode_eip712_field_value("uint256", "0").unwrap());
        struct_buf.extend_from_slice(
            &super::encode_eip712_field_value("string", "This message attests that I control the given wallet").unwrap(),
        );
        let struct_hash = super::keccak256(&struct_buf);

        let mut final_buf: Vec<u8> = Vec::new();
        final_buf.extend_from_slice(&[0x19u8, 0x01u8]);
        final_buf.extend_from_slice(&domain_sep);
        final_buf.extend_from_slice(&struct_hash);
        let final_hash = super::keccak256(&final_buf);

        let recovered_pk =
            VerifyingKey::recover_from_prehash(&final_hash, &signature, recovery_id).unwrap();
        let recovered_point = recovered_pk.to_encoded_point(false);
        let recovered_pk_bytes = &recovered_point.as_bytes()[1..];
        let h = super::keccak256(recovered_pk_bytes);
        let mut recovered_addr = [0u8; 20];
        recovered_addr.copy_from_slice(&h[12..]);
        let recovered_hex = format!("0x{}", hex::encode(recovered_addr));
        assert_eq!(
            recovered_hex, expected_address_hex,
            "EIP-712 signature must recover to the dummy wallet address"
        );
    }

    #[tokio::test]
    async fn test_resolve_host_credentials_no_store() {
        use crate::tools::wasm::wrapper::resolve_host_credentials;

        let caps = Capabilities::default();
        let result = resolve_host_credentials(&caps, None, "user1", None, None).await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_resolve_host_credentials_no_http_cap() {
        use crate::tools::wasm::wrapper::resolve_host_credentials;

        let store = test_secrets_store();

        let caps = Capabilities::default();
        let result = resolve_host_credentials(&caps, Some(&store), "user1", None, None).await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_resolve_host_credentials_bearer() {
        use crate::secrets::{
            CreateSecretParams, CredentialLocation, CredentialMapping, SecretsStore,
        };
        use crate::tools::wasm::capabilities::HttpCapability;
        use crate::tools::wasm::wrapper::resolve_host_credentials;

        let store = test_secrets_store();

        store
            .create(
                "user1",
                CreateSecretParams::new("google_oauth_token", TEST_GOOGLE_OAUTH_TOKEN),
            )
            .await
            .unwrap();

        let mut credentials = HashMap::new();
        credentials.insert(
            "google_oauth_token".to_string(),
            CredentialMapping {
                secret_name: "google_oauth_token".to_string(),
                location: CredentialLocation::AuthorizationBearer,
                host_patterns: vec!["www.googleapis.com".to_string()],
                path_patterns: Vec::new(),
                optional: false,
            },
        );

        let caps = Capabilities {
            http: Some(HttpCapability {
                credentials,
                ..Default::default()
            }),
            ..Default::default()
        };

        let result = resolve_host_credentials(&caps, Some(&store), "user1", None, None).await;
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].host_patterns, vec!["www.googleapis.com"]);
        assert_eq!(
            result[0].headers.get("Authorization"),
            Some(&format!("Bearer {TEST_GOOGLE_OAUTH_TOKEN}"))
        );
    }

    #[tokio::test]
    async fn test_resolve_host_credentials_owner_scope_bearer() {
        use crate::secrets::{
            CreateSecretParams, CredentialLocation, CredentialMapping, SecretsStore,
        };
        use crate::tools::wasm::capabilities::HttpCapability;
        use crate::tools::wasm::wrapper::resolve_host_credentials;

        let store = test_secrets_store();
        let ctx = JobContext::with_user("owner-scope", "owner-scope test", "owner-scope test");

        store
            .create(
                &ctx.user_id,
                CreateSecretParams::new("google_oauth_token", TEST_GOOGLE_OAUTH_TOKEN),
            )
            .await
            .unwrap();

        let mut credentials = HashMap::new();
        credentials.insert(
            "google_oauth_token".to_string(),
            CredentialMapping {
                secret_name: "google_oauth_token".to_string(),
                location: CredentialLocation::AuthorizationBearer,
                host_patterns: vec!["www.googleapis.com".to_string()],
                path_patterns: Vec::new(),
                optional: false,
            },
        );

        let caps = Capabilities {
            http: Some(HttpCapability {
                credentials,
                ..Default::default()
            }),
            ..Default::default()
        };

        let result = resolve_host_credentials(&caps, Some(&store), &ctx.user_id, None, None).await;
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].headers.get("Authorization"),
            Some(&format!("Bearer {TEST_GOOGLE_OAUTH_TOKEN}"))
        );
    }

    #[tokio::test]
    async fn test_execute_resolves_host_credentials_from_owner_scope_context() {
        use crate::secrets::{CredentialLocation, CredentialMapping};
        use crate::tools::wasm::capabilities::HttpCapability;

        let runtime = Arc::new(WasmToolRuntime::new(WasmRuntimeConfig::for_testing()).unwrap());
        let prepared = runtime
            .prepare("search", b"\0asm\x0d\0\x01\0", None)
            .await
            .unwrap();
        let store = Arc::new(RecordingSecretsStore::new());
        let ctx = JobContext::with_user("owner-scope", "owner-scope test", "owner-scope test");

        store
            .create(
                &ctx.user_id,
                CreateSecretParams::new("google_oauth_token", TEST_GOOGLE_OAUTH_TOKEN),
            )
            .await
            .unwrap();

        let mut credentials = HashMap::new();
        credentials.insert(
            "google_oauth_token".to_string(),
            CredentialMapping {
                secret_name: "google_oauth_token".to_string(),
                location: CredentialLocation::AuthorizationBearer,
                host_patterns: vec!["www.googleapis.com".to_string()],
                path_patterns: Vec::new(),
                optional: false,
            },
        );

        let caps = Capabilities {
            http: Some(HttpCapability {
                credentials,
                ..Default::default()
            }),
            ..Default::default()
        };

        let wrapper = super::WasmToolWrapper::new(Arc::clone(&runtime), prepared, caps)
            .with_secrets_store(store.clone());
        let result = wrapper.execute(serde_json::json!({}), &ctx).await;
        assert!(result.is_err());

        let lookups = store.decrypted_lookups();
        assert!(lookups.contains(&("owner-scope".to_string(), "google_oauth_token".to_string())));
        assert!(!lookups.contains(&("default".to_string(), "google_oauth_token".to_string())));
    }

    #[tokio::test]
    async fn test_resolve_host_credentials_missing_secret() {
        use crate::secrets::{CredentialLocation, CredentialMapping};
        use crate::tools::wasm::capabilities::HttpCapability;
        use crate::tools::wasm::wrapper::resolve_host_credentials;

        let store = test_secrets_store();

        // No secret stored, should silently skip
        let mut credentials = HashMap::new();
        credentials.insert(
            "missing_token".to_string(),
            CredentialMapping {
                secret_name: "missing_token".to_string(),
                location: CredentialLocation::AuthorizationBearer,
                host_patterns: vec!["api.example.com".to_string()],
                path_patterns: Vec::new(),
                optional: false,
            },
        );

        let caps = Capabilities {
            http: Some(HttpCapability {
                credentials,
                ..Default::default()
            }),
            ..Default::default()
        };

        let result = resolve_host_credentials(&caps, Some(&store), "user1", None, None).await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_resolve_host_credentials_skips_refresh_when_not_expired() {
        use crate::secrets::{
            CreateSecretParams, CredentialLocation, CredentialMapping, SecretsStore,
        };
        use crate::tools::wasm::capabilities::HttpCapability;
        use crate::tools::wasm::wrapper::{OAuthRefreshConfig, resolve_host_credentials};

        let store = test_secrets_store();

        // Store a token that expires 2 hours from now (well within buffer)
        let expires_at = chrono::Utc::now() + chrono::Duration::hours(2);
        store
            .create(
                "user1",
                CreateSecretParams::new("google_oauth_token", TEST_GOOGLE_OAUTH_FRESH)
                    .with_expiry(expires_at),
            )
            .await
            .unwrap();

        let mut credentials = HashMap::new();
        credentials.insert(
            "google_oauth_token".to_string(),
            CredentialMapping {
                secret_name: "google_oauth_token".to_string(),
                location: CredentialLocation::AuthorizationBearer,
                host_patterns: vec!["www.googleapis.com".to_string()],
                path_patterns: Vec::new(),
                optional: false,
            },
        );

        let caps = Capabilities {
            http: Some(HttpCapability {
                credentials,
                ..Default::default()
            }),
            ..Default::default()
        };

        let oauth_config = OAuthRefreshConfig {
            token_url: "https://oauth2.googleapis.com/token".to_string(),
            client_id: TEST_OAUTH_CLIENT_ID.to_string(),
            client_secret: Some(TEST_OAUTH_CLIENT_SECRET.to_string()),
            exchange_proxy_url: None,
            gateway_token: None,
            secret_name: "google_oauth_token".to_string(),
            provider: Some("google".to_string()),
            extra_refresh_params: HashMap::new(),
        };

        // Should resolve the existing fresh token without attempting refresh
        let result =
            resolve_host_credentials(&caps, Some(&store), "user1", None, Some(&oauth_config)).await;
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].headers.get("Authorization"),
            Some(&format!("Bearer {TEST_GOOGLE_OAUTH_FRESH}"))
        );
    }

    #[tokio::test]
    async fn test_resolve_host_credentials_skips_refresh_no_config() {
        use crate::secrets::{
            CreateSecretParams, CredentialLocation, CredentialMapping, SecretsStore,
        };
        use crate::tools::wasm::capabilities::HttpCapability;
        use crate::tools::wasm::wrapper::resolve_host_credentials;

        let store = test_secrets_store();

        // Store an expired token
        let expires_at = chrono::Utc::now() - chrono::Duration::hours(1);
        store
            .create(
                "user1",
                CreateSecretParams::new("my_token", "expired-value").with_expiry(expires_at),
            )
            .await
            .unwrap();

        let mut credentials = HashMap::new();
        credentials.insert(
            "my_token".to_string(),
            CredentialMapping {
                secret_name: "my_token".to_string(),
                location: CredentialLocation::AuthorizationBearer,
                host_patterns: vec!["api.example.com".to_string()],
                path_patterns: Vec::new(),
                optional: false,
            },
        );

        let caps = Capabilities {
            http: Some(HttpCapability {
                credentials,
                ..Default::default()
            }),
            ..Default::default()
        };

        // No OAuth config, expired token can't be resolved (get_decrypted returns Expired)
        let result = resolve_host_credentials(&caps, Some(&store), "user1", None, None).await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_resolve_host_credentials_skips_refresh_no_expires_at() {
        use crate::secrets::{
            CreateSecretParams, CredentialLocation, CredentialMapping, SecretsStore,
        };
        use crate::tools::wasm::capabilities::HttpCapability;
        use crate::tools::wasm::wrapper::{OAuthRefreshConfig, resolve_host_credentials};

        let store = test_secrets_store();

        // Legacy token: no expires_at set
        store
            .create(
                "user1",
                CreateSecretParams::new("google_oauth_token", TEST_GOOGLE_OAUTH_LEGACY),
            )
            .await
            .unwrap();

        let mut credentials = HashMap::new();
        credentials.insert(
            "google_oauth_token".to_string(),
            CredentialMapping {
                secret_name: "google_oauth_token".to_string(),
                location: CredentialLocation::AuthorizationBearer,
                host_patterns: vec!["www.googleapis.com".to_string()],
                path_patterns: Vec::new(),
                optional: false,
            },
        );

        let caps = Capabilities {
            http: Some(HttpCapability {
                credentials,
                ..Default::default()
            }),
            ..Default::default()
        };

        let oauth_config = OAuthRefreshConfig {
            token_url: "https://oauth2.googleapis.com/token".to_string(),
            client_id: TEST_OAUTH_CLIENT_ID.to_string(),
            client_secret: Some(TEST_OAUTH_CLIENT_SECRET.to_string()),
            exchange_proxy_url: None,
            gateway_token: None,
            secret_name: "google_oauth_token".to_string(),
            provider: Some("google".to_string()),
            extra_refresh_params: HashMap::new(),
        };

        // Should use the legacy token directly without attempting refresh
        let result =
            resolve_host_credentials(&caps, Some(&store), "user1", None, Some(&oauth_config)).await;
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].headers.get("Authorization"),
            Some(&format!("Bearer {TEST_GOOGLE_OAUTH_LEGACY}"))
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_resolve_host_credentials_refreshes_via_proxy_without_direct_token_url_validation()
    {
        use crate::secrets::{
            CreateSecretParams, CredentialLocation, CredentialMapping, SecretsStore,
        };
        use crate::tools::wasm::capabilities::HttpCapability;
        use crate::tools::wasm::wrapper::{OAuthRefreshConfig, resolve_host_credentials};

        struct EnvLockGuard {
            _guard: std::sync::MutexGuard<'static, ()>,
        }
        impl EnvLockGuard {
            fn new() -> Self {
                Self {
                    _guard: crate::config::helpers::lock_env(),
                }
            }
        }

        // The OAuth proxy URL is now SSRF-validated. The mock proxy below
        // binds to a loopback address, which is normally rejected; opt into
        // the loopback escape hatch so the test can exercise the proxy
        // refresh path end-to-end. The escape hatch only affects this
        // process and is not exposed to operators.
        let _env_lock = EnvLockGuard::new();
        struct EnvGuard {
            original: Option<std::ffi::OsString>,
        }
        impl Drop for EnvGuard {
            fn drop(&mut self) {
                // SAFETY: Tests serialize env access with lock_env().
                unsafe {
                    match &self.original {
                        Some(value) => {
                            std::env::set_var("IRONCLAW_OAUTH_PROXY_ALLOW_LOOPBACK", value)
                        }
                        None => std::env::remove_var("IRONCLAW_OAUTH_PROXY_ALLOW_LOOPBACK"),
                    }
                };
            }
        }
        let original = std::env::var_os("IRONCLAW_OAUTH_PROXY_ALLOW_LOOPBACK");
        // SAFETY: Tests serialize env access with lock_env().
        unsafe { std::env::set_var("IRONCLAW_OAUTH_PROXY_ALLOW_LOOPBACK", "1") };
        let _proxy_loopback_guard = EnvGuard { original };

        let proxy = MockProxyServer::start().await;
        let store = test_secrets_store();

        store
            .create(
                "user1",
                CreateSecretParams::new("google_oauth_token", "expired-access-token")
                    .with_expiry(chrono::Utc::now() - chrono::Duration::hours(1)),
            )
            .await
            .unwrap();
        store
            .create(
                "user1",
                CreateSecretParams::new("google_oauth_token_refresh_token", "stored-refresh-token"),
            )
            .await
            .unwrap();

        let mut credentials = HashMap::new();
        credentials.insert(
            "google_oauth_token".to_string(),
            CredentialMapping {
                secret_name: "google_oauth_token".to_string(),
                location: CredentialLocation::AuthorizationBearer,
                host_patterns: vec!["www.googleapis.com".to_string()],
                path_patterns: Vec::new(),
                optional: false,
            },
        );

        let caps = Capabilities {
            http: Some(HttpCapability {
                credentials,
                ..Default::default()
            }),
            ..Default::default()
        };

        let oauth_config = OAuthRefreshConfig {
            token_url: "http://127.0.0.1:9/provider-token-endpoint".to_string(),
            client_id: "hosted-google-client-id".to_string(),
            client_secret: None,
            exchange_proxy_url: Some(proxy.base_url()),
            gateway_token: Some("gateway-test-token".to_string()),
            secret_name: "google_oauth_token".to_string(),
            provider: Some("google".to_string()),
            extra_refresh_params: HashMap::new(),
        };

        let resolved =
            resolve_host_credentials(&caps, Some(&store), "user1", None, Some(&oauth_config)).await;
        assert_eq!(resolved.len(), 1);
        assert_eq!(
            resolved[0].headers.get("Authorization"),
            Some(&"Bearer mock-refreshed-access-token".to_string())
        );

        let access_secret = store.get("user1", "google_oauth_token").await.unwrap();
        assert!(
            access_secret
                .expires_at
                .expect("refreshed access token expiry")
                > chrono::Utc::now()
        );
        let access_value = store
            .get_decrypted("user1", "google_oauth_token")
            .await
            .unwrap();
        assert_eq!(access_value.expose(), "mock-refreshed-access-token");

        let refresh_value = store
            .get_decrypted("user1", "google_oauth_token_refresh_token")
            .await
            .unwrap();
        assert_eq!(refresh_value.expose(), "mock-rotated-refresh-token");

        let requests = proxy.requests().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].authorization.as_deref(),
            Some("Bearer gateway-test-token")
        );
        assert_eq!(
            requests[0].form.get("client_id").map(String::as_str),
            Some("hosted-google-client-id")
        );
        assert_eq!(
            requests[0].form.get("token_url").map(String::as_str),
            Some("http://127.0.0.1:9/provider-token-endpoint")
        );
        assert_eq!(
            requests[0].form.get("refresh_token").map(String::as_str),
            Some("stored-refresh-token")
        );
        assert_eq!(
            requests[0].form.get("provider").map(String::as_str),
            Some("google")
        );
        assert!(!requests[0].form.contains_key("client_secret"));

        proxy.shutdown().await;
    }

    #[tokio::test]
    async fn test_resolve_host_credentials_skips_refresh_token_lookup_without_oauth_proxy_auth_token()
     {
        use crate::secrets::{
            CreateSecretParams, CredentialLocation, CredentialMapping, SecretsStore,
        };
        use crate::tools::wasm::capabilities::HttpCapability;
        use crate::tools::wasm::wrapper::{OAuthRefreshConfig, resolve_host_credentials};

        let store = RecordingSecretsStore::new();

        store
            .create(
                "user1",
                CreateSecretParams::new("google_oauth_token", "expired-access-token")
                    .with_expiry(chrono::Utc::now() - chrono::Duration::hours(1)),
            )
            .await
            .unwrap();
        store
            .create(
                "user1",
                CreateSecretParams::new("google_oauth_token_refresh_token", "stored-refresh-token"),
            )
            .await
            .unwrap();

        let mut credentials = HashMap::new();
        credentials.insert(
            "google_oauth_token".to_string(),
            CredentialMapping {
                secret_name: "google_oauth_token".to_string(),
                location: CredentialLocation::AuthorizationBearer,
                host_patterns: vec!["www.googleapis.com".to_string()],
                path_patterns: Vec::new(),
                optional: false,
            },
        );

        let caps = Capabilities {
            http: Some(HttpCapability {
                credentials,
                ..Default::default()
            }),
            ..Default::default()
        };

        let oauth_config = OAuthRefreshConfig {
            token_url: "https://oauth2.googleapis.com/token".to_string(),
            client_id: "hosted-google-client-id".to_string(),
            client_secret: None,
            exchange_proxy_url: Some("https://compose-api.example.com".to_string()),
            gateway_token: None,
            secret_name: "google_oauth_token".to_string(),
            provider: Some("google".to_string()),
            extra_refresh_params: HashMap::new(),
        };

        let resolved =
            resolve_host_credentials(&caps, Some(&store), "user1", None, Some(&oauth_config)).await;
        assert!(resolved.is_empty());

        let lookups = store.decrypted_lookups();
        assert!(lookups.contains(&("user1".to_string(), "google_oauth_token".to_string())));
        assert!(!lookups.contains(&(
            "user1".to_string(),
            "google_oauth_token_refresh_token".to_string(),
        )));
    }

    #[tokio::test]
    async fn test_resolve_host_credentials_skips_refresh_token_lookup_for_invalid_direct_token_url()
    {
        use crate::secrets::{
            CreateSecretParams, CredentialLocation, CredentialMapping, SecretsStore,
        };
        use crate::tools::wasm::capabilities::HttpCapability;
        use crate::tools::wasm::wrapper::{OAuthRefreshConfig, resolve_host_credentials};

        let store = RecordingSecretsStore::new();

        store
            .create(
                "user1",
                CreateSecretParams::new("google_oauth_token", "expired-access-token")
                    .with_expiry(chrono::Utc::now() - chrono::Duration::hours(1)),
            )
            .await
            .unwrap();
        store
            .create(
                "user1",
                CreateSecretParams::new("google_oauth_token_refresh_token", "stored-refresh-token"),
            )
            .await
            .unwrap();

        let mut credentials = HashMap::new();
        credentials.insert(
            "google_oauth_token".to_string(),
            CredentialMapping {
                secret_name: "google_oauth_token".to_string(),
                location: CredentialLocation::AuthorizationBearer,
                host_patterns: vec!["www.googleapis.com".to_string()],
                path_patterns: Vec::new(),
                optional: false,
            },
        );

        let caps = Capabilities {
            http: Some(HttpCapability {
                credentials,
                ..Default::default()
            }),
            ..Default::default()
        };

        let oauth_config = OAuthRefreshConfig {
            token_url: "http://127.0.0.1:9/provider-token-endpoint".to_string(),
            client_id: TEST_OAUTH_CLIENT_ID.to_string(),
            client_secret: Some(TEST_OAUTH_CLIENT_SECRET.to_string()),
            exchange_proxy_url: None,
            gateway_token: None,
            secret_name: "google_oauth_token".to_string(),
            provider: Some("google".to_string()),
            extra_refresh_params: HashMap::new(),
        };

        let resolved =
            resolve_host_credentials(&caps, Some(&store), "user1", None, Some(&oauth_config)).await;
        assert!(resolved.is_empty());

        let lookups = store.decrypted_lookups();
        assert!(lookups.contains(&("user1".to_string(), "google_oauth_token".to_string())));
        assert!(!lookups.contains(&(
            "user1".to_string(),
            "google_oauth_token_refresh_token".to_string(),
        )));
    }

    #[test]
    fn test_is_private_ip_v4() {
        use std::net::IpAddr;
        // Private ranges
        assert!(super::is_private_ip("127.0.0.1".parse::<IpAddr>().unwrap()));
        assert!(super::is_private_ip("10.0.0.1".parse::<IpAddr>().unwrap()));
        assert!(super::is_private_ip(
            "172.16.0.1".parse::<IpAddr>().unwrap()
        ));
        assert!(super::is_private_ip(
            "192.168.1.1".parse::<IpAddr>().unwrap()
        ));
        assert!(super::is_private_ip(
            "169.254.1.1".parse::<IpAddr>().unwrap()
        ));
        assert!(super::is_private_ip("0.0.0.0".parse::<IpAddr>().unwrap()));
        // CGNAT
        assert!(super::is_private_ip(
            "100.64.0.1".parse::<IpAddr>().unwrap()
        ));

        // Public IPs
        assert!(!super::is_private_ip("8.8.8.8".parse::<IpAddr>().unwrap()));
        assert!(!super::is_private_ip("1.1.1.1".parse::<IpAddr>().unwrap()));
        assert!(!super::is_private_ip(
            "93.184.216.34".parse::<IpAddr>().unwrap()
        ));
    }

    #[test]
    fn test_is_private_ip_v6() {
        use std::net::IpAddr;
        assert!(super::is_private_ip("::1".parse::<IpAddr>().unwrap()));
        assert!(super::is_private_ip("::".parse::<IpAddr>().unwrap()));
        assert!(super::is_private_ip("fc00::1".parse::<IpAddr>().unwrap()));
        assert!(super::is_private_ip("fe80::1".parse::<IpAddr>().unwrap()));

        // Public
        assert!(!super::is_private_ip(
            "2606:4700::1111".parse::<IpAddr>().unwrap()
        ));
    }

    #[test]
    fn test_reject_private_ip_loopback() {
        let result = super::reject_private_ip("https://127.0.0.1:8080/api");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("private/internal IP"));
    }

    #[test]
    fn test_reject_private_ip_internal() {
        let result = super::reject_private_ip("https://192.168.1.1/admin");
        assert!(result.is_err());
    }

    #[test]
    fn test_reject_private_ip_public_ok() {
        // 8.8.8.8 (Google DNS) is public
        let result = super::reject_private_ip("https://8.8.8.8/dns-query");
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_untyped_override_preserves_extracted_discovery_schema() {
        let typed_schema = serde_json::json!({
            "type": "object",
            "properties": {
                "values": {
                    "type": ["array", "null"],
                    "items": { "type": "array" }
                }
            }
        });

        let runtime = Arc::new(WasmToolRuntime::new(WasmRuntimeConfig::for_testing()).unwrap()); // safety: test-only setup
        let mut prepared = runtime
            .prepare("sheets", b"\0asm\x0d\0\x01\0", None)
            .await
            .unwrap(); // safety: test-only setup
        Arc::get_mut(&mut prepared).unwrap().schema = typed_schema.clone(); // safety: test-only setup

        let wrapper =
            super::WasmToolWrapper::new(Arc::clone(&runtime), prepared, Capabilities::default())
                .with_schema(serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": true
                }));

        #[rustfmt::skip]
        assert_eq!( // safety: test-only assertion
            wrapper.parameters_schema(),
            serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": true
            })
        );
        assert_eq!(wrapper.discovery_schema(), typed_schema); // safety: test-only assertion
    }

    #[tokio::test]
    async fn test_wrapper_returns_curated_discovery_summary() {
        let runtime = Arc::new(WasmToolRuntime::new(WasmRuntimeConfig::for_testing()).unwrap()); // safety: test-only setup
        let prepared = runtime
            .prepare("github", b"\0asm\x0d\0\x01\0", None)
            .await
            .unwrap(); // safety: test-only setup

        let summary = crate::tools::tool::ToolDiscoverySummary {
            always_required: vec!["action".into()],
            notes: vec!["Use tool_info for the full schema".into()],
            ..crate::tools::tool::ToolDiscoverySummary::default()
        };

        let wrapper =
            super::WasmToolWrapper::new(Arc::clone(&runtime), prepared, Capabilities::default())
                .with_discovery_summary(summary.clone());

        assert_eq!(wrapper.discovery_summary(), Some(summary));
    }

    #[test]
    fn test_build_tool_usage_hint_detects_nullable_container_properties() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "requests": {
                    "type": ["array", "null"],
                    "items": { "type": "object" }
                }
            }
        });

        let hint = super::build_tool_usage_hint("google_docs", &schema);

        assert!(hint.contains("native JSON arrays/objects")); // safety: test-only assertion
    }

    /// The hint must NOT recommend calling `tool_info` when the schema
    /// information can be inlined directly. The previous implementation
    /// always emitted "Tip: call tool_info(...)" which forced the agent
    /// to spend an extra turn fetching what it could have received in
    /// the error message.
    #[test]
    fn test_build_tool_usage_hint_inlines_oneof_required_map() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["action"],
            "oneOf": [
                {
                    "properties": {
                        "action": { "type": "string", "const": "list_files" }
                    },
                    "required": ["action"]
                },
                {
                    "properties": {
                        "action": { "type": "string", "const": "get_file" },
                        "file_id": { "type": "string" }
                    },
                    "required": ["action", "file_id"]
                },
                {
                    "properties": {
                        "action": { "type": "string", "const": "share_file" },
                        "file_id": { "type": "string" },
                        "email": { "type": "string" }
                    },
                    "required": ["action", "file_id", "email"]
                }
            ]
        });

        let hint = super::build_tool_usage_hint("google-drive-tool", &schema);

        // The hint must NOT recommend an extra round-trip via tool_info.
        assert!(
            !hint.contains("call tool_info"),
            "hint should not recommend tool_info when info can be inlined; got: {hint}"
        );
        // The hint should map each action to its required fields,
        // excluding the discriminator (which is always implicit).
        assert!(hint.contains("list_files=[]"));
        assert!(hint.contains("get_file=[file_id]"));
        assert!(hint.contains("share_file=[file_id,email]"));
        assert!(hint.contains("Required fields per action for google-drive-tool"));
    }

    /// For flat (non-oneOf) schemas, the hint should embed the schema
    /// JSON directly as long as it's under the size budget.
    #[test]
    fn test_build_tool_usage_hint_inlines_flat_schema() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" }
            },
            "required": ["query"]
        });

        let hint = super::build_tool_usage_hint("web-search-tool", &schema);

        assert!(
            !hint.contains("call tool_info"),
            "hint should not recommend tool_info for compact schemas; got: {hint}"
        );
        assert!(hint.contains("Schema for web-search-tool"));
        assert!(hint.contains("\"query\""));
        assert!(hint.contains("\"required\""));
    }

    /// Adversarial fallback: if the schema is huge enough to blow the
    /// inline budget AND has no `oneOf` action map, we fall back to the
    /// old `tool_info` tip rather than dumping multi-megabyte schemas
    /// into every error message.
    #[test]
    fn test_build_tool_usage_hint_falls_back_for_huge_flat_schema() {
        // Build a flat schema with many properties to exceed 4 KB.
        let mut props = serde_json::Map::new();
        for i in 0..200 {
            props.insert(
                format!("field_{i}"),
                serde_json::json!({
                    "type": "string",
                    "description": "lorem ipsum dolor sit amet consectetur adipiscing elit"
                }),
            );
        }
        let schema = serde_json::json!({
            "type": "object",
            "properties": props,
        });

        let hint = super::build_tool_usage_hint("massive-tool", &schema);

        assert!(
            hint.contains("call tool_info"),
            "huge flat schema should fall back to tool_info tip; got: {hint}"
        );
        assert!(hint.contains("too large to inline"));
    }

    /// Direct unit test for the helper.
    #[test]
    fn test_extract_action_required_map_strips_discriminator() {
        let schema = serde_json::json!({
            "oneOf": [
                {
                    "properties": { "action": { "const": "a" } },
                    "required": ["action"]
                },
                {
                    "properties": {
                        "action": { "const": "b" },
                        "x": { "type": "string" }
                    },
                    "required": ["action", "x"]
                }
            ]
        });

        let map = super::extract_action_required_map(&schema).expect("should produce map");
        // The "action" discriminator must NOT appear in any per-action
        // required list — it's always implicit.
        assert_eq!(map, "a=[], b=[x]");
    }

    /// Schemas without `oneOf` should yield None so the caller falls
    /// back to either inlining the flat schema or the tool_info tip.
    #[test]
    fn test_extract_action_required_map_returns_none_for_flat_schema() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "query": { "type": "string" } },
            "required": ["query"]
        });
        assert!(super::extract_action_required_map(&schema).is_none());
    }

    /// Regression test: leak scan must run on raw headers (before credential
    /// injection), not after. If it ran post-injection, the host-injected
    /// Slack bot token (`xoxb-...`) would trigger a Block and reject the
    /// tool's own legitimate outbound request.
    #[test]
    fn test_leak_scan_runs_before_credential_injection() {
        use ironclaw_safety::LeakDetector;

        // Simulate pre-injection headers: WASM only sees the placeholder, not the real token.
        let raw_headers: Vec<(String, String)> = vec![
            (
                "Authorization".to_string(),
                "Bearer {SLACK_BOT_TOKEN}".to_string(),
            ),
            ("Content-Type".to_string(), "application/json".to_string()),
        ];

        let detector = LeakDetector::new();

        // Pre-injection scan should pass — placeholders are not secrets.
        let pre_result = detector.scan_http_request(
            "https://slack.com/api/chat.postMessage",
            &raw_headers,
            None,
        );
        assert!(
            pre_result.is_ok(),
            "Leak scan on pre-injection headers should pass, but got: {:?}",
            pre_result
        );

        // Post-injection headers would contain a real Slack token.
        let post_injection_headers: Vec<(String, String)> = vec![
            (
                "Authorization".to_string(),
                "Bearer xoxb-1234567890-abcdefghij".to_string(),
            ),
            ("Content-Type".to_string(), "application/json".to_string()),
        ];

        // Post-injection scan WOULD block — this is the false positive
        // that the pre-injection ordering prevents.
        let post_result = detector.scan_http_request(
            "https://slack.com/api/chat.postMessage",
            &post_injection_headers,
            None,
        );
        assert!(
            post_result.is_err(),
            "Leak scan on post-injection headers should block the Slack token"
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_resolve_host_credentials_denies_default_fallback_for_admin_user() {
        use crate::secrets::{CredentialLocation, CredentialMapping, SecretsStore};
        use crate::tools::wasm::capabilities::HttpCapability;
        use crate::tools::wasm::wrapper::resolve_host_credentials;

        let store = test_secrets_store();
        let db = test_user_db("routine_user_123", "admin").await;

        // Store a token under the "default" global user
        store
            .create(
                "default",
                crate::secrets::CreateSecretParams::new("google_oauth_token", "global_token_value"),
            )
            .await
            .expect("Failed to store global token"); // safety: test code only

        // Create capabilities requiring this credential
        let mut creds = std::collections::HashMap::new();
        creds.insert(
            "google_oauth_token".to_string(),
            CredentialMapping {
                secret_name: "google_oauth_token".to_string(),
                location: CredentialLocation::AuthorizationBearer,
                host_patterns: vec!["sheets.googleapis.com".to_string()],
                path_patterns: Vec::new(),
                optional: false,
            },
        );
        let caps = Capabilities {
            http: Some(HttpCapability {
                allowlist: vec![],
                credentials: creds,
                rate_limit: crate::tools::wasm::capabilities::RateLimitConfig::default(),
                max_request_bytes: 1024 * 1024,
                max_response_bytes: 10 * 1024 * 1024,
                timeout: std::time::Duration::from_secs(30),
            }),
            ..Default::default()
        };

        // Resolve credentials for a different user (routine context).
        // The WASM tool path must fail closed and refuse to borrow the
        // admin-only default scope.
        let result = resolve_host_credentials(
            &caps,
            Some(&store),
            "routine_user_123",
            Some(db.as_ref()),
            None,
        )
        .await;

        assert!(
            result.is_empty(),
            "WASM tool credential resolution must not borrow the default scope"
        );
        assert_eq!(
            result.missing_required,
            vec!["google_oauth_token".to_string()],
            "missing required credential should still be reported"
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_resolve_host_credentials_denies_default_fallback_when_caller_is_default() {
        // Regression: a caller whose `user_id` is literally "default" must NOT
        // be granted the AdminOnly default-fallback path. The fallback exists
        // so that admin-initiated background jobs can borrow a global secret
        // from the "default" scope; if the caller IS already "default", there
        // is nothing to fall back to and treating it as an admin loops the
        // resolution back into the same scope it just failed in.
        use crate::tools::wasm::wrapper::resolve_host_credentials;

        let store = test_secrets_store();
        // Even though the user has admin role, the literal id "default" must
        // short-circuit the fallback decision.
        let db = test_user_db("default", "admin").await;

        // No secret stored anywhere — neither under "default" nor any other
        // scope. The resolver should report an empty result, not panic and
        // not silently bypass the AdminOnly gate.
        let caps = test_capabilities_with_google_oauth();
        let result =
            resolve_host_credentials(&caps, Some(&store), "default", Some(db.as_ref()), None).await;

        assert!(
            result.is_empty(),
            "caller user_id == 'default' must not enter the fallback branch"
        );
        assert_eq!(
            result.missing_required,
            vec!["google_oauth_token".to_string()],
            "missing required credential should still be reported"
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_resolve_host_credentials_denies_default_fallback_for_member_user() {
        use crate::secrets::SecretsStore;
        use crate::tools::wasm::wrapper::resolve_host_credentials;

        let store = test_secrets_store();
        let db = test_user_db("member_user_123", "member").await;

        store
            .create(
                "default",
                crate::secrets::CreateSecretParams::new("google_oauth_token", "global_token"),
            )
            .await
            .expect("Failed to store global token");

        let caps = test_capabilities_with_google_oauth();
        let result = resolve_host_credentials(
            &caps,
            Some(&store),
            "member_user_123",
            Some(db.as_ref()),
            None,
        )
        .await;

        assert!(
            result.is_empty(),
            "member users must not fallback to default"
        );
    }

    fn test_capabilities_with_google_oauth() -> Capabilities {
        use crate::secrets::{CredentialLocation, CredentialMapping};
        use crate::tools::wasm::capabilities::HttpCapability;

        let mut creds = std::collections::HashMap::new();
        creds.insert(
            "google_oauth_token".to_string(),
            CredentialMapping {
                secret_name: "google_oauth_token".to_string(),
                location: CredentialLocation::AuthorizationBearer,
                host_patterns: vec!["sheets.googleapis.com".to_string()],
                path_patterns: Vec::new(),
                optional: false,
            },
        );
        Capabilities {
            http: Some(HttpCapability {
                allowlist: vec![],
                credentials: creds,
                rate_limit: crate::tools::wasm::capabilities::RateLimitConfig::default(),
                max_request_bytes: 1024 * 1024,
                max_response_bytes: 10 * 1024 * 1024,
                timeout: std::time::Duration::from_secs(30),
            }),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_resolve_host_credentials_prefers_user_specific_over_default() {
        use crate::secrets::SecretsStore;
        use crate::tools::wasm::wrapper::resolve_host_credentials;

        let store = test_secrets_store();

        // Store token under "default" (global)
        store
            .create(
                "default",
                crate::secrets::CreateSecretParams::new("google_oauth_token", "global_token"),
            )
            .await
            .expect("Failed to store global token"); // safety: test code only

        // Store token under user_123 (user-specific)
        store
            .create(
                "user_123",
                crate::secrets::CreateSecretParams::new(
                    "google_oauth_token",
                    "user_specific_token",
                ),
            )
            .await
            .expect("Failed to store user token"); // safety: test code only

        // Create capabilities
        let caps = test_capabilities_with_google_oauth();

        // Resolve credentials for user_123
        // Should prefer user_123's token over default
        let result = resolve_host_credentials(&caps, Some(&store), "user_123", None, None).await;

        assert!(!result.is_empty(), "has user credentials"); // safety: test code only
        assert_eq!(result[0].secret_value, "user_specific_token", "user token"); // safety: test code only
    }

    #[tokio::test]
    async fn test_resolve_host_credentials_no_fallback_when_already_default() {
        use crate::secrets::SecretsStore;
        use crate::tools::wasm::wrapper::resolve_host_credentials;

        let store = test_secrets_store();

        // Only store token under "default" (not a duplicate)
        store
            .create(
                "default",
                crate::secrets::CreateSecretParams::new("google_oauth_token", "default_token"),
            )
            .await
            .expect("Failed to store default token"); // safety: test code only

        // Create capabilities
        let caps = test_capabilities_with_google_oauth();

        // Resolve credentials for "default" user
        // Should NOT attempt fallback (already looking up default)
        let result = resolve_host_credentials(&caps, Some(&store), "default", None, None).await;

        assert!(!result.is_empty(), "Should find default token"); // safety: test code only
        assert_eq!(result[0].secret_value, "default_token"); // safety: test code only
    }

    #[tokio::test]
    async fn test_resolve_host_credentials_missing_secret_warns() {
        use crate::tools::wasm::wrapper::resolve_host_credentials;

        let store = test_secrets_store();

        // Don't store any token

        // Create capabilities expecting a credential
        let caps = test_capabilities_with_google_oauth();

        // Resolve credentials when neither user nor default has the token
        let result = resolve_host_credentials(&caps, Some(&store), "user_456", None, None).await;

        // Should return empty since credential can't be found anywhere
        assert!(result.is_empty(), "no credentials found"); // safety: test code only
    }

    // --- needs_content_length_zero (regression for #1529) ---

    #[test]
    fn post_no_body_needs_content_length() {
        let headers = HashMap::new();
        assert!(
            super::needs_content_length_zero("POST", &headers),
            "POST with no body must get Content-Length: 0 to avoid 411"
        );
    }

    #[test]
    fn put_no_body_needs_content_length() {
        assert!(super::needs_content_length_zero("PUT", &HashMap::new()));
    }

    #[test]
    fn delete_no_body_needs_content_length() {
        assert!(super::needs_content_length_zero("DELETE", &HashMap::new()));
    }

    #[test]
    fn patch_no_body_needs_content_length() {
        assert!(super::needs_content_length_zero("PATCH", &HashMap::new()));
    }

    #[test]
    fn get_no_body_skips_content_length() {
        assert!(!super::needs_content_length_zero("GET", &HashMap::new()));
    }

    #[test]
    fn head_no_body_skips_content_length() {
        assert!(!super::needs_content_length_zero("HEAD", &HashMap::new()));
    }

    #[test]
    fn post_no_body_respects_explicit_content_length() {
        let mut headers = HashMap::new();
        headers.insert("Content-Length".to_string(), "0".to_string());
        assert!(
            !super::needs_content_length_zero("POST", &headers),
            "should not double-add when tool already sets Content-Length"
        );
    }

    #[test]
    fn content_length_check_is_case_insensitive() {
        let mut headers = HashMap::new();
        headers.insert("content-length".to_string(), "0".to_string());
        assert!(!super::needs_content_length_zero("POST", &headers));
    }

    /// Downcast-based classification: real `wasmtime::Trap` variants
    /// map to the correct `WasmError` via structured downcast.
    #[test]
    fn trap_classification_fuel_via_downcast() {
        use crate::tools::wasm::error::WasmError;
        use crate::tools::wasm::limits::ResourceLimits;

        let limits = ResourceLimits::default();
        let err: wasmtime::Error = wasmtime::Trap::OutOfFuel.into();
        let result = super::classify_trap_error(err, &limits);
        assert!(
            matches!(result, WasmError::FuelExhausted { .. }),
            "OutOfFuel not detected: {result:?}"
        );
    }

    #[test]
    fn trap_classification_stack_overflow_via_downcast() {
        use crate::tools::wasm::error::WasmError;
        use crate::tools::wasm::limits::ResourceLimits;

        let limits = ResourceLimits::default();
        let err: wasmtime::Error = wasmtime::Trap::StackOverflow.into();
        let result = super::classify_trap_error(err, &limits);
        assert!(
            matches!(result, WasmError::Trapped(ref s) if s.contains("stack overflow")),
            "StackOverflow not detected: {result:?}"
        );
    }

    #[test]
    fn trap_classification_unreachable_via_downcast() {
        use crate::tools::wasm::error::WasmError;
        use crate::tools::wasm::limits::ResourceLimits;

        let limits = ResourceLimits::default();
        let err: wasmtime::Error = wasmtime::Trap::UnreachableCodeReached.into();
        let result = super::classify_trap_error(err, &limits);
        assert!(
            matches!(result, WasmError::Trapped(ref s) if s.contains("unreachable")),
            "UnreachableCodeReached not detected: {result:?}"
        );
    }

    /// Non-Trap errors (host glue, component model) pass through with full chain.
    #[test]
    fn trap_classification_non_trap_preserves_chain() {
        use crate::tools::wasm::error::WasmError;
        use crate::tools::wasm::limits::ResourceLimits;

        let limits = ResourceLimits::default();
        let err = wasmtime::Error::msg("component model glue exploded");
        let result = super::classify_trap_error(err, &limits);
        assert!(
            matches!(result, WasmError::Trapped(ref s) if s.contains("component model glue")),
            "non-trap error lost: {result:?}"
        );
    }

    /// String-matching fallback: when the Trap is wrapped in host/component
    /// glue that the downcast can't see through, the Display chain still
    /// contains the diagnostic string.
    #[test]
    fn trap_classification_fuel_via_string_fallback() {
        use crate::tools::wasm::error::WasmError;
        use crate::tools::wasm::limits::ResourceLimits;

        let limits = ResourceLimits::default();
        // Wrap the fuel message in a plain wasmtime::Error so downcast_ref
        // for Trap returns None — exercises the string-matching path.
        let err = wasmtime::Error::msg("wasm trap: all fuel consumed by wasm");
        let result = super::classify_trap_error(err, &limits);
        assert!(
            matches!(result, WasmError::FuelExhausted { .. }),
            "string-fallback fuel detection failed: {result:?}"
        );
    }

    #[test]
    fn resolved_host_credential_debug_redacts_secret_material() {
        // Defense-in-depth: a future log line / dbg!() / panic message that
        // accidentally formats a `ResolvedHostCredential` with `{:?}` must
        // never spill the decrypted secret. The hand-rolled Debug impl
        // prints structural info (host patterns + header / query NAMES)
        // and replaces every value with `[REDACTED]`.
        let mut headers = HashMap::new();
        headers.insert(
            "Authorization".to_string(),
            "Bearer super-secret-token-do-not-leak".to_string(),
        );
        let mut query_params = HashMap::new();
        query_params.insert(
            "api_key".to_string(),
            "another-secret-value-also-do-not-leak".to_string(),
        );
        let cred = super::ResolvedHostCredential {
            secret_name: "googleapis_secret".to_string(),
            host_patterns: vec!["www.googleapis.com".to_string()],
            path_patterns: vec![],
            headers,
            query_params,
            secret_value: "raw-secret-bytes".to_string(),
            signing: None,
        };

        let debug_output = format!("{cred:?}");

        // Structural info that's safe to log MUST be present.
        assert!(debug_output.contains("ResolvedHostCredential"));
        assert!(debug_output.contains("www.googleapis.com"));
        assert!(debug_output.contains("Authorization"));
        assert!(debug_output.contains("api_key"));
        assert!(debug_output.contains("[REDACTED]"));

        // Every secret-bearing value MUST be absent.
        assert!(
            !debug_output.contains("super-secret-token-do-not-leak"),
            "header value leaked: {debug_output}"
        );
        assert!(
            !debug_output.contains("another-secret-value-also-do-not-leak"),
            "query param value leaked: {debug_output}"
        );
        assert!(
            !debug_output.contains("raw-secret-bytes"),
            "secret_value leaked: {debug_output}"
        );
    }
}
