# ironclaw_trust guardrails

- Own host-controlled trust evaluation only: `EffectiveTrustClass`, `TrustPolicy`, layered `PolicySource`s, and the trust-change invalidation contract.
- Privileged variants of `EffectiveTrustClass` (FirstParty, System) MUST only be constructible from inside this crate. Public constructors expose Sandbox and UserTrusted only.
- Do not import any other `ironclaw_*` crate besides `ironclaw_host_api`. No dispatcher, capability host, runtimes, host runtime, approvals, run-state, processes, events, resources, or product workflow.
- Manifest input always flows through `RequestedTrustClass` and `PackageIdentity` from `host_api`. Manifest deserialization paths must never construct an `EffectiveTrustClass` directly.
- Trust downgrade or revocation must publish on `InvalidationBus` synchronously, before any subsequent `evaluate()` returns the new lower decision — fail-closed.
- `TrustClass` ceiling alone grants no capability authority. Authorization must consume both an `EffectiveTrustClass` *and* an explicit `CapabilityGrant`.
- Identity drift (`package_id`, `source`, `digest`, `signer`) or requested-authority growth invalidates retained grants; PR3 will use the helpers exposed here.
