use std::collections::HashMap;

use ironclaw_host_api::{RuntimeCredentialTarget, SecretHandle};
use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;

use crate::{
    ProcessSandboxPlanError, SandboxCredentialBinding, plan::validate_unique_credential_targets,
};

/// Header rewrite performed by the credential broker.
///
/// `old_value` is the placeholder-bearing value from the sandbox request. The
/// replacement secret is retained as `SecretString` so debug output does not
/// expose raw credential material.
#[derive(Debug, Clone)]
pub struct BrokerHeaderRewrite {
    pub name: String,
    pub old_value: String,
    pub new_value: SecretString,
    pub secret_alias: SecretHandle,
}

/// Broker rewrite result containing forwarded headers and audit metadata.
#[derive(Debug, Clone)]
pub struct BrokerRewriteResult {
    pub headers: Vec<(String, String)>,
    pub rewrites: Vec<BrokerHeaderRewrite>,
}

/// Failure returned when a required broker rewrite cannot be completed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BrokerRewriteError {
    #[error("required secret {secret_alias} is missing")]
    MissingRequiredSecret { secret_alias: SecretHandle },
}

/// Credential rewrite policy for brokered sandbox egress.
///
/// The policy matches approved hosts and header placeholders, rewrites them to
/// leased secrets, and provides best-effort redaction for broker-visible error
/// paths.
#[derive(Debug, Clone)]
pub struct SandboxBrokerPolicy {
    bindings: Vec<SandboxCredentialBinding>,
}

impl SandboxBrokerPolicy {
    /// Validates and constructs a broker rewrite policy.
    pub fn new(bindings: Vec<SandboxCredentialBinding>) -> Result<Self, ProcessSandboxPlanError> {
        let policy = Self { bindings };
        for binding in &policy.bindings {
            binding.validate()?;
        }
        validate_unique_credential_targets(&policy.bindings)?;
        Ok(policy)
    }

    /// Rewrites approved placeholder headers for the request host.
    pub fn rewrite_headers(
        &self,
        host: &str,
        headers: Vec<(String, String)>,
        secrets: &HashMap<SecretHandle, SecretString>,
    ) -> Result<BrokerRewriteResult, BrokerRewriteError> {
        let mut rewrites = Vec::new();
        let mut rewritten_headers = Vec::with_capacity(headers.len());
        for (name, value) in headers {
            let Some(binding) = self.matching_header_binding(host, &name, &value) else {
                rewritten_headers.push((name, value));
                continue;
            };
            let Some(secret) = secrets.get(&binding.handle) else {
                if binding.required {
                    return Err(BrokerRewriteError::MissingRequiredSecret {
                        secret_alias: binding.handle.clone(),
                    });
                }
                rewritten_headers.push((name, value));
                continue;
            };
            let RuntimeCredentialTarget::Header { prefix, .. } = &binding.target else {
                rewritten_headers.push((name, value));
                continue;
            };
            let prefix = prefix.as_deref().unwrap_or_default();
            let new_plain = format!("{prefix}{}", secret.expose_secret());
            rewrites.push(BrokerHeaderRewrite {
                name: name.clone(),
                old_value: value,
                new_value: SecretString::from(new_plain.clone()),
                secret_alias: binding.handle.clone(),
            });
            rewritten_headers.push((name, new_plain));
        }
        Ok(BrokerRewriteResult {
            headers: rewritten_headers,
            rewrites,
        })
    }

    /// Redacts leased secret values from text before returning it to callers.
    pub fn sanitize_text(
        &self,
        text: &str,
        secrets: &HashMap<SecretHandle, SecretString>,
    ) -> String {
        let mut values = secrets
            .values()
            .map(|secret| secret.expose_secret())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        values.sort_unstable_by_key(|value| std::cmp::Reverse(value.len()));
        values.into_iter().fold(text.to_string(), |acc, value| {
            acc.replace(value, "[REDACTED]")
        })
    }

    fn matching_header_binding(
        &self,
        host: &str,
        header_name: &str,
        header_value: &str,
    ) -> Option<&SandboxCredentialBinding> {
        let host_without_port = host.split(':').next().unwrap_or(host);
        self.bindings.iter().find(|binding| {
            binding
                .approved_host
                .eq_ignore_ascii_case(host_without_port)
                && match &binding.target {
                    RuntimeCredentialTarget::Header { name, prefix } => {
                        name.eq_ignore_ascii_case(header_name)
                            && header_value
                                == format!(
                                    "{}{}",
                                    prefix.as_deref().unwrap_or_default(),
                                    binding.placeholder_value
                                )
                    }
                    RuntimeCredentialTarget::QueryParam { .. } => false,
                }
        })
    }
}
