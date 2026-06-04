//! Composition-owned attested-signing raise hook (attested-signing PR14).
//!
//! This is the production implementation of the crypto-free
//! [`ironclaw_host_runtime::AttestedRaiseHook`]. When the agent invokes the
//! `request_signature` first-party capability, [`DefaultHostRuntime`] routes the
//! invocation here instead of normal dispatch.
//!
//! The hook is the ONLY place the raise path's crypto/chain logic lives — it
//! keeps `ironclaw_turns` / `ironclaw_agent_loop` / `ironclaw_host_runtime`
//! crypto-free. On a `request_signature` invocation it:
//!
//! 1. parses the opaque request params into a [`DecodedTransaction`] +
//!    custodial signer/account + provider hint;
//! 2. selects the signing provider (custodial-only on this branch);
//! 3. renders the tx and computes the binding [`ApprovedTxHash`];
//! 4. builds the authoritative [`AttestedGateBinding`] and calls
//!    [`RebornAttestedComposition::register_attested_gate`], which persists the
//!    binding the resume path reads back AND seals the one-shot grant;
//! 5. returns [`RuntimeCapabilityOutcome::AttestedSigningRequired`] carrying the
//!    opaque `gate_ref` + `expected_tx_hash`.
//!
//! **Fail-closed:** any failure (parse, provider selection, binding-persist,
//! grant-seal) returns [`RuntimeCapabilityOutcome::Failed`] — never a
//! half-raised gate.
//!
//! ## Custodial-only / NEAR-WC fail-closed boundary (this branch)
//!
//! The NEAR-redirect and WalletConnect resolve-side verifiers need the
//! `expected_access_key` / `expected_signing_payload` binding fields, which do
//! NOT exist on this branch's [`AttestedGateBinding`]. So only the **custodial**
//! provider path may raise end-to-end here; a NEAR/WC `provider_hint` fails
//! closed (returns `Failed`, raises no gate). Resolve-side verification is never
//! weakened to make a raise "work".

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;

use ironclaw_attestation::{
    DecodedTransaction, RenderingSchemaVersion, SealedGrantStore, SigningLedger,
    approved_tx_hash_for,
};
use ironclaw_attested_runtime::{AttestedGateBinding, Broadcaster, approved_tx_hash_ref_hex};
use ironclaw_chain_signing::ChainKeyId;
use ironclaw_host_api::ExecutionContext;
use ironclaw_host_runtime::{
    AttestedRaiseHook, AttestedRaiseRequest, RuntimeAttestedGate, RuntimeBlockedReason,
    RuntimeCapabilityFailure, RuntimeCapabilityOutcome, RuntimeFailureKind, RuntimeGateId,
};
use ironclaw_signing_provider::{
    ActorId, ApprovedTxHash, ChainId, GateRef as SigningGateRef, KeyOrAccountId, ProviderId, RunId,
    ScopeId, SigningContext, TenantId as SigningTenantId, UserId as SigningUserId,
};

use crate::attested::RebornAttestedComposition;

/// Maximum byte length of the agent-supplied `signer_account` before it enters
/// the hash domain. Chain-agnostic upper bound (no per-chain format knowledge):
/// it only closes the unbounded-allocation / hash-domain-confusion path that an
/// adversarial agent could open with an arbitrarily long string. The longest
/// real account identity (e.g. a NEAR named account or a hex address) is well
/// under this; honest callers are never affected. Per-chain *format* validation
/// belongs at the shared decoder/provider boundary, not here.
const SIGNER_ACCOUNT_MAX_BYTES: usize = 128;

/// Wire form of the `request_signature` params the agent supplies.
///
/// The transaction arrives already decoded (the host/decoder produced the
/// SDK-free [`DecodedTransaction`] projection upstream); this hook never decodes
/// raw chain bytes. `provider_hint` selects the trust model — only `custodial`
/// raises end-to-end on this branch.
#[derive(Debug, Deserialize)]
struct RequestSignatureParams {
    /// Provider trust-model hint. `custodial` is the only value that raises
    /// end-to-end on this branch.
    provider_hint: ProviderHint,
    /// The signer / account the custodial keystore signs for (lowercase hex, no
    /// `0x` prefix for EVM). Bound into the signing context + hash.
    signer_account: String,
    /// The server-decoded transaction (chain-tagged, SDK-free projection).
    decoded: DecodedTransaction,
}

/// Provider-hint wire form. Mirrors [`ProviderId`]'s snake_case tags so the
/// agent names the same provider identities the rest of the substrate uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProviderHint {
    Custodial,
    Injected,
    NearRedirect,
    WalletConnect,
}

/// Composition-owned [`AttestedRaiseHook`].
///
/// Holds the runtime's attested-signing composition (the same binding store +
/// grant store the resume path / driver read). Generic over the durable-vs
/// in-memory grant store `G`, ledger `L`, and broadcaster `B`.
pub struct RebornAttestedRaiseHook<B, G, L>
where
    B: Broadcaster + 'static,
    G: SealedGrantStore + 'static,
    L: SigningLedger + 'static,
{
    composition: Arc<RebornAttestedComposition<B, G, L>>,
}

