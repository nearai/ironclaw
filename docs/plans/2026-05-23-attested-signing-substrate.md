# IronClaw Attested-Signing / Secure-Channel Substrate — Consolidated Plan (v3)

## Goal

Let the agent get high-value blockchain transactions (Solana, NEAR, EVM)
signed under a human-in-the-loop gate, via a provider-agnostic
`SigningProvider` trait.

## Fixed Decisions

- **Chains:** Solana, NEAR, EVM.
- **Unifying abstraction:** `SigningProvider` (WalletConnector) trait; the gate
  machinery is provider-agnostic.
- **Four backends, all v1:**
  - WalletConnect v2.
  - Browser injected provider (`window.ethereum` / `window.solana`, web channel).
  - NEAR browser wallet protocol (Wallet Selector / redirect).
  - Custodial + WebAuthn (IronClaw holds keys behind
    `ironclaw_secrets` / `SecretsCrypto`; a passkey assertion authorizes
    signing).
- **External wallet PREFERRED; custodial is FALLBACK.** The external path is
  true wallet-side WYSIWYS ("what you see is what you sign"). Custodial WYSIWYS
  is an IronClaw-rendered view + WebAuthn presence (documented weaker
  limitation).

## Two Trust Models

| Trust model | Key custody | Render + sign | IronClaw role |
|-------------|-------------|---------------|---------------|
| **External-wallet** | User's wallet holds keys | Wallet renders and signs the real tx | IronClaw broadcasts / tracks only; no custody |
| **Custodial** | IronClaw holds keys | WebAuthn assertion authorizes; IronClaw renders | Mainnet / real-value gated on HSM/KMS |

## Load-Bearing Security Invariants (every PR honors)

- **Sealed one-shot `AttestedSigningGrant`** keyed by
  `(tenant, user, run_id, gate_ref, approved_tx_hash, key/account, chain_id)`,
  claimed once via an atomic CAS inside the signer. The approved hash is NOT
  itself authorization.
- **`ApprovedTxHash` binds** render ∥ canonical signing bytes ∥ signer/account ∥
  chain/network ∥ tx-type ∥ rendering-schema-version (domain-separated CBOR).
  EVM `from` is recovered from the signature (k256 ecrecover) and compared to
  the bound account.
- **Broadcast idempotency ledger**
  (`approved → signing → signed → broadcast_submitted → finalized | unknown |
  manual_review`), one-shot per `gate_ref`, keyed on ledger state NOT job
  state. Never retry with a fresh EVM nonce / Solana blockhash / NEAR nonce
  without re-approval.
- **Deterministic post-approval continuation:** a `BlockedAttested` resume must
  NOT requeue the LLM loop; `resume_turn_once` (crypto-free `ironclaw_turns`)
  only validates and transitions; signer continuation (chain I/O) lives in the
  reborn/runner layer.
- **Full WebAuthn RP validation (custodial):** UV required, `type ==
  "webauthn.get"`, echoed challenge, `rpIdHash`, origin / topOrigin, signCount
  regression, AAGUID / attestation policy. Durable fail-closed pre-sign /
  pre-broadcast audit (the existing `SecurityAuditSink` is best-effort; add
  `SecurityBoundary` variants `Attestation` / `CustodyKeyAccess` /
  `ChainSigning` / `BroadcastSubmit` + a stronger durable contract).
- **HSM/KMS ship-gate:** custodial mainnet is refused unless KMS is wired
  (mirror `HOOKS_THIRD_PARTY_ENABLED`). Hot-key custodial = testnet/dev only.

## Multi-Tenant Operating Model

The substrate is multi-tenant by construction: tenant identity is woven into the
primitives, not bolted on. `TenantId` is the **first field** of both
`SigningContext` (`ironclaw_signing_provider`) and `GrantKey`
(`ironclaw_attestation`), so every downstream artifact carries it.

### Isolation backbone (enforced today, fail-closed)

