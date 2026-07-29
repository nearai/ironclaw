# Family: `crates/contracts/` — neutral vocabulary and ports

**Layer(s):** `contracts` (the sole layer; every other layer sits above it) · **Crates (target):** 6 — `ironclaw_host_api`, `ironclaw_common`, `ironclaw_prompt_envelope`, `ironclaw_loop_contracts` (NEW), `ironclaw_extension_contracts` (NEW), `ironclaw_product_contracts` (NEW) · **Security posture:** executes nothing and persists nothing, yet is security-relevant because it is the only family permitted to declare *sealed* constructors (`Authorized`, `TrustClass` privileged variants, verified-inbound/bearer evidence) — the guarantee is negative: a bug here can misdescribe authority but cannot itself grant it.

*Authority: PROPOSAL.md §6.1 (family role + all five crate entries), §5 (tree), §8 (dependency model), §9 rows 1–6, §12.1/§12.10 (risks). CURRENT-state citations are to this session's read of `dde662d5a`.*

## Identity — what this family IS

Contracts is the leaf tier every other family may depend on. The four-part admission test a type must pass to live here (from `docs/reborn/contracts/host-api.md` §1, applied family-wide per PROPOSAL §6.1) requires that it:

- (a) names a concept crossing an authority/host/product boundary,
- (b) is neutral across vendor/runtime/storage/deployment,
- (c) is needed by lower layers without importing an owner, and
- (d) carries no execution, persistence, policy engine, or workflow.

CURRENTLY this is effectively one crate, `ironclaw_host_api` — 25,728 lines across 63 files/46 modules, zero internal dependencies, and 56 dependents (both fan-in and fan-out measured this session), making it the single most depended-upon crate in the workspace. Roughly 9.8k of its ~16.9k non-core lines (measured this session) are product/channel/manifest vocabulary that arrived during the product-adapter/product-surface era but was never given its own contract-tier home; that pooling — not a missing layer — is precisely what the family's three NEW crates resolve (PROPOSAL §2.2/§2.5). TARGET: the same vocabulary, correctly homed across six thin, allowlisted crates, each answerable to the same four-part test.

## What makes it distinct

- **vs `substrate/`:** they own privileged *mechanism* — real backends and drivers (disk/libSQL/Postgres, AES-GCM+OS keychain, reqwest+DNS resolution); we own the *shape* those mechanisms accept and return, with zero I/O. Proof in Cargo today: `host_api` has 0 internal deps and is barred from axum/reqwest/wasmtime/DB clients (`reborn_dependency_boundaries.rs:230-237`), while every substrate crate ships a real driver cone (`ironclaw_filesystem` alone pulls `libsql`+`deadpool-postgres`+`tokio-postgres`).
- **vs `domains/`:** they own durable record grammar and service *implementations* over `ScopedFilesystem`; we own only the vocabulary those services accept and return. The line: a type with a persistence story belongs in domains; a type that only names a boundary crossing belongs here — domains depend on contracts, never the reverse.
- **vs `kernel/`:** they make the authority *decision* and mint privileged instances; we declare the sealed *types* and the *port* a decision flows through, constructing none of it ourselves. `CapabilityDispatcher` is declared in `host_api::dispatch` (683 ln); its sole production implementation, `RuntimeDispatcher`, lives in `kernel/ironclaw_capabilities` — the port lives low, the authority to satisfy it lives high.
- **vs `loop/`:** they hold the port *implementations* (`loop_host`'s `HostRuntimeLoopCapabilityPort` and siblings, `runner`'s driver adapters); we hold the port *definitions*. `agent_loop`'s special matrix rule — contracts-layer normal deps only (`reborn_dependency_boundaries.rs:129`) — is the mechanically strictest proof of this line anywhere in the workspace.
- **vs `extensions/`:** they own the registry/records (`ironclaw_extensions`) and the generic hosting machinery (`ironclaw_extension_host`); we own only what an extension *is and exposes* — `ChannelAdapter`, manifest-surface descriptors, verified-inbound evidence. A lane depending on the registry crate instead of on us is exactly the standing W7 exception this split retires (PROPOSAL §8.3).
- **vs `product/`:** they own the `ProductSurface` *implementation* (`RebornServices`, 7,131 lines measured this session) and all admission/delivery/binding behavior; we own the membrane's shape and the ports whose real implementations sit beside product. Today those ports (`ChannelDeliveryResolver`, `LlmConfigService`, …) are defined *inside* `ironclaw_product` itself — the reason `extension_host`/`operator`/`telegram_extension` currently sit above it in the dependency graph (PROPOSAL §2.3, §6.1.3).

