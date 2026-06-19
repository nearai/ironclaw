//! Declared-host + credential-handle egress policy enforcement.
//!
//! `EgressPolicy` is the per-installation allow-list. When the host wires a
//! [`ironclaw_product_adapters::ProtocolHttpEgress`] for a v2 adapter, it
//! consults this policy on every request:
//!
//! 1. The target host must be in the adapter's declared host list.
//! 2. The credential handle (if any) must be paired with that host in the
//!    adapter's declared egress targets — i.e. the EXACT
//!    `(host, credential_handle)` pair must appear in the declaration.
//!
//! Storing hosts and handles as independent sets would authorize any allowed
//! handle against any declared host: an adapter declaring
//! `(api.slack.com, slack_bot_token)` and
//! `(api.telegram.org, telegram_bot_token)` would otherwise also permit
//! `(api.telegram.org, slack_bot_token)`, leaking the Slack credential to
//! Telegram. The pair set below pins the cross-pair denial invariant.
//!
//! The host applies the resolved credential at request time; the credential
//! material is never reachable from this struct.

use std::collections::BTreeSet;

use ironclaw_product_adapters::{DeclaredEgressHost, DeclaredEgressTarget, EgressCredentialHandle};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EgressPolicyError {
    #[error("egress to undeclared host {host}")]
    UndeclaredHost { host: DeclaredEgressHost },
    #[error("egress credential handle {handle} is unauthorized for this adapter installation")]
    UnauthorizedCredentialHandle { handle: EgressCredentialHandle },
    #[error(
        "egress credential handle {handle} is not declared for host {host} (declared for a different host in this installation)"
    )]
    CredentialHandleNotPairedWithHost {
        host: DeclaredEgressHost,
        handle: EgressCredentialHandle,
    },
    #[error(
        "unauthenticated egress to {host} is not declared (host is declared only with credentialed pairs)"
    )]
    UnauthenticatedEgressNotDeclared { host: DeclaredEgressHost },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EgressPolicyTarget<'a> {
    pub host: &'a DeclaredEgressHost,
    pub credential_handle: Option<&'a EgressCredentialHandle>,
}

#[derive(Debug, Clone, Default)]
pub struct EgressPolicy {
    /// Set of `(host, Option<credential_handle>)` pairs the adapter
    /// declared. A given host may appear with no credential (egress
    /// without a credential allowed) and/or with one or more specific
    /// credential handles. Membership of the EXACT pair is what
    /// authorizes a request — not membership of the host and handle
    /// independently.
    targets: BTreeSet<(DeclaredEgressHost, Option<EgressCredentialHandle>)>,
}

impl EgressPolicy {
    /// Build a policy from declared egress targets. The canonical
    /// `DeclaredEgressTarget` already carries the `(host, Option<handle>)`
    /// shape — taking it directly ensures the declaration the host
    /// reads from the adapter is the same shape the policy enforces.
    pub fn new(targets: impl IntoIterator<Item = DeclaredEgressTarget>) -> Self {
        Self {
            targets: targets
                .into_iter()
                .map(|t| (t.host, t.credential_handle))
                .collect(),
        }
    }

    pub fn check(&self, target: EgressPolicyTarget<'_>) -> Result<(), EgressPolicyError> {
        // 1. Reject any host that doesn't appear in any declared pair.
        let host_declared = self.targets.iter().any(|(h, _)| h == target.host);
        if !host_declared {
            return Err(EgressPolicyError::UndeclaredHost {
                host: target.host.clone(),
            });
        }
        // 2. Authorize only the EXACT `(host, credential_handle)` pair
        //    the adapter declared. The credential side is symmetric:
        //
        //    a. Request carries no credential. The pair `(host, None)`
        //       must be explicitly declared. Without this check, an
        //       adapter that declared only credentialed pairs (e.g.
        //       `(api.telegram.org, Some(telegram_token))`) could be
        //       bypassed by an unauthenticated request to the same
        //       host — the adapter said "I always send a credential
        //       here," but the policy would let through traffic that
        //       didn't.
        //
        //    b. Request carries a credential. The pair `(host,
        //       Some(handle))` must be declared. Two distinct failure
        //       modes when it isn't:
        //         - The handle is paired with a DIFFERENT host in this
        //           policy — `CredentialHandleNotPairedWithHost`. This
        //           is the cross-pair leak the policy denies (e.g.
        //           `slack_token` against `api.telegram.org`).
        //         - The handle isn't declared for any host —
        //           `UnauthorizedCredentialHandle`. Distinct diagnostic
        //           for the simpler "unknown handle" case.
        match target.credential_handle {
            None => {
                let unauthenticated_declared = self.targets.contains(&(target.host.clone(), None));
                if unauthenticated_declared {
                    Ok(())
                } else {
                    Err(EgressPolicyError::UnauthenticatedEgressNotDeclared {
                        host: target.host.clone(),
                    })
                }
            }
            Some(handle) => {
                let pair_declared = self
                    .targets
                    .contains(&(target.host.clone(), Some(handle.clone())));
                if pair_declared {
                    return Ok(());
                }
                let handle_declared_for_other_host =
                    self.targets.iter().any(|(_, h)| h.as_ref() == Some(handle));
                if handle_declared_for_other_host {
                    return Err(EgressPolicyError::CredentialHandleNotPairedWithHost {
                        host: target.host.clone(),
                        handle: handle.clone(),
                    });
                }
                Err(EgressPolicyError::UnauthorizedCredentialHandle {
                    handle: handle.clone(),
                })
            }
        }
    }