impl<B, G, L> RebornAttestedRaiseHook<B, G, L>
where
    B: Broadcaster + 'static,
    G: SealedGrantStore + 'static,
    L: SigningLedger + 'static,
{
    /// Build the raise hook over the runtime's attested-signing composition.
    pub fn new(composition: Arc<RebornAttestedComposition<B, G, L>>) -> Self {
        Self { composition }
    }
}

#[async_trait]
impl<B, G, L> AttestedRaiseHook for RebornAttestedRaiseHook<B, G, L>
where
    B: Broadcaster + 'static,
    G: SealedGrantStore + 'static,
    L: SigningLedger + 'static,
{
    async fn raise(&self, request: AttestedRaiseRequest) -> RuntimeCapabilityOutcome {
        match self.try_raise(&request).await {
            Ok(outcome) => outcome,
            // Fail-closed: any error in decode / provider selection / hash /
            // binding-persist / grant-seal yields Failed, never a raised gate.
            Err(failure) => RuntimeCapabilityOutcome::Failed(RuntimeCapabilityFailure {
                capability_id: request.capability_id.clone(),
                kind: failure.kind,
                message: Some(failure.message),
            }),
        }
    }
}

/// A sanitized raise failure (category + redacted message). Never carries key
/// material, decoded tx internals, or chain errors verbatim.
struct RaiseFailure {
    kind: RuntimeFailureKind,
    message: String,
}