| Surface | How tenant isolation is enforced |
|---------|----------------------------------|
| **One-shot grant** | The sealed-grant `key_hash` (PRIMARY KEY of the durable store) is `grant_key_hash(GrantKey)` over the full 7-tuple **including `tenant`**; the durable row also carries an explicit `tenant` column. The atomic CAS claim is therefore tenant-scoped — tenant A's gate yields a different `key_hash` than B's, so one tenant can never claim or replay another's grant. |
| **Custodial keys** | Encrypted with `chain_key_aad(scope, chain)` = AES-256-GCM AAD over `tenant_id ∥ user_id ∥ agent_id ∥ project_id ∥ chain` (owner-scope only — independent of invocation/mission/thread). A key sealed for tenant A **cannot decrypt** under tenant B's AAD (authentication-tag failure). The master key is an **operator/instance-level** secret; per-(tenant,user,…) custodial keys are stored ciphertexts under it. |
| **Ledger / challenge / binding stores** | Keyed by `gate_ref` (ledger/binding) or `GrantKey` (challenge). These stores are **not** tenant-columned: the durable `attested_signing_ledger` and `attested_gate_bindings` schemas are keyed purely by `gate_ref` (PRIMARY KEY), and the `SigningLedger`/binding trait APIs carry no tenant component. Tenant isolation here is inherited, not enforced at the row: `gate_ref` MUST be globally unique and tenant-qualified, but **this is a caller obligation NOT enforced by the store or the `GateRef`/`SigningGateRef` newtype** (the newtype is a transparent string with no prefix validation, and `put`/`create` collapse a colliding key via `ON CONFLICT DO NOTHING` / `AlreadyExists` rather than rejecting a tenant-prefix-less ref). If two tenants' gate-raising paths ever produced the same `gate_ref` string, the second write would collide with the first's row, so the gate-raising path is solely responsible for uniqueness. Adding a `tenant` column to both durable schemas (threaded through the `SigningLedger`/`AttestedGateBindingStore` traits) or enforcing a validated tenant prefix at `GateRef::new` is tracked as a follow-up; until then this row is protected only by that caller contract. Per-tenant binding/grant *lookup* is additionally fail-closed because the binding's bound account + the tenant-scoped grant `key_hash` must both match (see "Gate raise → resolve" / "One-shot grant"), so even a collided row cannot be turned into a cross-tenant signature. |
| **Gate raise → resolve** | `request_signature` stamps the authenticated tenant/user into `SigningContext`; the binding persists under that tenant. The reborn webui ingress authenticates the caller (`tenant_id`+`user_id`); the binding/grant lookup is tenant-scoped, so a cross-tenant resolve fails closed (`MissingBinding` / `SignerMismatch` / grant-CAS). |
| **WebAuthn (custodial presence)** | Credentials are registered per user (`userHandle` binding); a credential for user A cannot satisfy user B's gate. |
| **External wallets** | The signer/account proven by the wallet must equal the grant's **bound** account, so a wallet connected by tenant A's user cannot authorize tenant B's gate. WalletConnect `ProjectId` is a **shared app identity** (config, not a per-tenant secret); per-tenant override is for quota/attribution only. Injected / NEAR-redirect are inherently per-user-session. |
| **Audit** | The `ActionRecord` is per-tenant/user — the trail isolates by construction. |

**Invariant:** cross-tenant signing, replay, decryption, and resolution all fail
closed at the primitive layer (grant key, key-encryption AAD, gate binding). This
is the load-bearing guarantee and is covered by dedicated cross-tenant tests.

### Operational gaps remaining for production multi-tenancy

The *isolation* model above is complete and enforced. The *operational* story is
not, and splits into three tracks:

1. **Production multi-tenant runtime (gap D — reborn-wide).** `build_reborn_runtime`
   rejects non-local-dev, so the durable composition + per-tenant wiring is not yet
   assembled in a running multi-tenant server. The durable assembly seam
   (`assemble_libsql` / `assemble_postgres`) is built and tested but not driven.
   End-to-end operation today is local-dev/custodial only. **Owner: reborn runtime
   track**, not this substrate.
2. **Per-tenant configuration.** `AttestedProvidersConfig::from_env` resolves one
   NEAR `state_secret`, one WalletConnect `ProjectId`, and one RPC-endpoint set for
   the whole instance. A multi-tenant deployment needs a
   `TenantId → AttestedProvidersConfig` resolver (per-tenant override falling back
   to instance defaults), and per-tenant KMS key-ref selection (the custodial keystore
   already AAD-scopes by tenant; the `LocalKmsSigner` is keyed by opaque `key_ref`,
   so per-tenant key-refs fit — provisioning is the open piece). **Tractable; see
   the per-tenant-config PR.**
3. **Per-tenant key/credential lifecycle.** The items in *Open Questions* below are
   all per-tenant lifecycle concerns and require design decisions before
   implementation: custodial first-key bootstrap, WebAuthn first-credential
   bootstrap, connected-wallet trust registration, key rotation, custody
   recovery/backup, the KMS mainnet threshold, multi-sig/quorum, WC session TTL.

## Crates

| Crate | Responsibility |
|-------|----------------|
| `ironclaw_signing_provider` | The trait, no chain deps — **THIS PR**. Pins the binding model. |
| `ironclaw_attestation` | WebAuthn + `DecodedTransaction` / render / canonical + grant + ledger + challenge + audit. |
| `ironclaw_chain_signing` | Custodial keys behind secrets; per-chain decode / render / sign / broadcast. |
| `ironclaw_wallet_external` | WC v2 / injected / NEAR-redirect. |

