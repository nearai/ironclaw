//! Channel restricted egress: the policy half of `RestrictedEgress` for
//! channel adapters (OUT-8).
//!
//! The resolved `[[channel.egress]]` declarations are the sole authority:
//! scheme/host/method allowlisting, host-owned-header rejection, and
//! credential-handle screening all happen here, **before** any transport
//! activity. What leaves this module is an [`ApprovedChannelEgress`] — a
//! policy-approved plan carrying the declared injection target — executed by
//! the composition-injected [`ChannelEgressTransport`] (which resolves secret
//! material and drives the host runtime egress with its private-IP/redirect
//! denial and response caps). Adapters never see secret bytes; a buggy or
//! malicious adapter cannot name an undeclared host, method, or credential.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{
    NetworkMethod, NetworkScheme, RestrictedEgress, RestrictedEgressError, RestrictedEgressRequest,
    RestrictedEgressResponse, RuntimeCredentialTarget, SecretHandle,
};

use crate::lifecycle::EgressFactory;

/// Default response-body cap for channel vendor calls.
pub const CHANNEL_EGRESS_RESPONSE_BODY_LIMIT_BYTES: u64 = 256 * 1024;
/// Default per-call timeout for channel vendor calls.
pub const CHANNEL_EGRESS_TIMEOUT_MS: u64 = 10_000;

/// Headers the host owns; an adapter supplying one is rejected before any
/// network activity (`authorization` belongs to declared credential
/// injection; the rest are transport-owned or hop-by-hop).
const HOST_OWNED_HEADERS: &[&str] = &[
    "authorization",
    "host",
    "content-length",
    "connection",
    "transfer-encoding",
    "upgrade",
    "proxy-authorization",
    "te",
    "trailer",
];

/// One egress target's declared policy, resolved from the manifest.
#[derive(Debug, Clone)]
pub struct DeclaredChannelEgress {
    pub scheme: NetworkScheme,
    pub host: String,
    pub methods: Vec<NetworkMethod>,
    pub credential_handle: Option<SecretHandle>,
    pub injection: Option<RuntimeCredentialTarget>,
    /// Declared body-credential bindings: each maps a handle to the RFC 6901
    /// JSON pointer where the host inserts its resolved value. A request
    /// naming an undeclared handle is rejected before any transport activity.
    pub body_credentials: Vec<ironclaw_host_api::ChannelBodyCredentialDescriptor>,
}

/// The credential part of an approved plan: the declared handle plus the
/// declared injection target (defaulting to `Authorization: Bearer`).
#[derive(Debug, Clone)]
pub struct ApprovedChannelCredential {
    pub handle: SecretHandle,
    pub target: RuntimeCredentialTarget,
}

/// A policy-approved vendor call. Everything here has passed the declared
/// allowlist; the transport must still enforce private-IP/redirect denial and
/// the response cap at the network boundary.
#[derive(Debug, Clone)]
pub struct ApprovedChannelEgress {
    pub extension_id: String,
    pub installation_id: String,
    pub method: NetworkMethod,
    /// Full URL; when the injection target is a path placeholder the
    /// `{placeholder}` is still present — the transport substitutes secret
    /// material host-side.
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// The single host the transport must pin its network policy to.
    pub host: String,
    pub credential: Option<ApprovedChannelCredential>,
    /// Declared body credentials this call opted into: handle plus its
    /// manifest-declared `BodyJsonPointer` target. Resolution and insertion
    /// are the transport's job, host-side.
    pub body_credentials: Vec<ApprovedChannelCredential>,
    pub response_body_limit: u64,
    pub timeout_ms: u64,
}

/// Executes approved plans. Implemented by composition over the host runtime
/// egress (secret-material resolution + injection + SSRF-safe transport).
#[async_trait]
pub trait ChannelEgressTransport: Send + Sync {
    async fn execute(
        &self,
        approved: ApprovedChannelEgress,
    ) -> Result<RestrictedEgressResponse, RestrictedEgressError>;
}

/// Per-extension channel egress: declared policy + injected transport.
pub struct PolicyEnforcedChannelEgress {
    extension_id: String,
    installation_id: String,
    declared: Vec<DeclaredChannelEgress>,
    transport: Arc<dyn ChannelEgressTransport>,
}

impl PolicyEnforcedChannelEgress {
    pub fn new(
        extension_id: impl Into<String>,
        installation_id: impl Into<String>,
        declared: Vec<DeclaredChannelEgress>,
        transport: Arc<dyn ChannelEgressTransport>,
    ) -> Self {
        Self {
            extension_id: extension_id.into(),
            installation_id: installation_id.into(),
            declared,
            transport,
        }
    }

