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
use ironclaw_host_api::FailureKind;
use ironclaw_host_runtime::{
    AttestedRaiseHook, AttestedRaiseRequest, RuntimeAttestedGate, RuntimeBlockedReason,
    RuntimeCapabilityFailure, RuntimeCapabilityOutcome, RuntimeGateId,
};
use ironclaw_signing_provider::{
    ActorId, ApprovedTxHash, ChainId, GateRef as SigningGateRef, KeyOrAccountId, ProviderId, RunId,
    ScopeId, SigningContext, TenantId as SigningTenantId, UserId as SigningUserId,
};

use ironclaw_attestation::{
    AgentKeyId, IntentId, IntentRecord, IntentSigner, IntentStore, ReviewTokenHash, UnsignedIntent,
};

use crate::attested::RebornAttestedComposition;

/// Everything the raise needs to mint, sign, and persist a [`SignedIntent`]
/// alongside the gate (§B3).
///
/// Bundled rather than threaded as four separate `Option`s so the seam is
/// all-or-nothing: a deployment either mints intents or does not, and there is
/// no half-configured shape where a signer exists but the store to put its
/// output in does not.
#[derive(Clone)]
pub struct IntentMinting {
    signer: Arc<dyn IntentSigner>,
    intents: Arc<dyn IntentStore>,
    /// How long the intent — and therefore its review link — stays valid.
    ttl_ms: i64,
    /// Base for the review URL the agent relays to the approver; the minted
    /// token is appended. Fixed server-side (no attacker-controllable
    /// redirect target).
    review_url_base: String,
}

impl IntentMinting {
    /// The intent store this minting writes to.
    ///
    /// Exposed so the composition can hand the SAME instance to the
    /// continuation port's lifecycle projection. Two separate stores would make
    /// the projection silently never find the intent it just settled.
    pub fn intents(&self) -> &Arc<dyn IntentStore> {
        &self.intents
    }

    /// The server-fixed base the review link is built on (config, never a
    /// request value).
    pub fn review_url_base(&self) -> &str {
        &self.review_url_base
    }

    /// Wire intent minting for the raise path.
    pub fn new(
        signer: Arc<dyn IntentSigner>,
        intents: Arc<dyn IntentStore>,
        ttl_ms: i64,
        review_url_base: impl Into<String>,
    ) -> Self {
        Self {
            signer,
            intents,
            ttl_ms,
            review_url_base: review_url_base.into(),
        }
    }
}

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
    // arch-exempt: optional_arc, intent minting is a Phase B capability a
    // deployment either composes or does not; when absent the gate machinery is
    // unchanged and simply carries no review link.
    minting: Option<IntentMinting>,
}

impl<B, G, L> RebornAttestedRaiseHook<B, G, L>
where
    B: Broadcaster + 'static,
    G: SealedGrantStore + 'static,
    L: SigningLedger + 'static,
{
    /// Build the raise hook over the runtime's attested-signing composition.
    pub fn new(composition: Arc<RebornAttestedComposition<B, G, L>>) -> Self {
        Self {
            composition,
            minting: None,
        }
    }

    /// Attach intent minting (§B3). With it wired, every raise also mints,
    /// signs, and persists a `SignedIntent` and returns its review link.
    pub fn with_intent_minting(mut self, minting: IntentMinting) -> Self {
        self.minting = Some(minting);
        self
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
            Err(failure) => RuntimeCapabilityOutcome::Failed(RuntimeCapabilityFailure::new(
                request.capability_id.clone(),
                failure.kind,
                Some(failure.message),
            )),
        }
    }
}

/// A sanitized raise failure (category + redacted message). Never carries key
/// material, decoded tx internals, or chain errors verbatim.
struct RaiseFailure {
    kind: FailureKind,
    message: String,
}