## What belongs here / What must never be here

**Belongs here:**
- Identity/scope/path/mount vocabulary; capability/action/decision/approval/resource/audit shapes.
- Sealed authority witnesses and the ports they gate (`Authorized`, `CapabilityDispatcher`) — declared, never privilegedly constructed outside kernel/host code.
- Cross-boundary adapter/surface trait *definitions* (`ChannelAdapter`, `ToolAdapter`, `ProductSurface`, the `Loop*Port` set) — never their implementations.
- Wire DTO homes shared by transports that must not see each other's owners (`AppEvent`, product command/view descriptor types, manifest-surface descriptors).
- Domain-free cross-cutting primitives with persisted-compatibility guarantees (identity newtypes, pkce, hashing, timezone).

**Must never be here:**
- Any port/trait *implementation*, storage, or an HTTP/DB/wasmtime client.
- Rendering, parsing, or classification behavior — the four audited violations (`render_channel_auth_prompt`, `parse_product_slash_command`, `classify_channel_inbound_text`, `parse_interaction_resolution_text`) all move to product because they are workflow, not vocabulary.
- Logging or channel side effects (the `tracing::error!` at `product_surface.rs:309`; the `tokio::sync::mpsc` in `product_adapter/projection.rs:7`).
- Vendor names (scanner-enforced, PROPOSAL §8.1 rule 4) or framework types — Axum stays in `product/ironclaw_host_ingress` specifically so contracts stays Axum-free.
- Persistence *ports* — even a trait — for the same reason a domain's store interface belongs in the domain, not the vocabulary crate (`host_api::user_identity`'s store ports move to `domains/ironclaw_identity`).

## Dependency direction

- **May depend on (internal):** nothing, for `host_api`/`common`/`prompt_envelope` (each a leaf). The three NEW crates depend only within the family:
  - `loop_contracts` → `host_api`, `common`, `prompt_envelope`.
  - `extension_contracts` → `host_api`, `common`.
  - `product_contracts` → `host_api`, `common`, `extension_contracts` (for channel-facing DTO reuse).