    fn approve(
        &self,
        request: RestrictedEgressRequest,
    ) -> Result<ApprovedChannelEgress, RestrictedEgressError> {
        let url = url::Url::parse(&request.url).map_err(|_| RestrictedEgressError::PolicyDenied)?;
        let scheme = match url.scheme() {
            "https" => NetworkScheme::Https,
            _ => return Err(RestrictedEgressError::PolicyDenied),
        };
        let host = url
            .host_str()
            .ok_or(RestrictedEgressError::PolicyDenied)?
            .to_ascii_lowercase();
        let declared = self
            .declared
            .iter()
            .find(|target| target.scheme == scheme && target.host.eq_ignore_ascii_case(&host))
            .ok_or_else(|| RestrictedEgressError::UndeclaredHost { host: host.clone() })?;
        if !declared.methods.contains(&request.method) {
            return Err(RestrictedEgressError::UndeclaredMethod);
        }
        for (name, _) in &request.headers {
            if HOST_OWNED_HEADERS
                .iter()
                .any(|owned| name.eq_ignore_ascii_case(owned))
            {
                return Err(RestrictedEgressError::HostOwnedHeader { name: name.clone() });
            }
        }
        let credential = match (&request.credential, &declared.credential_handle) {
            (None, _) => None,
            (Some(handle), Some(declared_handle)) if handle == declared_handle => {
                Some(ApprovedChannelCredential {
                    handle: handle.clone(),
                    target: declared.injection.clone().unwrap_or_else(|| {
                        RuntimeCredentialTarget::Header {
                            name: "authorization".to_string(),
                            prefix: Some("Bearer ".to_string()),
                        }
                    }),
                })
            }
            (Some(handle), _) => {
                return Err(RestrictedEgressError::UndeclaredCredential {
                    handle: handle.as_str().to_string(),
                });
            }
        };
        let mut body_credentials = Vec::new();
        for handle in &request.body_credentials {
            let declared_binding = declared
                .body_credentials
                .iter()
                .find(|binding| &binding.handle == handle)
                .ok_or_else(|| RestrictedEgressError::UndeclaredCredential {
                    handle: handle.as_str().to_string(),
                })?;
            // A duplicate opt-in would double-insert at the same pointer;
            // reject it as the adapter bug it is.
            if body_credentials
                .iter()
                .any(|approved: &ApprovedChannelCredential| &approved.handle == handle)
            {
                return Err(RestrictedEgressError::UndeclaredCredential {
                    handle: handle.as_str().to_string(),
                });
            }
            body_credentials.push(ApprovedChannelCredential {
                handle: handle.clone(),
                target: RuntimeCredentialTarget::BodyJsonPointer {
                    pointer: declared_binding.pointer.clone(),
                },
            });
        }
        Ok(ApprovedChannelEgress {
            extension_id: self.extension_id.clone(),
            installation_id: self.installation_id.clone(),
            method: request.method,
            url: request.url,
            headers: request.headers,
            body: request.body.unwrap_or_default(),
            host,
            credential,
            body_credentials,
            response_body_limit: CHANNEL_EGRESS_RESPONSE_BODY_LIMIT_BYTES,
            timeout_ms: CHANNEL_EGRESS_TIMEOUT_MS,
        })
    }
}

#[async_trait]
impl RestrictedEgress for PolicyEnforcedChannelEgress {
    async fn send(
        &self,
        request: RestrictedEgressRequest,
    ) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
        let approved = self.approve(request)?;
        let response = self.transport.execute(approved).await?;
        if response.body.len() as u64 > CHANNEL_EGRESS_RESPONSE_BODY_LIMIT_BYTES {
            // Defensive double-check; the transport enforces the cap at the
            // network boundary.
            return Err(RestrictedEgressError::ResponseTooLarge);
        }
        Ok(response)
    }
}

impl DeclaredChannelEgress {
    /// Lift one resolved `[[channel.egress]]` declaration into policy form.
    pub fn from_descriptor(descriptor: &ironclaw_host_api::ChannelEgressDescriptor) -> Self {
        Self {
            scheme: descriptor.scheme,
            host: descriptor.host.clone(),
            methods: descriptor.methods.clone(),
            credential_handle: descriptor.credential_handle.clone(),
            injection: descriptor.injection.clone(),
            body_credentials: descriptor.body_credentials.clone(),
        }
    }
}

/// The production [`EgressFactory`]: builds a policy-enforced egress from the
/// declaration the lifecycle passes (staged or active) over one injected
/// transport. An extension with no declared egress gets a policy that rejects
/// every host — fail-closed, never a panic.
pub struct TransportBackedEgressFactory {
    transport: Arc<dyn ChannelEgressTransport>,
}

impl TransportBackedEgressFactory {
    pub fn new(transport: Arc<dyn ChannelEgressTransport>) -> Self {
        Self { transport }
    }
}

