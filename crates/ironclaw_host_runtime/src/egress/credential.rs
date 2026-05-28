use ironclaw_host_api::{
    CapabilityId, RuntimeCredentialInjection, RuntimeCredentialSource, RuntimeCredentialTarget,
    RuntimeHttpEgressError, RuntimeHttpEgressRequest, SecretHandle,
};
use ironclaw_network::is_rfc3986_unreserved_segment;
use ironclaw_safety::redaction_values_for_secret;
use ironclaw_secrets::{SecretMaterial, SecretStore, SecretStoreError};
use secrecy::ExposeSecret;

use crate::obligations::RuntimeSecretInjectionStore;

#[derive(Clone, PartialEq, Eq)]
enum CredentialCacheKey {
    SecretStoreLease {
        handle: SecretHandle,
    },
    StagedObligation {
        capability_id: CapabilityId,
        handle: SecretHandle,
    },
}

struct CredentialCacheEntry {
    key: CredentialCacheKey,
    /// Leased secret material kept inside `SecretString` so the bytes are
    /// zeroized when this entry — and its enclosing `Vec` — is dropped at
    /// the end of the egress call. Holding plaintext as `String` here
    /// instead would leave the leased credential on the heap for the
    /// duration of the request, defeating `SecretMaterial::ZeroizeOnDrop`.
    value: Option<SecretMaterial>,
}

enum CredentialSourceStrategy<'a> {
    SecretStoreLease,
    StagedObligation { capability_id: &'a CapabilityId },
}

impl<'a> CredentialSourceStrategy<'a> {
    fn for_injection(injection: &'a RuntimeCredentialInjection) -> Self {
        match &injection.source {
            RuntimeCredentialSource::SecretStoreLease => Self::SecretStoreLease,
            RuntimeCredentialSource::StagedObligation { capability_id } => {
                Self::StagedObligation { capability_id }
            }
        }
    }

    fn validate_for_request(
        &self,
        request: &RuntimeHttpEgressRequest,
        allow_direct_secret_lease: bool,
    ) -> Result<(), RuntimeHttpEgressError> {
        match self {
            Self::SecretStoreLease if !allow_direct_secret_lease => {
                Err(RuntimeHttpEgressError::Credential {
                    reason:
                        "direct secret-store leases are unavailable for production runtime egress"
                            .to_string(),
                })
            }
            Self::SecretStoreLease => Ok(()),
            Self::StagedObligation { capability_id }
                if *capability_id != &request.capability_id =>
            {
                Err(RuntimeHttpEgressError::Credential {
                    reason: "staged credential capability does not match request capability"
                        .to_string(),
                })
            }
            Self::StagedObligation { .. } => Ok(()),
        }
    }

    fn cache_key(&self, injection: &RuntimeCredentialInjection) -> CredentialCacheKey {
        match self {
            Self::SecretStoreLease => CredentialCacheKey::SecretStoreLease {
                handle: injection.handle.clone(),
            },
            Self::StagedObligation { capability_id } => CredentialCacheKey::StagedObligation {
                capability_id: (*capability_id).clone(),
                handle: injection.handle.clone(),
            },
        }
    }

    fn resolve<S>(
        &self,
        secrets: &S,
        secret_injections: Option<&RuntimeSecretInjectionStore>,
        request: &RuntimeHttpEgressRequest,
        injection: &RuntimeCredentialInjection,
    ) -> Result<Option<SecretMaterial>, RuntimeHttpEgressError>
    where
        S: SecretStore,
    {
        match self {
            Self::SecretStoreLease => lease_secret_for_injection(secrets, request, injection),
            Self::StagedObligation { capability_id } => take_staged_secret_for_injection(
                secret_injections,
                request,
                capability_id,
                injection,
            ),
        }
    }
}

pub(super) fn validate_sources_for_request(
    request: &RuntimeHttpEgressRequest,
    allow_direct_secret_lease: bool,
) -> Result<(), RuntimeHttpEgressError> {
    for injection in &request.credential_injections {
        CredentialSourceStrategy::for_injection(injection)
            .validate_for_request(request, allow_direct_secret_lease)?;
    }
    Ok(())
}

pub(super) fn apply_credential_injections<S>(
    secrets: &S,
    secret_injections: Option<&RuntimeSecretInjectionStore>,
    allow_direct_secret_lease: bool,
    request: &mut RuntimeHttpEgressRequest,
) -> Result<Vec<String>, RuntimeHttpEgressError>
where
    S: SecretStore,
{
    let mut redaction_values = Vec::new();
    let mut credential_materials = Vec::new();
    let mut parsed_url = None;
    let credential_injections = std::mem::take(&mut request.credential_injections);
    for injection in &credential_injections {
        let value = credential_value_for_injection(
            &mut credential_materials,
            secrets,
            secret_injections,
            allow_direct_secret_lease,
            request,
            injection,
        )?;
        let Some(value) = value else {
            continue;
        };
        // Borrow the leased plaintext only for the narrow window where the
        // egress code needs it (header/query injection + redaction-token
        // generation). The `SecretMaterial` stays inside the cache; the
        // exposed `&str` does not outlive this loop iteration. Plaintext
        // does land in `request.headers` and `redaction_values` after
        // injection because the network layer and response-body scanner
        // consume raw bytes, but the cache itself never holds a non-zeroizing
        // copy.
        let plaintext = value.expose_secret();
        apply_credential_injection(request, &mut parsed_url, &injection.target, plaintext)?;
        redaction_values.extend(redaction_values_for_secret(plaintext));
    }
    if let Some(url) = parsed_url {
        request.url = url.to_string();
    }
    Ok(redaction_values)
}