impl RaiseFailure {
    fn new(kind: FailureKind, message: impl Into<String>) -> Self {
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
                RaiseFailure::new(
                    FailureKind::InputEncode,
                    format!("request_signature params invalid: {error}"),
                )
            })?;

        // Bound the agent-supplied signer string before it enters the hash
        // domain. `signer_account` is raw caller-supplied JSON; a length cap
        // (chain-agnostic) closes the unbounded-allocation / hash-domain-confusion
        // path without needing per-chain format knowledge. Fail-closed: an
        // over-long signer raises no gate.
        if params.signer_account.len() > SIGNER_ACCOUNT_MAX_BYTES {
            return Err(RaiseFailure::new(
                FailureKind::InputEncode,
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
                    FailureKind::Backend,
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

        // Binding hash (PR2 domain-separated digest) via the SAFE public API:
        // `approved_tx_hash_for` derives the render AND the canonical signing
        // bytes from the same decoded transaction, and reads chain/network +
        // tx-type off it, so a raise can never fold a render of one transaction
        // together with the canonical bytes of another.
        //
        // The signer/account folded in is the one the SIGNING CONTEXT binds
        // (`key_or_account_id`), never a value derived from the transaction
        // body — exactly what the resolve-side `recompute_approved_hash` passes
        // (`req.context.key_or_account_id`), so the sign-time hash re-check
        // (threat #3) cannot diverge from what was bound at raise.
        let approved_tx_hash: ApprovedTxHash = approved_tx_hash_for(
            &params.decoded,
            context.key_or_account_id.as_str(),
            schema_version,
        )
        .map_err(|error| {
            RaiseFailure::new(
                FailureKind::InputEncode,
                format!("transaction could not be bound for approval: {error}"),
            )
        })?;
        let expected_tx_hash = approved_tx_hash_ref_hex(approved_tx_hash.as_bytes());

        // The custodial keystore lookup scope is the authorized execution
        // scope, carried directly into the binding.
        let scope = request.context.resource_scope.clone();
        let chain = ChainKeyId::new(chain_network.clone()).map_err(|error| {
            RaiseFailure::new(
                FailureKind::InputEncode,
                format!("transaction chain id is not usable for signing: {error}"),
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

        // Mint + persist the intent BEFORE the gate exists (§B3). Ordering is
        // deliberate: a minting failure here leaves nothing behind, whereas
        // minting after registration could strand a raised gate whose approver
        // never receives a review link and therefore can never resolve it.
        let review_url = match self.minting.clone() {
            Some(minting) => Some(
                self.mint_intent(&minting, &request.context, &binding, now_unix_millis())
                    .await?,
            ),
            None => None,
        };

        // Persist the authoritative binding + seal the one-shot grant. If this
        // fails, surface Failed (fail-closed) — no gate is raised.
        self.composition
            .register_attested_gate(signing_gate_ref, binding, now_unix_millis(), None)
            .await
            .map_err(|error| {
                RaiseFailure::new(
                    FailureKind::Backend,
                    format!("attested gate registration failed: {error}"),
                )
            })?;

        Ok(RuntimeCapabilityOutcome::AttestedSigningRequired(
            RuntimeAttestedGate {
                gate_id,
                capability_id: request.capability_id.clone(),
                expected_tx_hash,
                reason: RuntimeBlockedReason::AttestedSigningRequired,
                review_url,
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

impl<B, G, L> RebornAttestedRaiseHook<B, G, L>
where
    B: Broadcaster + 'static,
    G: SealedGrantStore + 'static,
    L: SigningLedger + 'static,
{
    /// Mint, sign, and persist the intent for a raise; returns its review URL.
    ///
    /// The intent embeds the SAME `approved_tx_hash` the binding carries, so it
    /// attests to exactly the transaction the device will clear-sign and the
    /// resume path will verify — never a re-derived one.
    async fn mint_intent(
        &self,
        minting: &IntentMinting,
        context: &ExecutionContext,
        binding: &AttestedGateBinding,
        now_ms: i64,
    ) -> Result<String, RaiseFailure> {
        // Attribution is per (tenant, agent); an invocation with no agent id
        // attributes to the user, matching how the signing context labels its
        // scope and actor.
        let agent = context
            .agent_id
            .as_ref()
            .map(|id| id.as_str().to_string())
            .unwrap_or_else(|| context.user_id.as_str().to_string());
        let tenant = binding.context.tenant.clone();

        let key_id: AgentKeyId = minting
            .signer
            .active_key_id(&tenant, &agent, now_ms)
            .await
            .map_err(|error| {
                RaiseFailure::new(
                    FailureKind::Backend,
                    format!("no usable agent signing key: {error}"),
                )
            })?;

        let intent = UnsignedIntent {
            intent_id: IntentId::new(),
            tenant: tenant.clone(),
            agent_key_id: key_id.clone(),
            // The approver is the authenticated human this raise is for. Phase
            // C authorizes viewing and signing against exactly this identity.
            approver: binding.context.user.clone(),
            chain_id: binding.context.chain_id.clone(),
            approved_tx_hash: binding.approved_tx_hash,
            decoded_tx: binding.decoded.clone(),
            created_at_ms: now_ms,
            expires_at_ms: now_ms.saturating_add(minting.ttl_ms),
            schema_version: binding.schema_version,
        };
        intent.validate().map_err(|error| {
            RaiseFailure::new(
                FailureKind::InputEncode,
                format!("intent could not be constructed: {error}"),
            )
        })?;

        let signature = minting
            .signer
            .sign_intent(&key_id, &intent.signing_preimage())
            .map_err(|error| {
                RaiseFailure::new(
                    FailureKind::Backend,
                    format!("intent could not be signed: {error}"),
                )
            })?;

        // 256 bits of OS entropy, base64url, and only its hash is stored.
        let token = mint_review_token()?;
        let record = IntentRecord::pending(
            intent.into_signed(signature),
            binding.context.gate_ref.clone(),
            ReviewTokenHash::of_token(&token),
        );
        minting.intents.put(record).await.map_err(|error| {
            RaiseFailure::new(
                FailureKind::Backend,
                format!("intent could not be persisted: {error}"),
            )
        })?;

        Ok(format!(
            "{}/{token}",
            minting.review_url_base.trim_end_matches('/')
        ))
    }
}

/// Mint a review token: 256 bits of OS entropy, base64url (no padding).
///
/// Unguessable is the whole security property of the token — it is an
/// addressing convenience, never an authorization — so entropy failure is
/// fail-closed rather than degraded to a weaker source.
fn mint_review_token() -> Result<String, RaiseFailure> {
    use base64::Engine as _;
    use rand::RngExt as _;
    // `rand::rng()` is the OS-seeded thread RNG, the same source this crate's
    // channel-pairing code mint uses.
    let mut rng = rand::rng();
    let bytes: [u8; 32] = std::array::from_fn(|_| rng.random());
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
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
    SigningContext {
        tenant: SigningTenantId::new(context.tenant_id.as_str()),
        user: SigningUserId::new(context.user_id.as_str()),
        // The signing-context scope id is a coarse authorization-scope label;
        // the authoritative keystore-owner scope is carried separately as the
        // binding's `ResourceScope`. Derive a stable label from the project (or
        // the user when no project is scoped).
        scope: ScopeId::new(
            context
                .resource_scope
                .project_id
                .as_ref()
                .map(|id| id.as_str().to_string())
                .unwrap_or_else(|| context.user_id.as_str().to_string()),
        ),
        actor: ActorId::new(
            context
                .agent_id
                .as_ref()
                .map(|id| id.as_str().to_string())
                .unwrap_or_else(|| context.user_id.as_str().to_string()),
        ),
        run_id: RunId::new(context.invocation_id.to_string()),
        gate_ref,
        chain_id: ChainId::new(chain_network),
        key_or_account_id: KeyOrAccountId::new(signer_account),
    }
}