    /// Distinct declared hosts across all `(host, handle)` pairs. A host
    /// declared more than once (e.g. with multiple credential pairings)
    /// appears once.
    pub fn declared_hosts(&self) -> impl Iterator<Item = &DeclaredEgressHost> {
        // BTreeSet keeps pairs ordered, so iterating in order and only
        // emitting a host the first time it appears yields a sorted,
        // de-duplicated view without a temporary collection.
        let mut last: Option<&DeclaredEgressHost> = None;
        self.targets.iter().filter_map(move |(h, _)| {
            if last == Some(h) {
                None
            } else {
                last = Some(h);
                Some(h)
            }
        })
    }

    /// Distinct declared credential handles across all pairs. A handle
    /// declared more than once (across multiple host pairings) appears
    /// once. Pairs with no credential (`None`) are skipped.
    pub fn allowed_credential_handles(&self) -> impl Iterator<Item = &EgressCredentialHandle> {
        let mut seen: BTreeSet<&EgressCredentialHandle> = BTreeSet::new();
        let mut out: Vec<&EgressCredentialHandle> = Vec::new();
        for (_, handle) in &self.targets {
            if let Some(h) = handle.as_ref()
                && seen.insert(h)
            {
                out.push(h);
            }
        }
        out.into_iter()
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

    fn pair(h: &str, c: &str) -> DeclaredEgressTarget {
        DeclaredEgressTarget::new(host(h), Some(handle(c)))
    }

    #[test]
    fn declared_host_with_paired_handle_passes() {
        let policy = EgressPolicy::new([pair("api.telegram.org", "telegram_bot_token")]);
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
        let policy = EgressPolicy::new([pair("api.telegram.org", "telegram_bot_token")]);
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
        let policy = EgressPolicy::new([pair("api.telegram.org", "telegram_bot_token")]);
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

    #[test]
    fn cross_pair_credential_handle_is_denied() {
        // Henry's review: an installation declaring multiple
        // `(host, credential)` pairs must not authorize a credential
        // from one pair against the host of another. The previous
        // implementation, which stored hosts and handles as independent
        // sets, would have allowed `slack_bot_token` to be sent to
        // `api.telegram.org` and vice versa — the canonical
        // cross-pair leak this test pins.
        let policy = EgressPolicy::new([
            pair("api.slack.com", "slack_bot_token"),
            pair("api.telegram.org", "telegram_bot_token"),
        ]);

        let slack_host = host("api.slack.com");
        let telegram_host = host("api.telegram.org");
        let slack_handle = handle("slack_bot_token");
        let telegram_handle = handle("telegram_bot_token");

        // The intended pairs still pass.
        for (h, c) in [
            (&slack_host, &slack_handle),
            (&telegram_host, &telegram_handle),
        ] {
            assert!(
                policy
                    .check(EgressPolicyTarget {
                        host: h,
                        credential_handle: Some(c),
                    })
                    .is_ok(),
                "declared pair must pass: ({}, {})",
                h.as_str(),
                c.as_str(),
            );
        }

        // Cross-pair: slack handle against telegram host.
        let err = policy
            .check(EgressPolicyTarget {
                host: &telegram_host,
                credential_handle: Some(&slack_handle),
            })
            .expect_err("slack handle must not authorize against telegram host");
        match err {
            EgressPolicyError::CredentialHandleNotPairedWithHost { host: h, handle: c } => {
                assert_eq!(h, telegram_host);
                assert_eq!(c, slack_handle);
            }
            other => panic!("expected CredentialHandleNotPairedWithHost, got {other:?}"),
        }

        // Cross-pair: telegram handle against slack host.
        let err = policy
            .check(EgressPolicyTarget {
                host: &slack_host,
                credential_handle: Some(&telegram_handle),
            })
            .expect_err("telegram handle must not authorize against slack host");
        assert!(matches!(
            err,
            EgressPolicyError::CredentialHandleNotPairedWithHost { .. }
        ));
    }

    #[test]
    fn multiple_declared_hosts_and_handles_preserve_typed_policy_membership() {
        let policy = EgressPolicy::new([
            pair("api.slack.com", "slack_bot_token"),
            pair("api.telegram.org", "telegram_bot_token"),
        ]);

        let declared_hosts = policy
            .declared_hosts()
            .map(DeclaredEgressHost::as_str)
            .collect::<Vec<_>>();
        assert_eq!(declared_hosts, ["api.slack.com", "api.telegram.org"]);

        let allowed_handles = policy
            .allowed_credential_handles()
            .map(EgressCredentialHandle::as_str)
            .collect::<Vec<_>>();
        assert_eq!(allowed_handles, ["slack_bot_token", "telegram_bot_token"]);

        let slack_host = host("api.slack.com");
        let slack_handle = handle("slack_bot_token");

        let evil_host = host("evil.example.com");
        let undeclared_err = policy
            .check(EgressPolicyTarget {
                host: &evil_host,
                credential_handle: Some(&slack_handle),
            })
            .expect_err("undeclared host");
        assert_eq!(
            undeclared_err,
            EgressPolicyError::UndeclaredHost { host: evil_host }
        );

        let ghost_handle = handle("ghost_token");
        let unauthorized_err = policy
            .check(EgressPolicyTarget {
                host: &slack_host,
                credential_handle: Some(&ghost_handle),
            })
            .expect_err("unauthorized handle");
        assert_eq!(
            unauthorized_err,
            EgressPolicyError::UnauthorizedCredentialHandle {
                handle: ghost_handle,
            }
        );
    }

    #[test]
    fn unauthenticated_request_to_credential_only_host_is_denied() {
        // Henry's review: when an adapter declares ONLY credentialed
        // pairs for a host (e.g. `(api.telegram.org,
        // Some(telegram_bot_token))`) and never declares the bare
        // `(host, None)` pair, a request that arrives WITHOUT a
        // credential must be rejected. Otherwise a component can
        // bypass the `(host, credential_handle)` contract by sending
        // unauthenticated traffic to a host the adapter explicitly
        // told the host requires a credential.
        let policy = EgressPolicy::new([pair("api.telegram.org", "telegram_bot_token")]);
        let target_host = host("api.telegram.org");
        let err = policy
            .check(EgressPolicyTarget {
                host: &target_host,
                credential_handle: None,
            })
            .expect_err("unauthenticated egress to a credential-only host must fail closed");
        assert_eq!(
            err,
            EgressPolicyError::UnauthenticatedEgressNotDeclared { host: target_host },
        );
    }

    #[test]
    fn host_declared_with_both_pairs_admits_both_request_shapes() {
        // An adapter can declare BOTH `(host, None)` and `(host,
        // Some(handle))` if it wants to allow either egress shape
        // against the same host. Pin this admit-both contract so a
        // future tightening doesn't inadvertently force adapters to
        // pick one.
        let policy = EgressPolicy::new([
            DeclaredEgressTarget::new(host("api.example.com"), None),
            pair("api.example.com", "example_token"),
        ]);
        let target_host = host("api.example.com");
        let target_handle = handle("example_token");
        // Bare request passes.
        assert!(
            policy
                .check(EgressPolicyTarget {
                    host: &target_host,
                    credential_handle: None,
                })
                .is_ok()
        );
        // Credentialed request passes.
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
    fn host_declared_without_credential_allows_handle_free_request() {
        // A pair with `credential_handle = None` declares that the host
        // may be reached without sending a credential. Such requests
        // must pass; declarations are explicit, not catch-all.
        let policy = EgressPolicy::new([DeclaredEgressTarget::new(
            host("api.public.example.com"),
            None,
        )]);
        let target_host = host("api.public.example.com");
        assert!(
            policy
                .check(EgressPolicyTarget {
                    host: &target_host,
                    credential_handle: None,
                })
                .is_ok()
        );
    }
}
