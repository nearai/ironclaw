use std::collections::HashMap;

use ironclaw_host_api::{RuntimeCredentialTarget, SecretHandle};
use secrecy::{ExposeSecret, SecretString};

use crate::{SandboxCredentialBinding, SandboxPlanError};

#[derive(Debug, Clone)]
pub struct BrokerHeaderRewrite {
    pub name: String,
    pub old_value: String,
    pub new_value: SecretString,
    pub secret_alias: SecretHandle,
}

#[derive(Debug, Clone)]
pub struct BrokerRewriteResult {
    pub headers: Vec<(String, String)>,
    pub rewrites: Vec<BrokerHeaderRewrite>,
}

#[derive(Debug, Clone)]
pub struct SandboxBrokerPolicy {
    bindings: Vec<SandboxCredentialBinding>,
}

impl SandboxBrokerPolicy {
    pub fn new(bindings: Vec<SandboxCredentialBinding>) -> Result<Self, SandboxPlanError> {
        let policy = Self { bindings };
        for binding in &policy.bindings {
            binding.validate()?;
        }
        Ok(policy)
    }

    pub fn rewrite_headers(
        &self,
        host: &str,
        headers: Vec<(String, String)>,
        secrets: &HashMap<SecretHandle, SecretString>,
    ) -> BrokerRewriteResult {
        let mut rewrites = Vec::new();
        let rewritten_headers = headers
            .into_iter()
            .map(|(name, value)| {
                let Some(binding) = self.matching_header_binding(host, &name, &value) else {
                    return (name, value);
                };
                let Some(secret) = secrets.get(&binding.handle) else {
                    return (name, value);
                };
                let RuntimeCredentialTarget::Header { prefix, .. } = &binding.target else {
                    return (name, value);
                };
                let prefix = prefix.as_deref().unwrap_or_default();
                let new_plain = format!("{prefix}{}", secret.expose_secret());
                rewrites.push(BrokerHeaderRewrite {
                    name: name.clone(),
                    old_value: value,
                    new_value: SecretString::from(new_plain.clone()),
                    secret_alias: binding.handle.clone(),
                });
                (name, new_plain)
            })
            .collect();
        BrokerRewriteResult {
            headers: rewritten_headers,
            rewrites,
        }
    }

    pub fn sanitize_text(
        &self,
        text: &str,
        secrets: &HashMap<SecretHandle, SecretString>,
    ) -> String {
        secrets.values().fold(text.to_string(), |acc, secret| {
            let value = secret.expose_secret();
            if value.is_empty() {
                acc
            } else {
                acc.replace(value, "[REDACTED]")
            }
        })
    }

    fn matching_header_binding(
        &self,
        host: &str,
        header_name: &str,
        header_value: &str,
    ) -> Option<&SandboxCredentialBinding> {
        self.bindings.iter().find(|binding| {
            binding.approved_host.eq_ignore_ascii_case(host)
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