`ironclaw_turns` gets only `BlockedReason::Attested` + `TurnStatus::BlockedAttested`
+ an injected `AttestedResumePort` (it stays crypto-free; persists only opaque
refs + `ApprovedTxHash`). Proof ingress = the existing
`POST /api/chat/gate/resolve`.

## 10-PR Stack

1. **PR1 — trait + boundary test (THIS).**
2. PR2 — canonical / hash.
3. PR3 — grant + ledger.
4. PR4 — challenge + WebAuthn + audit.
5. PR5 — turns `BlockedAttested` + resume split.
6. PR6 — `chain_signing` custodial + per-chain.
7. PR7 — injected provider + web wiring.
8. PR8 — NEAR redirect.
9. PR9 — WalletConnect.
10. PR10 — ship-gates + KMS stub + threat matrix.

PR1–7 are unblocked; WC / NEAR are isolated last.

## Threat Matrix

Each threat maps to a fail-closed mechanism.

| # | Threat | Fail-closed mechanism |
|---|--------|-----------------------|
| 1 | Sealed-grant replay | One-shot grant claimed via atomic CAS inside the signer |
| 2 | Caller-supplied tx | Tx decoded / canonicalized server-side; caller cannot inject bytes |
| 3 | Caller-supplied hash | `ApprovedTxHash` recomputed from canonical bytes, not trusted from caller |
| 4 | Caller-supplied key | Key/account bound into the grant key and re-checked at sign time |
| 5 | EVM `from` spoof | `from` recovered from signature (k256 ecrecover), compared to bound account |
| 6 | Broadcast retry | Idempotency ledger one-shot per `gate_ref`; keyed on ledger state |
| 7 | `Stuck → InProgress` double-broadcast | Ledger state (not job state) gates broadcast |
| 8 | Hash field-smuggling | Domain-separated CBOR over a fixed field set |
| 9 | Hidden-field attack | Canonical encoding rejects unknown / extra fields |
| 10 | Untrusted RPC / token metadata | Render derived from canonical decode, not RPC-supplied display data |
| 11 | Narrow challenge reuse | Single-use challenge bound to the specific gate |
| 12 | WebAuthn UP-only | UV required, not just UP |
| 13 | WebAuthn type-confusion | `type == "webauthn.get"` enforced |
| 14 | WebAuthn signCount regression | signCount monotonicity enforced |
| 15 | WebAuthn foreign-credential | rpIdHash / origin / topOrigin / AAGUID validation |
| 16 | LLM-loop reinterpretation | Resume only validates + transitions; never requeues the loop |
| 17 | Dropped audit | Durable fail-closed pre-sign / pre-broadcast audit contract |
| 18 | Compromised-host hot-key | HSM/KMS ship-gate; hot-key custodial = testnet/dev only |
| 19 | WC pairing-phishing | Pairing scope validated against the requested operation |
| 20 | WC relay-compromise / session-hijack | Session binding + scope re-check; deep-link interception hardening |
| 21 | WC scope-escalation | Requested scope compared against approved operation |
| 22 | NEAR access-key scope | Access-key scope validated against the bound account / operation |

(Includes deep-link interception under #20 and WC session-hijack under #20.)

## Dependency Table

| Dependency | Used by | Notes |
|------------|---------|-------|
| `alloy` (modular sub-crates) | `ironclaw_chain_signing` (EVM) | Pull only needed sub-crates |
| `k256` | EVM ecrecover | `from` recovery |
| `sha3` | EVM / keccak | Hashing |
| `solana-sdk` / `solana-program` | Solana | Heaviest dependency |
| `near-primitives` / `near-crypto` | NEAR | |
| `webauthn-rs` | `ironclaw_attestation` | MPL-2.0 — confirm license compatibility |
| `walletconnect` fork | `ironclaw_wallet_external` | Fork from issue #1739 (see Open Questions) |
| `borsh` + `ed25519-dalek` | Solana / NEAR | Already vendored |

**`ironclaw_signing_provider` (this PR) depends on NONE of the above.** It is a
pure trait crate; the dependency-boundary test enforces that it carries no
chain / crypto / secrets dependency.

## Open Questions

- The **WalletConnect fork from issue #1739 does NOT exist in-tree** — the
  biggest v1 risk, isolated to PR9.
- Custodial first-key bootstrap.
- WebAuthn first-credential bootstrap.
- Connected-wallet trust registration.
- Key rotation.
- Custody recovery / backup.
- HSM/KMS mainnet threshold.
- Multi-sig / quorum.
- WC session TTL / re-auth.