impl RaiseFailure {
    fn new(kind: RuntimeFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl<B, G, L> RebornAttestedRaiseHook<B, G, L>
where
    B: Broadcaster + 'static,
    G: SealedGrantStore + 'static,
    L: SigningLedger + 'static,
{
    async fn try_raise(
        &self,
        request: &AttestedRaiseRequest,
    ) -> Result<RuntimeCapabilityOutcome, RaiseFailure> {
        let params: RequestSignatureParams = serde_json::from_value(request.input.clone())
            .map_err(|error| {
                // Redacted: the verbatim serde error can leak the params schema
                // (field names, expected types, nesting) to the agent/LLM
                // context, letting an adversarial caller probe the wire shape.
                // Log the detail server-side; surface only a static category.
                tracing::debug!(%error, "request_signature params failed to deserialize");
                RaiseFailure::new(
                    RuntimeFailureKind::InvalidInput,
                    "request_signature params invalid",
                )
            })?;

        // Bound the agent-supplied signer string before it enters the hash
        // domain. `signer_account` is raw caller-supplied JSON; a length cap
        // (chain-agnostic) closes the unbounded-allocation / hash-domain-confusion
        // path without needing per-chain format knowledge. Fail-closed: an
        // over-long signer raises no gate.
        if params.signer_account.len() > SIGNER_ACCOUNT_MAX_BYTES {
            return Err(RaiseFailure::new(
                RuntimeFailureKind::InvalidInput,
                "request_signature signer_account exceeds maximum length",
            ));
        }

        // Provider selection. Custodial-only on this branch: NEAR/WC fail closed
        // because their resolve-side verifiers need binding fields
        // (expected_access_key / expected_signing_payload) that do not exist on
        // this branch's AttestedGateBinding.
        //
        // rebase: raise NEAR/WC once expected_access_key/expected_signing_payload
        // binding fields land (PR8/PR9).
        match params.provider_hint {
            ProviderHint::Custodial => {}
            ProviderHint::Injected | ProviderHint::NearRedirect | ProviderHint::WalletConnect => {
                return Err(RaiseFailure::new(
                    RuntimeFailureKind::Backend,
                    "only custodial signing can raise an attested gate on this branch; \
                     external-wallet providers fail closed pending binding fields",
                ));
            }
        }

        // The chain/network domain separator the hash binds to is derived from
        // the decoded tx itself, so the binding can never disagree with what was
        // hashed.
        let chain_network = params.decoded.chain_network();
        let schema_version = RenderingSchemaVersion::CURRENT;

        // Generate the gate id and the loop-facing gate ref. The binding store
        // key and the SigningContext.gate_ref MUST equal the loop's gate ref
        // (`gate:attested-<id>`) so the resume path reads the binding back.
        let gate_id = RuntimeGateId::default();
        let gate_ref_str = format!("gate:attested-{gate_id}");
        let signing_gate_ref = SigningGateRef::new(gate_ref_str.clone());

        let context = signing_context_from_execution(
            &request.context,
            signing_gate_ref.clone(),
            &chain_network,
            &params.signer_account,
        );

        // Binding hash (PR2 domain-separated digest) via the safe public API:
        // `approved_tx_hash_for` derives the render and canonical signing bytes
        // from the SAME decoded tx (so they can never describe different
        // transactions), and reads chain/network + tx-type off the tx. The
        // signer bound into the hash is the GATE-BOUND signer from the
        // authoritative `SigningContext.key_or_account_id` — NOT derived from the
        // transaction body — so the sign-time hash re-check (threat #3) binds the
        // exact account the keystore is asked to sign for. Fail-closed: any
        // render/canonical error raises no gate.
        let approved_tx_hash: ApprovedTxHash = approved_tx_hash_for(
            &params.decoded,
            context.key_or_account_id.as_str(),
            schema_version,
        )
        .map_err(|error| {
            // Redacted: chain/render errors can leak decoded-tx internals.
            tracing::debug!(%error, "approved_tx_hash computation failed");
            RaiseFailure::new(
                RuntimeFailureKind::InvalidInput,
                "request_signature transaction could not be rendered",
            )
        })?;
        let expected_tx_hash = approved_tx_hash_ref_hex(approved_tx_hash.as_bytes());

        // The custodial keystore lookup scope is the authorized execution
        // scope, carried directly into the binding.
        let scope = request.context.resource_scope.clone();
        let chain = ChainKeyId::new(chain_network.clone()).map_err(|error| {
            tracing::debug!(%error, "chain key id rejected");
            RaiseFailure::new(
                RuntimeFailureKind::InvalidInput,
                "request_signature chain identity invalid",
            )
        })?;

        let binding = AttestedGateBinding {
            provider_id: ProviderId::Custodial,
            context,
            approved_tx_hash,
            decoded: params.decoded,
            chain,
            scope,
            schema_version,
        };

        // Persist the authoritative binding + seal the one-shot grant. If this
        // fails, surface Failed (fail-closed) — no gate is raised.
        //
        // `created_at_ms` is the real wall-clock raise time so audit ordering,
        // replay-window detection, and any future time-based expiry are correct
        // (a zero/epoch value would make every grant look created at 1970).
        let created_at_ms = now_unix_millis();
        self.composition
            .register_attested_gate(signing_gate_ref, binding, created_at_ms, None)
            .await
            .map_err(|error| {
                // Redacted: the RaiseFailure contract promises messages never
                // carry storage internals / key material verbatim. The detailed
                // error is logged server-side, not surfaced to the agent.
                tracing::debug!(%error, "attested gate registration failed");
                RaiseFailure::new(
                    RuntimeFailureKind::Backend,
                    "attested gate registration failed",
                )
            })?;

        Ok(RuntimeCapabilityOutcome::AttestedSigningRequired(
            RuntimeAttestedGate {
                gate_id,
                capability_id: request.capability_id.clone(),
                expected_tx_hash,
                reason: RuntimeBlockedReason::AttestedSigningRequired,
            },
        ))
    }
}

/// Current Unix time in milliseconds.
///
/// Two saturating fallbacks, neither reachable in practice on a sane clock:
/// returns `0` if the system clock is set before the Unix epoch, and clamps to
/// `i64::MAX` if the millisecond count somehow exceeds `i64` (year ~292 million).
/// `i64::MAX` is a deliberately monotonic-high sentinel: it can never look like a
/// stale/expired timestamp to a future replay-window or expiry check (those
/// treat *older* timestamps as suspect), so the clamp fails safe rather than
/// silently aging a grant.
fn now_unix_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Derive a coarse scope label from an optional identity, defaulting to the
/// user id when the identity is absent. Used for the signing-context scope and
/// actor labels (the authoritative owner scope rides the binding separately).
fn scope_label(maybe_id: Option<&str>, fallback: &str) -> String {
    maybe_id.unwrap_or(fallback).to_string()
}

/// Build the authoritative [`SigningContext`] from the already-authorized
/// execution context. Identities are taken from the host context, never from
/// caller-supplied params (the params only carry the decoded tx + signer).
fn signing_context_from_execution(
    context: &ExecutionContext,
    gate_ref: SigningGateRef,
    chain_network: &str,
    signer_account: &str,
) -> SigningContext {
    let user_id = context.user_id.as_str();
    SigningContext {
        tenant: SigningTenantId::new(context.tenant_id.as_str()),
        user: SigningUserId::new(user_id),
        // The signing-context scope id is a coarse authorization-scope label;
        // the authoritative keystore-owner scope is carried separately as the
        // binding's `ResourceScope`. Derive a stable label from the project (or
        // the user when no project is scoped).
        scope: ScopeId::new(scope_label(
            context
                .resource_scope
                .project_id
                .as_ref()
                .map(|id| id.as_str()),
            user_id,
        )),
        actor: ActorId::new(scope_label(
            context.agent_id.as_ref().map(|id| id.as_str()),
            user_id,
        )),
        run_id: RunId::new(context.invocation_id.to_string()),
        gate_ref,
        chain_id: ChainId::new(chain_network),
        key_or_account_id: KeyOrAccountId::new(signer_account),
    }
}