fn credential_value_for_injection<'cache, S>(
    cache: &'cache mut Vec<CredentialCacheEntry>,
    secrets: &S,
    secret_injections: Option<&RuntimeSecretInjectionStore>,
    allow_direct_secret_lease: bool,
    request: &RuntimeHttpEgressRequest,
    injection: &RuntimeCredentialInjection,
) -> Result<Option<&'cache SecretMaterial>, RuntimeHttpEgressError>
where
    S: SecretStore,
{
    let strategy = CredentialSourceStrategy::for_injection(injection);
    strategy.validate_for_request(request, allow_direct_secret_lease)?;
    let key = strategy.cache_key(injection);
    if let Some(idx) = cache.iter().position(|entry| entry.key == key) {
        // Negative cache hit (missing optional credential on a prior pass)
        // must still error out if *this* injection marks the same handle as
        // required. `required` is per-injection, not per-cache-entry.
        if cache[idx].value.is_none() && injection.required {
            return Err(RuntimeHttpEgressError::Credential {
                reason: "required credential is unavailable".to_string(),
            });
        }
        return Ok(cache[idx].value.as_ref());
    }

    let value = strategy.resolve(secrets, secret_injections, request, injection)?;
    let pushed_index = cache.len();
    cache.push(CredentialCacheEntry { key, value });
    Ok(cache[pushed_index].value.as_ref())
}

fn take_staged_secret_for_injection(
    secret_injections: Option<&RuntimeSecretInjectionStore>,
    request: &RuntimeHttpEgressRequest,
    capability_id: &CapabilityId,
    injection: &RuntimeCredentialInjection,
) -> Result<Option<SecretMaterial>, RuntimeHttpEgressError> {
    let Some(secret_injections) = secret_injections else {
        return missing_runtime_credential(injection.required);
    };
    match secret_injections.take(&request.scope, capability_id, &injection.handle) {
        Ok(Some(material)) => Ok(Some(material)),
        Ok(None) => missing_runtime_credential(injection.required),
        Err(_) => Err(RuntimeHttpEgressError::Credential {
            reason: "runtime credential injection store unavailable".to_string(),
        }),
    }
}

fn missing_runtime_credential(
    required: bool,
) -> Result<Option<SecretMaterial>, RuntimeHttpEgressError> {
    if required {
        Err(RuntimeHttpEgressError::Credential {
            reason: "required credential is unavailable".to_string(),
        })
    } else {
        Ok(None)
    }
}

fn lease_secret_for_injection<S>(
    secrets: &S,
    request: &RuntimeHttpEgressRequest,
    injection: &RuntimeCredentialInjection,
) -> Result<Option<SecretMaterial>, RuntimeHttpEgressError>
where
    S: SecretStore,
{
    match block_on_secret_store(async {
        let metadata = secrets.metadata(&request.scope, &injection.handle).await?;
        if metadata.is_none() {
            return Ok(None);
        }
        let lease = secrets
            .lease_once(&request.scope, &injection.handle)
            .await?;
        secrets.consume(&request.scope, lease.id).await.map(Some)
    }) {
        Ok(Some(material)) => Ok(Some(material)),
        Ok(None) => missing_runtime_credential(injection.required),
        Err(SecretStoreError::UnknownSecret { .. }) => {
            missing_runtime_credential(injection.required)
        }
        Err(error) => Err(RuntimeHttpEgressError::Credential {
            reason: sanitized_secret_error(&error),
        }),
    }
}

fn block_on_secret_store<T>(
    future: impl std::future::Future<Output = Result<T, SecretStoreError>> + Send,
) -> Result<T, SecretStoreError>
where
    T: Send,
{
    let joined = std::thread::scope(|scope| {
        scope
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|_| SecretStoreError::StoreUnavailable {
                        reason: "secret store runtime unavailable".to_string(),
                    })?;
                runtime.block_on(future)
            })
            .join()
    });
    joined.unwrap_or_else(|_| {
        Err(SecretStoreError::StoreUnavailable {
            reason: "secret store worker panicked".to_string(),
        })
    })
}