impl EgressFactory for TransportBackedEgressFactory {
    fn egress_for_channel(
        &self,
        extension_id: &str,
        installation_id: &str,
        declared: &[ironclaw_host_api::ChannelEgressDescriptor],
    ) -> Arc<dyn RestrictedEgress> {
        Arc::new(PolicyEnforcedChannelEgress::new(
            extension_id,
            installation_id,
            declared
                .iter()
                .map(DeclaredChannelEgress::from_descriptor)
                .collect(),
            Arc::clone(&self.transport),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingTransport {
        approved: Mutex<Vec<ApprovedChannelEgress>>,
    }

    #[async_trait]
    impl ChannelEgressTransport for RecordingTransport {
        async fn execute(
            &self,
            approved: ApprovedChannelEgress,
        ) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
            self.approved.lock().unwrap().push(approved);
            Ok(RestrictedEgressResponse {
                status: 200,
                body: b"{}".to_vec(),
            })
        }
    }

    fn declared_vendor() -> Vec<DeclaredChannelEgress> {
        vec![DeclaredChannelEgress {
            scheme: NetworkScheme::Https,
            host: "vendor.example".to_string(),
            methods: vec![NetworkMethod::Post],
            credential_handle: Some(SecretHandle::new("vendor_bot_token").unwrap()),
            injection: None,
            body_credentials: Vec::new(),
        }]
    }

    fn egress_over(
        declared: Vec<DeclaredChannelEgress>,
    ) -> (PolicyEnforcedChannelEgress, Arc<RecordingTransport>) {
        let transport = Arc::new(RecordingTransport::default());
        (
            PolicyEnforcedChannelEgress::new(
                "vendorx",
                "inst-1",
                declared,
                transport.clone() as Arc<dyn ChannelEgressTransport>,
            ),
            transport,
        )
    }

    fn post(url: &str) -> RestrictedEgressRequest {
        RestrictedEgressRequest {
            method: NetworkMethod::Post,
            url: url.to_string(),
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: Some(b"{}".to_vec()),
            credential: Some(SecretHandle::new("vendor_bot_token").unwrap()),
            body_credentials: Vec::new(),
        }
    }

    #[tokio::test]
    async fn undeclared_host_is_rejected_before_any_transport_activity() {
        let (egress, transport) = egress_over(declared_vendor());
        let error = egress
            .send(post("https://evil.example/api/x"))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            RestrictedEgressError::UndeclaredHost { .. }
        ));
        assert!(transport.approved.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn non_https_and_undeclared_method_are_rejected_before_transport() {
        let (egress, transport) = egress_over(declared_vendor());
        let error = egress
            .send(post("http://vendor.example/api/x"))
            .await
            .unwrap_err();
        assert!(matches!(error, RestrictedEgressError::PolicyDenied));

        let mut get = post("https://vendor.example/api/x");
        get.method = NetworkMethod::Get;
        let error = egress.send(get).await.unwrap_err();
        assert!(matches!(error, RestrictedEgressError::UndeclaredMethod));
        assert!(transport.approved.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn adapter_supplied_authorization_header_is_rejected() {
        let (egress, transport) = egress_over(declared_vendor());
        let mut request = post("https://vendor.example/api/x");
        request
            .headers
            .push(("Authorization".to_string(), "Bearer stolen".to_string()));
        let error = egress.send(request).await.unwrap_err();
        assert!(matches!(
            error,
            RestrictedEgressError::HostOwnedHeader { .. }
        ));
        assert!(transport.approved.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn undeclared_credential_handle_is_rejected() {
        let (egress, transport) = egress_over(declared_vendor());
        let mut request = post("https://vendor.example/api/x");
        request.credential = Some(SecretHandle::new("some_other_secret").unwrap());
        let error = egress.send(request).await.unwrap_err();
        assert!(matches!(
            error,
            RestrictedEgressError::UndeclaredCredential { .. }
        ));
        assert!(transport.approved.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn approved_plan_carries_default_bearer_injection_and_pinned_host() {
        let (egress, transport) = egress_over(declared_vendor());
        egress
            .send(post("https://vendor.example/api/chat.postMessage"))
            .await
            .unwrap();
        let approved = transport.approved.lock().unwrap();
        assert_eq!(approved.len(), 1);
        assert_eq!(approved[0].host, "vendor.example");
        assert_eq!(approved[0].extension_id, "vendorx");
        let credential = approved[0].credential.as_ref().unwrap();
        assert!(matches!(
            &credential.target,
            RuntimeCredentialTarget::Header { name, prefix }
                if name == "authorization" && prefix.as_deref() == Some("Bearer ")
        ));
    }

    #[tokio::test]
    async fn approved_plan_carries_declared_path_placeholder_injection() {
        let mut declared = declared_vendor();
        declared[0].injection = Some(RuntimeCredentialTarget::PathPlaceholder {
            placeholder: "token".to_string(),
        });
        let (egress, transport) = egress_over(declared);
        egress
            .send(post("https://vendor.example/bot{token}/sendMessage"))
            .await
            .unwrap();
        let approved = transport.approved.lock().unwrap();
        let credential = approved[0].credential.as_ref().unwrap();
        assert!(matches!(
            &credential.target,
            RuntimeCredentialTarget::PathPlaceholder { placeholder } if placeholder == "token"
        ));
        assert!(
            approved[0].url.contains("{token}"),
            "placeholder substitution is the transport's job, host-side"
        );
    }

    #[tokio::test]
    async fn declared_body_credential_is_approved_with_its_declared_pointer() {
        let mut declared = declared_vendor();
        declared[0].body_credentials = vec![ironclaw_host_api::ChannelBodyCredentialDescriptor {
            handle: SecretHandle::new("vendor_webhook_secret").unwrap(),
            pointer: "/secret_token".to_string(),
        }];
        let (egress, transport) = egress_over(declared);
        let mut request = post("https://vendor.example/api/setWebhook");
        request.body_credentials = vec![SecretHandle::new("vendor_webhook_secret").unwrap()];
        egress.send(request).await.unwrap();
        let approved = transport.approved.lock().unwrap();
        assert_eq!(approved[0].body_credentials.len(), 1);
        assert_eq!(
            approved[0].body_credentials[0].handle.as_str(),
            "vendor_webhook_secret"
        );
        assert!(
            matches!(
                &approved[0].body_credentials[0].target,
                RuntimeCredentialTarget::BodyJsonPointer { pointer } if pointer == "/secret_token"
            ),
            "the pointer comes from the DECLARATION, never from the adapter"
        );
    }

    #[tokio::test]
    async fn undeclared_body_credential_handle_is_rejected_before_transport() {
        // No body_credentials declared at all: any opt-in is rejected.
        let (egress, transport) = egress_over(declared_vendor());
        let mut request = post("https://vendor.example/api/setWebhook");
        request.body_credentials = vec![SecretHandle::new("vendor_webhook_secret").unwrap()];
        let error = egress.send(request).await.unwrap_err();
        assert!(matches!(
            error,
            RestrictedEgressError::UndeclaredCredential { .. }
        ));
        assert!(transport.approved.lock().unwrap().is_empty());

        // Declared for a DIFFERENT handle: still rejected.
        let mut declared = declared_vendor();
        declared[0].body_credentials = vec![ironclaw_host_api::ChannelBodyCredentialDescriptor {
            handle: SecretHandle::new("vendor_webhook_secret").unwrap(),
            pointer: "/secret_token".to_string(),
        }];
        let (egress, transport) = egress_over(declared);
        let mut request = post("https://vendor.example/api/setWebhook");
        request.body_credentials = vec![SecretHandle::new("some_other_secret").unwrap()];
        let error = egress.send(request).await.unwrap_err();
        assert!(matches!(
            error,
            RestrictedEgressError::UndeclaredCredential { .. }
        ));
        assert!(transport.approved.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn duplicate_body_credential_opt_in_is_rejected_before_transport() {
        let mut declared = declared_vendor();
        declared[0].body_credentials = vec![ironclaw_host_api::ChannelBodyCredentialDescriptor {
            handle: SecretHandle::new("vendor_webhook_secret").unwrap(),
            pointer: "/secret_token".to_string(),
        }];
        let (egress, transport) = egress_over(declared);
        let mut request = post("https://vendor.example/api/setWebhook");
        request.body_credentials = vec![
            SecretHandle::new("vendor_webhook_secret").unwrap(),
            SecretHandle::new("vendor_webhook_secret").unwrap(),
        ];
        let error = egress.send(request).await.unwrap_err();
        assert!(matches!(
            error,
            RestrictedEgressError::UndeclaredCredential { .. }
        ));
        assert!(transport.approved.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn oversized_transport_response_is_rejected() {
        struct HugeTransport;
        #[async_trait]
        impl ChannelEgressTransport for HugeTransport {
            async fn execute(
                &self,
                _approved: ApprovedChannelEgress,
            ) -> Result<RestrictedEgressResponse, RestrictedEgressError> {
                Ok(RestrictedEgressResponse {
                    status: 200,
                    body: vec![0u8; (CHANNEL_EGRESS_RESPONSE_BODY_LIMIT_BYTES + 1) as usize],
                })
            }
        }
        let egress = PolicyEnforcedChannelEgress::new(
            "vendorx",
            "inst-1",
            declared_vendor(),
            Arc::new(HugeTransport),
        );
        let error = egress
            .send(post("https://vendor.example/api/x"))
            .await
            .unwrap_err();
        assert!(matches!(error, RestrictedEgressError::ResponseTooLarge));
    }
}