- **Forbidden (external):** axum, reqwest, wasmtime, any DB client — enforced today for `host_api` and extended family-wide by the new contracts-purity rule (§11.2.3).
- **Who may depend on it:** every other family, directly or transitively — substrate, events, domains, kernel, lanes, loop, extensions, product, and app all resolve to `contracts` in the dependency diagram (PROPOSAL §8.1). This is the only family with that property, which is the practical definition of "leaf tier."
- **Notable inversions (port defined low, implemented high — PROPOSAL §8.1 rule 3):**
  - `CapabilityDispatcher` (host_api → `kernel/ironclaw_capabilities`'s `RuntimeDispatcher`).
  - `ChannelAdapter`/`ToolAdapter` (extension_contracts → `extensions/packages/*`).
  - `ProductSurface` + product-side ports (product_contracts → product/operator/extension_host/extension_manager/composition).
  - The `Loop*Port` set (loop_contracts → loop_host/runner/hooks).
  - Each is justified by the same test: "a neutral port is justified exactly when the lower layer must invoke behavior whose implementation cannot live below the caller" — single-impl ports that fail that test are deleted, not relocated (PROPOSAL §2.6/§8.1).

## Security & authority role

Contracts holds the sealed constructors that make forgery a compile-time impossibility rather than a review discipline: `Authorized` (kernel-only mint), `TrustClass` privileged variants (`serde(skip_deserializing)`), and — after §6.1.2/§11.2.5 land — verified-inbound and bearer/session evidence. None of these types can be *constructed* outside the crate-visible seam their owner exposes, so a contracts crate can misdescribe authority (a bad DTO) but never grant it. This is why the family's own admission test forbids execution and persistence: adding either would turn a vocabulary crate into a second place authority could originate.

## Crate specifications

### `ironclaw_host_api`

- **Path & disposition:** `crates/contracts/ironclaw_host_api` — retain, narrow (PROPOSAL §9 row 1; §6.1.1).
- **Purpose:** the dependency-free authority vocabulary — identities/scopes/paths/mounts, capability/decision/approval/resource/audit shapes, the sealed dispatch port, sanitized resolution/failure vocabulary, HTTP-egress and ingress-descriptor vocabulary, runtime/trust/policy vocabulary, and turn vocabulary.
- **Target contents:** grounded in the CURRENT tree (63 files, 46 `pub mod` entries, `lib.rs:34-83`). Stays:
  - `ids`, `scope`, `path`, `mount`, `error` — core identity/authority primitives.
  - `capability`/`capability_profile`/`action`/`decision`/`approval` — requested-effect and host-decision vocabulary.
  - `authorized` (`Authorized`, 351 ln, sealed) + `dispatch` (`CapabilityDispatcher`, 683 ln).
  - `invocation`/`lane` (closed `RuntimeLane`, 331/194 ln).
  - the resolution/failure cluster: `resolution`, `result_meta`, `gate_record`, `failure` (166 ln), `safe_summary`, `model_result_preview`, `host_remediation`, `credential_redaction` (362 ln).
  - `resource`, `audit`, `host_port`.
  - `http` (`RuntimeHttpEgress`, 576 ln); `ingress` (`IngressRouteDescriptor`/`IngressPolicy`/`ListenerClass`, 1,039 ln).
  - `runtime` (262 ln), `runtime_policy` (909 ln), `trust` (160 ln).
  - `turn` (716 ln) — becomes the *complete* canonical turn vocabulary. `turn.rs:248` already defines `ReplyTargetBindingRef` via the `bounded_ref!` macro — consumers reach it only through `ironclaw_turns`'s re-export (`turns/lib.rs:64`) today, which is the exact indirection the A4 audit flagged as a false "product/turns dependency."
- **Migration delta:** sheds ~9.8k of its current ~16.9k non-core lines to the two new sibling crates.
  - To `extension_contracts`: `product_adapter/{channel_adapter.rs 309, tool_adapter.rs 208, egress.rs 601, auth.rs 584}`, `channel.rs` (732), `channel_identity.rs` (72), `recipe.rs` (935), `memory.rs` (144), `state.rs` (187), `extension.rs` (165, `Extension` trait :116).
  - To `product_contracts`: `product_surface.rs` (608, `ProductSurface` trait :352), `package_lifecycle.rs` (650), `operator_llm.rs` (140), `product_adapter/{inbound.rs 1537, projection.rs 207, external.rs 470, interaction_commands.rs 272}`.
  - Out entirely: `user_identity.rs` (160, persistence *ports*) moves to `domains/ironclaw_identity`.
  - Gains: failure-summary data tables from `runner::failure_summary` (product's sole current import from `runner`, `projection/turn_events.rs:34`).
  - **Prerequisite:** replace the 45-module wildcard prelude (`lib.rs:85-132`, confirmed live — `pub use module::*` for every module but `credential_redaction`) with module-qualified exports; PROPOSAL calls this "the single cheapest enabling change in the whole proposal," and every carve-out above depends on it landing first.
- **Owns:** the vocabulary listed above, in full.
- **Must never contain:** vendor names; adapter-trait implementations; ProductSurface/product DTOs; loop-port traits; the rendering/parsing/classification helpers named in the family "must never" list; `tracing`/`tokio` side effects; feature-gated channel-verification evidence minting (moves with its callers to `extension_contracts`); persistence ports.
- **Allowed internal deps:** none (mechanically enforced, `reborn_dependency_boundaries.rs:230-237`).
- **Forbidden:** every internal crate; axum/reqwest/wasmtime/DB clients externally.
- **Public contracts & ports:** the vocabulary above; sealed constructors for `Authorized` and bearer/session `ProtocolAuthEvidence`. The six `mark_*_verified` mint functions (`product_adapter/auth.rs:366-470`, all gated `#[cfg(feature = "host-auth-mint")]`) are today reached through two import paths — direct, and via `ironclaw_product`'s re-export at `product/src/lib.rs:186-207` — the "two mint families" PROPOSAL Invariant C names are this single definition site seen through two doors; §6.1.2/§11.2.5 seal it to one.
- **Security & authority role:** domain-ownership boundary for authority vocabulary; security-relevant because it holds the sealed `Authorized`/`TrustClass` constructors (`serde(skip_deserializing)` on privileged variants stays verbatim).
- **Why a crate (not a module):** criterion 1+2 — one neutral contract with 56 verified dependents and a mechanically enforced zero-dep barrier; no consumer count below "six-plus, none of which can import an owner" would justify a further split.
- **Enforcement:**
  - `reborn_workspace_crates_declare_layers_and_follow_layer_matrix` (`reborn_dependency_boundaries.rs:49`).
  - The host_api zero-internal-deps allowlist (`:230-237`).
  - `host_product_surface_method_set_is_frozen` (moves to guard `product_contracts`'s `ProductSurface` once relocated).
  - NEW: contracts-purity allowlist + external-framework deny (§11.2.3), port-location scan forbidding cross-crate `pub use` of relocated traits (§11.2.4), sealed-evidence rule pinning the `host-auth-mint` feature's removal (§11.2.5).

### `ironclaw_common`

- **Path & disposition:** `crates/contracts/ironclaw_common` — retain, narrow (PROPOSAL §9 row 2; §6.1.5).
- **Purpose:** domain-free cross-cutting primitives with persisted-compatibility guarantees — the newtype template contract from `.claude/rules/types.md` is anchored here.
- **Target contents:** grounded in the CURRENT tree (5,175 lines, 17 files, measured this session). Stays:
  - `identity.rs` (933 ln — `CredentialName`/`ExtensionName`/`McpServerName`/`ExternalThreadId`, documented `#[serde(transparent)]` + `from_trusted` compatibility exception).
  - `pkce.rs` (78), `hashing.rs` (30), `paths.rs` (57), `timezone.rs` (166), `util.rs` (126), `env_helpers.rs` (215).
  - `attachment.rs` (171) + `attachment_format.rs` (949) — reconciled once the `AttachmentRef` name collision with `host_api::product_adapter::channel_adapter.rs:134` resolves (the channel-facing concept is renamed, e.g. `VendorAttachmentRef`, when it lands in `extension_contracts`).
- **Migration delta:**
  - `event.rs` (1,234 ln — the 42/43-variant `AppEvent` wire enum at line 201) → `product_contracts`.
  - `llm_costs.rs` (383), `provider_transcript.rs` (155), `model_selection.rs` (51) → `domains/ironclaw_llm`.
  - `platform.rs` (59) → its consumer.
  - `automation.rs` (104) → its owner (evicted; destination not further specified by PROPOSAL).
  - `trust_boundary.rs` (395 ln, `#[allow(dead_code)]`, verified 0 consumers this session) → deleted outright, not moved.
- **Owns:** the primitives above.
- **Must never contain:** wire protocols, LLM domain data, prompt-construction data, budget-policy constants (→ `kernel/ironclaw_resources`), dead scaffolding, automation vocabulary.
- **Allowed internal deps:** none. **Forbidden:** all internal.
- **Public contracts & ports:** the primitives above; no traits of note — this crate is data, not behavior.
- **Security & authority role:** domain-ownership (cross-domain primitives) plus persisted-wire-compatibility authority for the two legacy identity newtypes — the one place allowed to carry a documented backward-compatibility exception rather than a clean invariant.
- **Why a crate (not a module):** criterion 1 — 17-file surface genuinely domain-free; the persisted-compat exception needs exactly one home so it cannot silently re-appear elsewhere.
- **Enforcement:** layer-matrix test (host_api-identical zero-dep posture); NEW contracts-purity allowlist (§11.2.3) once `event.rs`/`llm_costs.rs`/`trust_boundary.rs` are evicted; `.claude/rules/types.md`'s newtype template remains the review-side companion to the mechanical checks.

### `ironclaw_prompt_envelope`

- **Path & disposition:** `crates/contracts/ironclaw_prompt_envelope` — retain as-is (PROPOSAL §9 row 3; §6.1.6).
- **Purpose:** the one primitive that wraps untrusted model-visible snippets with closed-vocabulary trust markers, hijack rejection, and byte caps before they reach a model.
- **Target contents:** unchanged single-file crate (432 lines, `lib.rs` only, confirmed live).
  - `wrap_untrusted{,_with_limit}`.
  - `EnvelopeSource{Memory,Hook,Skill}` (exactly 3 variants).
  - `EnvelopeTrust{Trusted,Untrusted}`, `EnvelopedContent`.
  - The instruction-hijack marker denylist; `DEFAULT_MAX_ENVELOPE_BYTES = 4096`.
  - Its own doc comment states the design intent verbatim: "the crate is a leaf: no other ironclaw crate is in its dependency tree" (`lib.rs:21`).
- **Migration delta:** none structural. Two fixes ship with the move:
  - The manifest bug — the crate description currently sits inside `[package.metadata.ironclaw]` instead of `[package]` (confirmed in `Cargo.toml`) — is corrected.
  - A guidance file is added; none exists today (no `.md` file in the crate directory, confirmed).
- **Owns:** the wrapping primitive and its closed vocabulary.
- **Must never contain:** model routing, policy, free-form labels, or additional `EnvelopeSource` variants without contract review (adding one is a deliberate API change by design, per its own doc comment).
- **Allowed internal deps:** none. **Forbidden:** all internal.
- **Public contracts & ports:** `wrap_untrusted`/`wrap_untrusted_with_limit`; no traits — pure functions over closed enums.
- **Security & authority role:** security-relevant leaf (the prompt-injection fence); its leaf-ness *is* the guarantee.
- **Why a crate (not a module):** criterion 2 — its 3 consumers (`hooks`, `host_runtime`, `memory_native`, verified this session via `Cargo.toml` grep) are exactly its 3 `EnvelopeSource` variants; folding it into `safety` would hand all three a 7,226-line regex/Aho-Corasick dependency cone for one function.
- **Enforcement:** leaf zero-dep posture (mechanically identical check to host_api/common); NEW contracts-purity allowlist (§11.2.3).
- **Open questions (§12.10):** the `prompt_envelope`⇄`safety` wrapping-pipeline unification direction is explicitly unresolved — `safety::wrap_external_content` (`ironclaw_safety/src/lib.rs:287`) is a second, independently-implemented wrapping pipeline with its own denylist; PROPOSAL records only the direction ("safety delegates to this crate's denylist") as a recorded-not-forced option, not a decision.

### `ironclaw_loop_contracts` — NEW

- **Path & disposition:** `crates/contracts/ironclaw_loop_contracts` — new, carved from `ironclaw_turns::run_profile/**` + `loop_exit` (DTO half) + `checkpoint_state` (port half) (PROPOSAL §9 row 4; §6.1.4).
- **Purpose:** the loop-tier contract — how any loop, hook, or host adapter talks to the turn kernel without importing it.
- **Target contents:** fed entirely from `ironclaw_turns` (CURRENT tree, measured this session):
  - The `run_profile/` subtree (14,346 lines) — `driver.rs`, `resolver.rs`, `compaction.rs`, `policy.rs`, `prompt.rs`, `prompt_text.rs`, `milestones.rs`, `instruction_bundle.rs`, `memory_context.rs`, `skill_context.rs`, `snapshot.rs`, `refs.rs`, `content_digest.rs`, `context_budget.rs`, `model.rs`, `model_observation.rs`, `model_work.rs`, `runtime_context.rs`, `system_inference.rs`.
  - The `run_profile/host/` subdirectory holding the individual `Loop*Port` trait files — `capability.rs`, `model.rs`, `transcript.rs`, `context.rs`, `input.rs`, `progress.rs`, `checkpoint.rs`, `run_context.rs`.
  - `loop_exit.rs` (984 lines, DTO half); `checkpoint_state.rs` (242 lines, port half — `CheckpointStateStorePort`).
  - `run_profile/` already ships its own `CLAUDE.md`, whose charter previews this crate's target charter verbatim: "Owns neutral run-profile and agent-loop host contracts... This directory defines contracts only. It must not construct concrete capability hosts, dispatchers, host runtime services, workspace readers, DB backends, product adapters, or provider clients" (`run_profile/CLAUDE.md:1-19`).
- **Migration delta:** wholesale carve-out — nothing here is deleted; everything moves from `ironclaw_turns` (which sheds `run_profile/`, `loop_exit`'s DTO half, and `checkpoint_state`'s port half as part of its own §6.5.8 narrowing).
- **Owns:**
  - The 11 `Loop*Port` traits (`LoopCapabilityPort`, `LoopModelPort`, `LoopPromptPort`, `LoopTranscriptPort`, `LoopContextPort`, `LoopInputPort`, `LoopRunInfoPort`, `LoopCancellationPort`, `LoopCompactionPort`, `LoopProgressPort`, `LoopCheckpointPort`) + the blanket `AgentLoopDriverHost`.
  - `AgentLoopDriver`; run-profile vocabulary; `LoopExit` and its evidence-ref DTOs; `CheckpointStateStorePort`; loop-side error vocabulary (`AgentLoopHostError*`, `LoopSafeSummary`, `CapabilityInputRef`).
- **Must never contain:** the coordinator, state store, or exit *applier* (those stay kernel-side in `ironclaw_turns`); model-gateway implementations; prompt *content*.
- **Allowed internal deps:** `ironclaw_host_api`, `ironclaw_common`, `ironclaw_prompt_envelope`.
- **Forbidden:** everything else internal — notably **not** `ironclaw_turns`; the direction inverts (turns implements/validates against these contracts, not the reverse).
- **Public contracts & ports:** the port set above, almost purely trait+DTO. Implementations today live across 5 crates for `LoopCapabilityPort` alone (15 production impls per the A2/A5 audit); the family's port-implementer census (§11.4) is what keeps that chain declared rather than accidental.
- **Security & authority role:** security-relevant contract boundary — the typed membrane between untrusted/replaceable loop userland and the kernel. `agent_loop`'s "contracts-layer deps only" rule becomes fully satisfiable through this one crate rather than by reaching into the turn kernel for vocabulary, which is what produces today's W4.3 exceptions.
- **Why a crate (not a module):** criterion 1+2 — six consumers (`agent_loop`, `loop_host`, `hooks`, `capabilities`, `extension_host`, `host_runtime`) need exactly this tier and today reach it through the kernel crate, producing seven of the twenty standing `LAYER_MATRIX_EXCEPTIONS`; the repo already treats `run_profile/` as a distinct contract (its own `CLAUDE.md`, ~14.3k lines, zero execution). The name resolves the "turn_contracts JIT split" milestone the exceptions cite: ID vocabulary goes to `host_api::turn` (already its canonical home), port/profile vocabulary comes here.
- **Enforcement:** NEW family⇄layer consistency check (§11.2.1); NEW port-location rule pinning `Loop*Port` definitions to this crate only (§11.2.4); the existing `run_profile/CLAUDE.md` boundary language becomes this crate's `AGENTS.md` seed text; exception ratchet (§11.2.2) proves the seven W4.3 exceptions this crate resolves do not silently reappear.

### `ironclaw_extension_contracts` — NEW

- **Path & disposition:** `crates/contracts/ironclaw_extension_contracts` — new, carved from `host_api::product_adapter` (channel/tool/egress/external/auth-mint parts), `host_api::{channel, channel_identity, recipe, memory, state, extension}`, `extension_host::entrypoint`, and the `PreferenceTargetCodec`/`ReplyTargetBindingRef` vocabulary (PROPOSAL §9 row 5; §6.1.2).
- **Purpose:** the neutral vocabulary of what an installable extension *is and exposes* — surfaces, adapters, recipes, states, and verified-inbound evidence — shared by lanes, hosts, packages, product, and the manager without any of them importing a registry or an owner.
- **Target contents:** fed from `host_api` (CURRENT, all confirmed this session):
  - `ChannelAdapter` (`product_adapter/channel_adapter.rs:27`, 5-method trait, 309 ln) + its `VerifiedInbound`/`InboundOutcome`/`NormalizedInboundMessage`/`OutboundEnvelope`/`OutboundPart`/`DeliveryReport`/`TargetQuery`/`TargetCandidate`/`ChannelError` vocabulary.
  - `ToolAdapter` + `RestrictedEgress` (`tool_adapter.rs`, 208 ln).
  - `Extension`/`ExtensionEntrypoint`/`ExtensionBindings`/`check_binding` — split today between `host_api::extension.rs:116` (`Extension` trait) and `extension_host::entrypoint.rs:35` (`ExtensionEntrypoint` trait, 186 ln — this half moves in from `extension_host`, not `host_api`).
  - Channel manifest-surface descriptors (`channel.rs`, 732 ln); auth recipe schema (`recipe.rs`, 935 ln); memory manifest surface (`memory.rs`, 144 ln).
  - `InstallationState` + `LifecyclePublicState` + `AuthAccountState` (`state.rs`, 187 ln); channel-identity hooks (`channel_identity.rs`, 72 ln).
  - `PreferenceTargetCodec` (`product_adapter/outbound.rs:54`, within a 2,738-line file) and `ReplyTargetBindingRef` (already canonically `host_api::turn.rs:248` via `bounded_ref!`, but reached today by `telegram_extension` only through `ironclaw_product`'s re-export at `product/src/lib.rs:147` — confirmed live).
  - The six sealed `mark_*_verified` evidence-mint functions (`product_adapter/auth.rs:366-470`, `#[cfg(feature = "host-auth-mint")]`).
- **Migration delta:** wholesale carve-out from `host_api` + a smaller carve-out from `extension_host` (the `ExtensionEntrypoint` trait half) + a re-homing (not a new definition) for `PreferenceTargetCodec`/`ReplyTargetBindingRef`, both of which are *already* host_api types reached indirectly. Nothing here is genuinely new vocabulary — every item has a live CURRENT definition site.
- **Owns:** the adapter traits + surface/recipe/state vocabulary + evidence types above.
- **Must never contain:** the registry or installation stores (→ `extensions/ironclaw_extensions`); lifecycle execution, binding orchestration, or ingress routing (→ `extensions/ironclaw_extension_host`); vendor names (scanner-enforced); WASM/MCP mechanics; product workflow.
- **Allowed internal deps:** `ironclaw_host_api`, `ironclaw_common`. **Forbidden:** everything else internal; no axum/reqwest/wasmtime.
- **Public contracts & ports:** the adapter traits + vocabulary above. Implementations: `ChannelAdapter` today has exactly two production implementors — `SlackChannelAdapter` (`slack_extension/src/channel.rs:40`) and `TelegramChannelAdapter` (`telegram_extension/src/channel.rs:97`) — plus `HostServedChannelBridge` infrastructure in `extension_host/src/generic_host.rs:446`; both real implementors stay in `extensions/packages/*` under the target.
- **Security & authority role:** **security/authority boundary** — it defines the shape of the host↔extension membrane and, after §6.1.2/§11.2.5, owns inbound-verification evidence minting exclusively; a vendor adapter can lie about parsed content but cannot forge verification or scope, because the sealed constructor lives here, not in the adapter's crate.
- **Why a crate (not a module):** criterion 1+2 — it is the "extension runtime descriptors move to a neutral contract" target four standing W7 exceptions already name (`mcp/scripts → extensions/resources`); it lets `mcp`, `wasm`, `sandbox`, channel packages, `extension_host`, `product`, and the manager share one vocabulary with no registry/product edge. Without it, either lanes depend on the registry crate (today's exception) or packages depend on product (today's telegram shape — confirmed: `telegram_extension/src/preference_targets.rs:10` imports `PreferenceTargetCodec` from `ironclaw_product`, not `ironclaw_host_api`, even though the trait is defined in `host_api`).
- **Enforcement:**
  - NEW port-location rule (§11.2.4) pinning `ChannelAdapter`/`ToolAdapter` definitions to this crate and closing the two-import-path trap the `PreferenceTargetCodec` case demonstrates.
  - NEW sealed-evidence rule (§11.2.5) making the mint functions callable only from the generic verifier, with the `host-auth-mint` feature itself removed and pinned gone by test.
  - The existing `reborn_extension_specificity.rs` vendor-name scanner (`ALLOWLIST` at line 1029, shrink-only) extends to cover this crate at zero budget from day one.

### `ironclaw_product_contracts` — NEW

- **Path & disposition:** `crates/contracts/ironclaw_product_contracts` — new, carved from `host_api` (product-surface/lifecycle/operator halves), `ironclaw_product` (delivery/admission/operator/lifecycle ports), and `ironclaw_common::event` (PROPOSAL §9 row 6; §6.1.3).
- **Purpose:** the neutral product-boundary vocabulary — the `ProductSurface` membrane, its caller/descriptor types, product wire DTOs, and the product-side ports whose implementations live beside/below product.
- **Target contents:** fed from `host_api` (CURRENT, confirmed this session):
  - `ProductSurface` + `BoundProductSurface` + `ProductSurfaceCaller` + invoke/query/stream DTOs + `ChannelInboundProductSurface` (`product_surface.rs`, 608 ln, `ProductSurface` trait :352).
  - Command/view/capability descriptor *types* (`ProductSurfaceCommandDescriptor`, `ProductCapabilityDescriptor`, `ProductView` — the types only; product's 27/33/18 concrete constants stay in `product/ironclaw_product` as the frozen inventory).
  - `package_lifecycle.rs` (650 ln), `operator_llm.rs` (140 ln), and the product-facing halves of `product_adapter/{inbound.rs 1537, projection.rs 207, external.rs 470}`.
  - Fed from `ironclaw_product` (CURRENT, all confirmed this session with exact definitions): `ChannelDeliveryResolver` (`delivery_coordinator.rs:124`), `DeliveryReplyContextSource` (`delivery_coordinator.rs:132`), `ProductCommandAdmissionService` (`command_dispatch.rs:72`), `LlmConfigService` (`reborn_services/llm_config.rs:57`), `ActiveModelReader` (`reborn_services/llm_config.rs:48`), `OperatorLogsService` (`reborn_services.rs:775`), `OperatorServiceLifecycleService` (`reborn_services.rs:798`), `OperatorStatusService` (`reborn_services.rs:733`), `LifecycleProductService` (`lifecycle.rs:48`), `AccountConnectionStatusSource` (`extension_account_setup.rs:34`), `ChannelConfigProductService` (`reborn_services.rs:712`), plus their DTO groups (`reborn_services/types.rs`, 1,836 ln of `Reborn*` wire DTOs).
  - Fed from `ironclaw_common`: `event.rs` (1,234 ln, the 43-variant `AppEvent` wire enum at line 201).
- **Migration delta:** eleven named single-impl ports move out of `ironclaw_product` into this crate without changing their implementations' locations (operator's five ports keep their sole implementors in `operator`; `ChannelConfigProductService`/`AccountConnectionStatusSource` keep theirs in `extension_host` — `channel_config.rs:689`, `channel_pairing.rs:1176`). `AppEvent` moves wholesale from `common`. Nothing here is deleted.
- **Owns:** everything listed above; all ports here follow the rule "defined here, implemented by exactly the crates the caller wires — product, operator, extension_host, extension_manager, composition."
- **Must never contain:** the `ProductSurface` *implementation* (stays in `product/ironclaw_product`); any handler/admission/delivery logic; HTTP anything; projections' reducers; vendor names beyond the LLM-vendor command-id strings already frozen on the wire (flagged §12.9 as a wire-compatibility constraint, not a violation).
- **Allowed internal deps:** `ironclaw_host_api`, `ironclaw_common`, `ironclaw_extension_contracts` (for channel-facing DTO reuse). **Forbidden:** everything else internal.
- **Public contracts & ports:** the `ProductSurface` membrane + eleven relocated ports above. The dependency inversion is explicit and total: every implementation of every port here is wired by whichever crate the composition root chooses, never by this crate.
- **Security & authority role:** domain-ownership boundary (product vocabulary) plus the compile-time enforcement of "transports use DTOs/descriptors only" — the discipline `webui`'s own guidance already demands but today only code review enforces.
- **Why a crate (not a module):** criterion 1+2+4 — it converts three review-enforced disciplines into Cargo facts: `webui`'s "DTOs/descriptors only" rule, operator's inverted contract ownership (its ports/DTOs are currently defined in the crate it must sit beside), and channel/`extension_host` port implementations that currently force those crates *above* product. It removes the `extension_host→product`, `operator→product`, `telegram_extension→product` edges and lets `webui`/`openai_compat` compile against a thin contracts crate instead of the 51.6k-line `ironclaw_product`.
- **Enforcement:**
  - `host_product_surface_method_set_is_frozen` (relocates here with `ProductSurface`) + `reborn_service_method_freeze_ratchet`'s companion `product_local_product_surface_traits_stay_retired`.
  - NEW port-location rule (§11.2.4) pinning `ProductSurface` to this crate exclusively.
  - NEW contracts-purity allowlist (§11.2.3) keeping the LLM-vendor command-id strings as the crate's only sanctioned vendor text (per PROPOSAL §8.1 rule 4's exact carve-out).

## Family AGENTS.md obligations

Per PROPOSAL §6.1's family intro and §11.4, `crates/contracts/AGENTS.md` must state, verbatim or near-verbatim:

- The four-part admission test (crosses an authority/host/product boundary; neutral across vendor/runtime/storage/deployment; needed by lower layers without importing an owner; carries no execution/persistence/policy/workflow) as the single gate for "does a new type belong here."
- The no-wildcard-re-export rule — `host_api`'s de-wildcarded prelude (post-migration) is the house style every contracts crate follows; a new flat `pub use module::*` is a review-blocking regression.
- "A new type here requires naming the two-or-more consumers that cannot both import an owner" — the exact test that justified each of the three NEW crates and must gate any future fourth.
- The port-location rule in plain language: a port's *definition* lives in contracts; its *implementations* live wherever the caller wires them, and no contracts crate may re-export another crate's port under its own path (the trap `PreferenceTargetCodec` demonstrates today).
- The external-dependency ceiling (no axum/reqwest/wasmtime/DB clients, ever) and the six crates' individual allowlists from §6.1, so a reviewer can check a `Cargo.toml` diff against this file alone.
- A pointer to each crate's own guidance file for its specific vocabulary inventory — this file stays the family-wide gate, not a duplicate crate index (the audit's own drift finding: the hand-maintained `crates/AGENTS.md` table decays; family roots replace it, PROPOSAL §11.4).

## Current → target summary

| Target crate | Current name/location | Disposition |
|---|---|---|
| `contracts/ironclaw_host_api` | `crates/ironclaw_host_api` | retain-narrow + split (sheds ~9.8k ln to the two new siblings; §6.1.1) |
| `contracts/ironclaw_common` | `crates/ironclaw_common` | retain-narrow (sheds `event.rs`→product_contracts, llm data→llm, `trust_boundary`→deleted; §6.1.5) |
| `contracts/ironclaw_prompt_envelope` | `crates/ironclaw_prompt_envelope` | retain as-is + move (manifest fix, add guidance; §6.1.6) |
| `contracts/ironclaw_loop_contracts` | — (NEW) | carved from `ironclaw_turns::run_profile/**` + `loop_exit` (DTO half) + `checkpoint_state` (port half); §6.1.4 |
| `contracts/ironclaw_extension_contracts` | — (NEW) | carved from `host_api::{product_adapter, channel, channel_identity, recipe, memory, state, extension}` + `extension_host::entrypoint` + re-homed `PreferenceTargetCodec`/`ReplyTargetBindingRef`; §6.1.2 |
| `contracts/ironclaw_product_contracts` | — (NEW) | carved from `host_api::{product_surface, package_lifecycle, operator_llm, product_adapter product-halves}` + 11 ports from `ironclaw_product` + `common::event`; §6.1.3 |
