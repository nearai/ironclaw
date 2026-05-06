//! Declared-host + credential-handle egress policy enforcement.
//!
//! `EgressPolicy` is the per-installation allow-list. When the host wires a
//! [`ironclaw_product_adapters::ProtocolHttpEgress`] for a v2 adapter, it
//! consults this policy on every request:
//!
//! 1. The target host must be in the adapter's declared host list.
//! 2. The credential handle (if any) must be one the policy was told this
//!    adapter installation may consume.
//!
//! The host applies the resolved credential at request time; the credential
//! material is never reachable from this struct.

use std::collections::BTreeSet;

use ironclaw_product_adapters::{DeclaredEgressHost, EgressCredentialHandle};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EgressPolicyError {
    #[error("egress to undeclared host {host}")]
    UndeclaredHost { host: String },
    #[error("egress credential handle {handle} is unauthorized for this adapter installation")]
    UnauthorizedCredentialHandle { handle: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EgressPolicyTarget<'a> {
    pub host: &'a DeclaredEgressHost,
    pub credential_handle: Option<&'a EgressCredentialHandle>,
}

#[derive(Debug, Clone, Default)]
pub struct EgressPolicy {
    declared_hosts: BTreeSet<String>,
    allowed_credential_handles: BTreeSet<String>,
}

impl EgressPolicy {
    pub fn new(
        declared_hosts: impl IntoIterator<Item = DeclaredEgressHost>,
        allowed_credential_handles: impl IntoIterator<Item = EgressCredentialHandle>,
    ) -> Self {
        Self {
            declared_hosts: declared_hosts
                .into_iter()
                .map(|h| h.as_str().to_string())
                .collect(),
            allowed_credential_handles: allowed_credential_handles
                .into_iter()
                .map(|h| h.as_str().to_string())
                .collect(),
        }
    }

    pub fn check(&self, target: EgressPolicyTarget<'_>) -> Result<(), EgressPolicyError> {
        if !self.declared_hosts.contains(target.host.as_str()) {
            return Err(EgressPolicyError::UndeclaredHost {
                host: target.host.as_str().to_string(),
            });
        }
        if let Some(handle) = target.credential_handle
            && !self.allowed_credential_handles.contains(handle.as_str())
        {
            return Err(EgressPolicyError::UnauthorizedCredentialHandle {
                handle: handle.as_str().to_string(),
            });
        }
        Ok(())
    }

    pub fn declared_hosts(&self) -> impl Iterator<Item = &str> {
        self.declared_hosts.iter().map(String::as_str)
    }

    pub fn allowed_credential_handles(&self) -> impl Iterator<Item = &str> {
        self.allowed_credential_handles.iter().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(value: &str) -> DeclaredEgressHost {
        DeclaredEgressHost::new(value).expect("valid")
    }

    fn handle(value: &str) -> EgressCredentialHandle {
        EgressCredentialHandle::new(value).expect("valid")
    }

    #[test]
    fn declared_host_with_known_handle_passes() {
        let policy = EgressPolicy::new([host("api.telegram.org")], [handle("telegram_bot_token")]);
        let target_host = host("api.telegram.org");
        let target_handle = handle("telegram_bot_token");
        assert!(
            policy
                .check(EgressPolicyTarget {
                    host: &target_host,
                    credential_handle: Some(&target_handle),
                })
                .is_ok()
        );
    }

    #[test]
    fn undeclared_host_fails_closed() {
        let policy = EgressPolicy::new([host("api.telegram.org")], [handle("telegram_bot_token")]);
        let other = host("evil.example.com");
        let err = policy
            .check(EgressPolicyTarget {
                host: &other,
                credential_handle: None,
            })
            .expect_err("undeclared");
        assert!(matches!(err, EgressPolicyError::UndeclaredHost { .. }));
    }

    #[test]
    fn unknown_handle_fails_closed_even_for_declared_host() {
        let policy = EgressPolicy::new([host("api.telegram.org")], [handle("telegram_bot_token")]);
        let target_host = host("api.telegram.org");
        let target_handle = handle("ghost_token");
        let err = policy
            .check(EgressPolicyTarget {
                host: &target_host,
                credential_handle: Some(&target_handle),
            })
            .expect_err("unauthorized handle");
        assert!(matches!(
            err,
            EgressPolicyError::UnauthorizedCredentialHandle { .. }
        ));
    }
}