fn sanitized_secret_error(error: &SecretStoreError) -> String {
    match error {
        SecretStoreError::UnknownSecret { .. } => "credential is unavailable".to_string(),
        SecretStoreError::UnknownLease { .. } => "credential lease is unavailable".to_string(),
        SecretStoreError::LeaseConsumed { .. } => "credential lease was already used".to_string(),
        SecretStoreError::LeaseRevoked { .. } => "credential lease was revoked".to_string(),
        SecretStoreError::LeaseExpired { .. } | SecretStoreError::SecretExpired => {
            "credential expired".to_string()
        }
        SecretStoreError::BackendMisconfigured { .. } => {
            "credential store is misconfigured".to_string()
        }
        SecretStoreError::StoreUnavailable { .. } => "credential store unavailable".to_string(),
    }
}

fn apply_credential_injection(
    request: &mut RuntimeHttpEgressRequest,
    parsed_url: &mut Option<url::Url>,
    target: &RuntimeCredentialTarget,
    value: &str,
) -> Result<(), RuntimeHttpEgressError> {
    target
        .validate_declaration()
        .map_err(|_| RuntimeHttpEgressError::Credential {
            reason: "credential injection target is invalid".to_string(),
        })?;
    match target {
        RuntimeCredentialTarget::Header { name, prefix } => {
            let injected = match prefix {
                Some(prefix) => format!("{prefix}{value}"),
                None => value.to_string(),
            };
            if injected.chars().any(char::is_control) {
                return Err(RuntimeHttpEgressError::Credential {
                    reason: "credential injection header value is invalid".to_string(),
                });
            }
            request.headers.push((name.clone(), injected));
        }
        RuntimeCredentialTarget::QueryParam { name } => {
            let url = parsed_request_url(&request.url, parsed_url)?;
            url.query_pairs_mut().append_pair(name, value);
        }
        RuntimeCredentialTarget::PathPlaceholder { placeholder } => {
            if !is_rfc3986_unreserved_segment(placeholder) {
                return Err(RuntimeHttpEgressError::Credential {
                    reason: "credential injection path placeholder is invalid".to_string(),
                });
            }
            if !is_rfc3986_unreserved_segment(value) {
                return Err(RuntimeHttpEgressError::Credential {
                    reason: "credential injection path value is invalid".to_string(),
                });
            }
            let url = parsed_request_url(&request.url, parsed_url)?;
            if url.scheme() != "https" {
                return Err(RuntimeHttpEgressError::Credential {
                    reason: "credential injection path placeholder requires HTTPS".to_string(),
                });
            }
            let Some(_) = url.path_segments() else {
                return Err(RuntimeHttpEgressError::Credential {
                    reason: "credential injection target URL has no path segments".to_string(),
                });
            };
            let path = url.path().to_string();
            let path = path.strip_prefix('/').unwrap_or(&path);
            let placeholder_count = path
                .split('/')
                .filter(|segment| *segment == placeholder)
                .count();
            match placeholder_count {
                0 => {
                    return Err(RuntimeHttpEgressError::Credential {
                        reason: "credential injection path placeholder was not found".to_string(),
                    });
                }
                1 => {}
                _ => {
                    return Err(RuntimeHttpEgressError::Credential {
                        reason: "credential injection path placeholder must appear exactly once"
                            .to_string(),
                    });
                }
            }
            let mut rewritten_path = String::with_capacity(path.len() + value.len());
            for (index, segment) in path.split('/').enumerate() {
                if index > 0 {
                    rewritten_path.push('/');
                }
                if segment == placeholder {
                    rewritten_path.push_str(value);
                } else {
                    rewritten_path.push_str(segment);
                }
            }
            url.set_path(&rewritten_path);
        }
    }
    Ok(())
}

fn parsed_request_url<'a>(
    raw_url: &str,
    parsed_url: &'a mut Option<url::Url>,
) -> Result<&'a mut url::Url, RuntimeHttpEgressError> {
    if parsed_url.is_none() {
        *parsed_url =
            Some(
                url::Url::parse(raw_url).map_err(|_| RuntimeHttpEgressError::Credential {
                    reason: "credential injection target URL is invalid".to_string(),
                })?,
            );
    }
    parsed_url
        .as_mut()
        .ok_or_else(|| RuntimeHttpEgressError::Credential {
            reason: "credential injection target URL is invalid".to_string(),
        })
}

/// The credential material cache value field must hold a `ZeroizeOnDrop`
/// carrier. Holding the leased plaintext as `Option<String>` (the original
/// bug) leaves it on the heap until the cache `Vec` is dropped at end-of-call,
/// then frees the bytes without wiping. This `const _: fn(...) = ...`
/// references the field's inner type through a `ZeroizeOnDrop`-bounded helper,
/// so any refactor that downgrades the field to a non-zeroizing type (e.g.
/// plain `Option<String>`) stops the crate from compiling rather than waiting
/// for a test run. `String` implements `Zeroize` but not `ZeroizeOnDrop`, so
/// the constraint fires on exactly the bug shape this guard protects against.
/// The function is never called — only type-checked.
const _: fn(&CredentialCacheEntry) = |entry| {
    fn require_zeroize_on_drop<T: ?Sized + secrecy::zeroize::ZeroizeOnDrop>(_: &T) {}
    if let Some(value) = entry.value.as_ref() {
        require_zeroize_on_drop(value);
    }
};
