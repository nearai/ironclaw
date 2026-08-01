# `crates/contracts/` — neutral vocabulary and ports

**Layer(s):** contracts · **Crates:** 6 — `ironclaw_host_api`, `ironclaw_common`, `ironclaw_prompt_envelope`, `ironclaw_loop_contracts`, `ironclaw_extension_contracts`, `ironclaw_product_contracts` · **Security posture:** executes nothing and persists nothing, yet is the only family that may declare sealed authority constructors — a defect here can misdescribe authority but can never grant it.

*This document specifies the target architecture as designed. Dispositions, migration constraints, evidence, and open decisions live in [PROPOSAL.md](../PROPOSAL.md), [CHECKLIST.md](../CHECKLIST.md), and [PLAN.md](../PLAN.md).*

> ✎ **Landed marker — the three port crates now exist (2026-08-01, audited at `a50ad0638`).** `ironclaw_loop_contracts` (#6975), `ironclaw_extension_contracts` (#6977, extended by #6980 and #6981), and `ironclaw_product_contracts` (#6980) are on `main`, flat under `crates/` until the family directory arrives in Wave 5. This file stays a **forward specification** and is not rewritten to the built shape. Where building it produced a *recorded correction* to what is specified here, the spec says so inline: **three ✎ corrections** (`AuthAccountState` and `ReplyTargetBindingRef` under `ironclaw_extension_contracts`, and the event wire enumeration under `ironclaw_product_contracts`) and **one ✎ addition** (the hosted-MCP registration vocabulary, which arrived mid-wave with #6930). **The built tree also diverges from this spec in three further places that are debt rather than correction, and this marker names them so the file is not read as complete parity.** All three are `ironclaw_loop_contracts`, all three are marked ✎ at that entry below, and all three are recorded with their evidence in PROPOSAL §6.1.4: its dependency set is not the one specified here (it holds `ironclaw_extension_contracts`, which §8.2's contracts row sanctions; it holds neither `ironclaw_common` nor `ironclaw_prompt_envelope`); it carries a `tokio` dependency, waived in its manifest against §11.2.3's contracts-purity rule; and it embeds a prompt asset against this file's own "Never contains … prompt content" clause — a WS4 debt on the `loop_host` re-charter row, not a settled charter change. Per-module as-built inventories, with the dispositions that forced them, are in PROPOSAL §6.1.1–§6.1.4.

```text
crates/contracts/
├── ironclaw_host_api              authority vocabulary & sealed witnesses
├── ironclaw_common                cross-domain primitives
├── ironclaw_prompt_envelope       untrusted-snippet envelope
├── ironclaw_loop_contracts        the loop-tier port set
├── ironclaw_extension_contracts   extension surfaces, recipes & inbound evidence
└── ironclaw_product_contracts     ProductSurface, wire DTOs & product ports
```

## Role

Contracts is the vocabulary tier: the one family every other family depends on, and which depends on nothing itself. A type earns a home here by passing a four-part test — it names a concept that crosses an authority, host, or product boundary; it is neutral with respect to vendor, runtime, storage, and deployment; it is needed by two or more consumers that must not import one another; and it carries no execution, persistence, policy engine, or workflow. Four kinds of vocabulary live here: the identity and authority primitives shared by the whole workspace (`ironclaw_host_api`, `ironclaw_common`); the untrusted-content fence every prompt-construction path wraps snippets with (`ironclaw_prompt_envelope`); and three purpose-built membranes, one per adjacent family whose authority crosses a boundary, for the loop tier, the extension surface, and the product surface (`ironclaw_loop_contracts`, `ironclaw_extension_contracts`, `ironclaw_product_contracts`). Nothing in this family runs, stores, or decides; it only names.

## Boundaries — what makes this family distinct

- **vs `substrates/`:** substrate owns privileged *mechanism* — real backends and drivers, disk and database engines, keychains, DNS resolvers; contracts owns only the *shape* those mechanisms accept and return, with zero I/O of its own. A substrate crate takes contracts types as input and does real work; a contracts crate never does work at all.
- **vs `domains/`:** domains own durable record grammar and service identity built on top of a storage fabric; contracts owns only the vocabulary those services accept and return. If a type has a persistence story, it belongs in domains; if it only names a boundary crossing, it belongs here — domains depend on contracts, never the reverse.
- **vs `kernel/`:** kernel makes the authority *decision* and mints privileged instances; contracts declares the sealed *types* and the *port* a decision flows through, constructing none of it itself. `CapabilityDispatcher` is declared in `host_api`; its production authority, `RuntimeDispatcher`, lives in the kernel. The port lives low; the power to satisfy it lives high.
- **vs `loop/`:** loop holds the port *implementations* — host-runtime-backed capability, model, transcript, and context adapters; contracts holds the port *definitions*. The agent loop itself may depend on contracts and nothing else, which is the strictest dependency rule in the workspace and the clearest proof of this line.
- **vs `extensions/`:** extensions own the registry, the installation records, and the generic hosting machinery; contracts owns only what an extension *is and exposes* — the adapter trait, the manifest-surface descriptors, the verified-inbound evidence. Nothing that runs an extension lives here; only the shape of what it must implement.
- **vs `product/`:** product owns the `ProductSurface` implementation and all admission, delivery, and binding behavior; contracts owns the membrane's shape and the ports whose real implementations sit beside product — the operator's LLM-admin services, the extension host's delivery resolver. Those ports are declared here precisely so product's collaborators never need to depend on product itself.

## What belongs here / What never belongs here

**Belongs here:**
- Identity, scope, path, and mount vocabulary; capability, action, decision, approval, resource, and audit shapes.
- Sealed authority witnesses and the ports they gate — declared, never privilegedly constructed, outside kernel or host code.
- Cross-boundary adapter and surface trait definitions — never their implementations.
- Wire DTO homes shared by transports that must never see each other's owners.
- Domain-free cross-cutting primitives with long-lived wire-compatibility guarantees.

**Never belongs here:**
- Any port or trait implementation, any storage, any HTTP, database, or WASM runtime client.
- Rendering, parsing, or classification behavior — that is workflow, and workflow lives above this family.
- Logging or channel side effects of any kind.
- Vendor names, or framework types — HTTP frameworks stay in the product tier so contracts never needs one.
- Persistence ports — even a bare trait — for the same reason a domain's store interface belongs in the domain, not in the vocabulary crate that describes it.

## Dependency direction

- **Depends on:** nothing, for the three foundational crates (`host_api`, `common`, `prompt_envelope`) — each is a leaf. The three port crates depend only within the family: `loop_contracts` on `host_api`, `common`, and `prompt_envelope`; `extension_contracts` on `host_api` and `common`; `product_contracts` on `host_api`, `common`, and `extension_contracts` for channel-facing DTO reuse.
- **Never depends on:** any crate outside this family; no HTTP framework, no database client, no WASM runtime.
- **Depended on by:** every other family — substrate, events, domains, kernel, lanes, loop, extensions, product, and app all resolve to contracts somewhere in their dependency graph. No other family has that property; it is the definition of "leaf tier."
- **Inversions:** every privileged port in this family is defined low and implemented high. `CapabilityDispatcher` is satisfied by the kernel's dispatch authority; `ChannelAdapter` and `ToolAdapter` are satisfied by extension packages; `ProductSurface` and its companion ports are satisfied by product, operator, the extension host, the extension manager, and composition; the `Loop*Port` set is satisfied by the loop-hosting tier. A port belongs here exactly when the lower layer must invoke behavior whose implementation cannot live below the caller — anything that fails that test is not a port, it is an unnecessary indirection, and has no place in this family.

## Security & authority

Contracts holds the sealed constructors for the `Authorized` witness, the privileged variants of `TrustClass`, and the verified-inbound and bearer/session evidence types. None of these can be constructed outside its sealed constructor path: the capability membrane mints `Authorized`, a host authenticator mints bearer/session evidence, a generic ingress verifier mints channel evidence. ✎ **Corrected 2026-08-01 (was: "the sealed constructors that make forged authority a *compile-time impossibility* rather than a review discipline … callable in production only by the sanctioned minters, enforced by constructor visibility plus a workspace string-scan pin"):** that stated two different strengths of claim as one, and only the first is the compiler's. The **compile-time** guarantee is that evidence cannot be constructed without a grant, so nothing outside a workspace Rust crate can mint at all — an installed package, a channel adapter, a product handler, full stop. **Which** workspace crate may hold a grant is not a compiler guarantee but an architecture-test one, and the clarification directly below states what that test does and does not prove. Because this family executes nothing and persists nothing, a defect here can misdescribe authority — a wrong field, a bad DTO — but can never grant it. That is exactly why the family's admission test forbids execution and persistence: either one would turn a vocabulary crate into a second place authority could originate.

✎ **Two clarifications the built crates forced, recorded 2026-08-01 so the absence of a mechanism reads as a decision rather than an omission.**

**Sealing is not the family's default — witnessing is.** "Sealed" here means a *constructor* is unreachable, not that a trait's implementor set is closed. Almost every trait in this family is deliberately **open**: they exist to be implemented above, by the crate that owns the behavior, and closing them would forbid exactly the extensibility the model is built on — a channel package implements the adapter and the preference codec, an extension implements the entrypoint and its identity hooks, the kernel implements the dispatch port. A trait here that genuinely needs a closed implementor set is a signal it belongs in an owner crate instead. What *is* closed is a small, named set of constructors — the authority witness, the privileged trust variants, and the inbound-verification evidence — and each is closed by the same idiom: a zero-sized grant whose field is private to the declaring crate, obtainable only from the provided body of one trait method that exactly one crate may implement. Because pure cross-crate type-sealing is not expressible in Rust, that seal has two halves by construction: **the compiler enforces "no minting without a grant," and an architecture test enforces "only one crate may implement the grant-producing trait."** A reviewer should read the second half as the weaker one — measurably so. The grant-producing traits (`HostProtocolAuthenticator`, `ChannelIngressVerifier`) are public and unsealed, and `reborn_sealed_evidence_mint_ratchet` polices them with a line-oriented substring scan for `"<Trait> for"` over comment-stripped production sources, so a workspace crate that aliases the trait on import or splits its `impl` header across lines evades that census while the test still reports `permitted_impls == 1`. A second census over the mint-function *names* is the backstop a forger would also have to defeat, which is why this is a hardening item rather than a live hole — it is on the WS10 guardrail row, and PROPOSAL §12.1(a) carries the same statement with the file:line detail. It bounds a rogue *workspace* crate either way: a package or a handler obtains no grant by any route. A reviewer should also treat any claim that a *cargo feature* seals something as unproven until measured — feature unification makes a feature enabled anywhere enabled everywhere in the same build (PROPOSAL §12.1a).

**"No implementations" governs ports, and the exceptions are visible ones.** A contracts crate never implements a port it declares — that is the rule that keeps the port's power above its definition. Two shapes are permitted and should not be mistaken for drift: test doubles behind an explicit test-support gate, which exist so consumers can build fakes without inventing a parallel vocabulary; and pure forwarding implementations over smart pointers, which add no behavior. Anything else — a real implementation, a default body that does work, an embedded asset — is a breach whether or not it is convenient, and it is recorded as debt with a named owner rather than absorbed into the charter. One such breach stands today in the loop-tier crate (an embedded prompt asset, taken deliberately because it bought the removal of a dependency exception) and is tracked to the loop-hosting tier for resolution; PROPOSAL §6.1.4 and CHECKLIST WS4 carry it.

## Crates

### `ironclaw_host_api`

- **Purpose:** the dependency-free authority vocabulary of the whole system — identities, scopes, paths, mounts, capability and decision shapes, the sealed dispatch port, sanitized resolution and failure vocabulary, egress and ingress descriptors, runtime and trust vocabulary, and the canonical turn vocabulary.
- **Owns:**
  - `ids`, `scope`, `path`, `mount`, `error` — the identity and authority primitives every other crate in the workspace builds on.
  - `capability`, `action`, `decision`, `approval` — the requested-effect and host-decision vocabulary that flows through every capability invocation.
  - `authorized` and `dispatch` — the sealed `Authorized` witness and the `CapabilityDispatcher` port it is handed to.
  - `invocation` and `lane` — the closed `RuntimeLane` enumeration and invocation identity.
  - a resolution and failure cluster — sanitized resolution results, gate records, safe summaries, model-result previews, host remediation guidance, and credential redaction.
  - `resource`, `audit`, `host_port` — budget and quota vocabulary, redacted audit envelopes, and host-port identity.
  - `http` and `ingress` — the `RuntimeHttpEgress` port and the ingress route descriptor, policy, and listener-class vocabulary.
  - `runtime`, `runtime_policy`, `trust` — runtime-kind and trust-class vocabulary, deployment-mode and policy shapes.
  - `turn` — the complete canonical turn vocabulary: turn and run identity, status, and the small set of turn-scoped reference types — such as the reply-target binding a channel adapter resolves against — that any crate touching a turn needs without importing the turn kernel.
- **Never contains:** vendor names; adapter-trait implementations; product surface or product DTOs; loop-port traits; rendering, parsing, or classification helpers; logging or async-runtime side effects; persistence ports.
- **Public surface:** the vocabulary above, plus sealed constructors for `Authorized` and for bearer/session evidence. The evidence constructors are callable in production only by the host authenticator that performs verification — enforced by visibility plus the workspace scan; no other caller, including a channel adapter or a product handler, can construct them.
- **Depends on:** nothing internal.
- **Never depends on:** any other crate in the workspace; any HTTP framework, database client, or WASM runtime.
- **Security & authority role:** the authority-vocabulary boundary for the whole system. It is security-relevant purely by declaration — it holds the sealed `Authorized` and `TrustClass` constructors that every privileged path in the system is built on.
- **Why a separate crate:** it is the one contract nearly every other crate in the workspace depends on, and its zero-dependency posture is what makes that safe — any dependency added here becomes a dependency of the entire system.

### `ironclaw_common`

- **Purpose:** domain-free cross-cutting primitives that carry long-lived, persisted-compatibility guarantees.
- **Owns:**
  - `identity` — the credential, extension, MCP-server, and external-thread identity newtypes that anchor the workspace's newtype discipline, including the one documented wire-compatibility exception this crate is permitted to carry.
  - `pkce`, `hashing`, `paths`, `timezone`, `util`, `env_helpers` — small, genuinely domain-free helpers.
  - `attachment` — a generic attachment reference and format vocabulary, distinct in name and shape from the channel-facing attachment reference that `ironclaw_extension_contracts` owns for vendor payloads.
- **Never contains:** wire protocols or event vocabulary, which belong to `product_contracts`; LLM domain data; prompt-construction data; budget-policy constants; or any scaffolding without a live consumer.
- **Public surface:** the primitives above. This crate is data, not behavior — it defines almost no traits.
- **Depends on:** nothing internal.
- **Never depends on:** any other crate in the workspace.
- **Security & authority role:** domain-ownership boundary for cross-domain primitives, and the sole place allowed to carry a documented backward-compatibility exception for a persisted wire format rather than a clean invariant.
- **Why a separate crate:** a genuinely domain-free primitive surface with consumers across every layer; the persisted-compatibility exception needs exactly one home so it cannot silently reappear elsewhere.

### `ironclaw_prompt_envelope`

- **Purpose:** the one primitive that wraps untrusted, model-visible content with an explicit, closed-vocabulary trust marker before it reaches a model.
- **Owns:** `wrap_untrusted` and its bounded-length variant; a closed `EnvelopeSource` enumeration (memory, hook, skill); an `EnvelopeTrust` classification (trusted, untrusted); the instruction-hijack marker denylist; and the byte budget that caps a wrapped envelope.
- **Never contains:** model routing, policy, free-form source labels, or an additional envelope source without a deliberate, reviewed API change — adding a source is a security-relevant decision, not a routine one.
- **Public surface:** `wrap_untrusted` and its bounded variant; no traits — pure functions over closed enumerations.
- **Depends on:** nothing internal.
- **Never depends on:** any other crate in the workspace, by design — its dependency-free posture is itself part of the guarantee.
- **Security & authority role:** the prompt-injection fence between untrusted content sources and the model. Any crate that hands untrusted, source-attributed text to a model does so through this envelope.
- **Why a separate crate:** its consumers each own exactly one `EnvelopeSource` variant (memory, hooks, skills); folding it into a larger safety crate would hand every consumer a much heavier detection and redaction dependency for one small, security-critical function.

### `ironclaw_loop_contracts`

- **Purpose:** the loop-tier contract — how any loop, hook, or host adapter talks to the turn kernel without importing it.
- **Owns:**
  - the `Loop*Port` family — capability, model, prompt, transcript, context, input, run-info, cancellation, compaction, progress, and checkpoint ports — plus the blanket `AgentLoopDriverHost` a host implements to expose them together.
  - `AgentLoopDriver` and the run-profile vocabulary: resolved profile shape, capability surface, context and checkpoint policy, prompt and model contract types.
  - `LoopExit` and its evidence-reference DTOs — the claim a loop makes about how its turn ended, which only the kernel may validate into a durable transition.
  - `CheckpointStateStorePort` and loop-side error and safe-summary vocabulary.
- **Never contains:** the turn coordinator, the turn state store, or the exit applier that validates a `LoopExit` claim — those are kernel authority, not contract vocabulary. Never contains model-gateway implementations or prompt content itself. ✎ **As-built divergence, recorded 2026-08-01 (not a charter change):** the built crate *does* hold prompt content — `instruction_bundle.rs:29` embeds `prompts/capability_surface_usage_policy.md` via `include_str!`. It came anyway because `ironclaw_hooks` consumes `InstructionMaterializationStore` and leaving the module in the turn kernel would have kept the `hooks → turns` exception alive, i.e. the breach bought an exception deletion. The clause above stands; the resolution is to hoist `InstructionBundleBuilder` to `ironclaw_loop_host` and leave the bundle/store *types* here — a WS4 item on the CHECKLIST `loop_host` re-charter row (#6975; PROPOSAL §6.1.4, §6.7.2).
- **Public surface:** the port family above, almost entirely trait and DTO. Each port has exactly one canonical implementation chain, declared by the loop-hosting tier that hosts it, so no implementation silently shadows another.
- **Depends on:** `ironclaw_host_api`, `ironclaw_common`, `ironclaw_prompt_envelope`. ✎ **As-built divergence, recorded 2026-08-01:** at `a50ad0638` the manifest names `ironclaw_host_api` and **`ironclaw_extension_contracts`**, and neither `ironclaw_common` nor `ironclaw_prompt_envelope`. The extension-contracts edge is sanctioned, not waived — `LoopRuntimeContext` carries `Option<ChannelPresentation>` and `render_presentation_hint` reads it, and §8.2's contracts row already reads "others: host_api/common±extension_contracts"; the *Allowed deps* list in PROPOSAL §6.1.4 predates the extension tier existing and was amended in place with that evidence. Also carved with evidence and not reflected above: a `tokio` dependency with the `rt` feature only, for the single `JoinHandle` in `CommunicationContextFetch::Spawned`, explicitly waived in the manifest against §11.2.3 and pinned by the contracts-purity allowlist, which denies every other framework outright (#6975; PROPOSAL §6.1.4).
- **Never depends on:** the turn kernel itself — the direction inverts: the turn kernel implements and validates against these contracts, never the reverse.
- **Security & authority role:** the typed membrane between replaceable loop userland and the kernel. The agent loop's rule that it may depend on contracts and nothing else is fully satisfiable through this one crate, so a loop implementation never has a reason to reach past it toward kernel vocabulary.
- **Why a separate crate:** the loop-hosting tier, the hook framework, and every kernel crate that constructs or consumes a loop need exactly this vocabulary and nothing more of the turn kernel; keeping it separate from the turn kernel is what lets the turn kernel evolve its internal state machinery without touching the loop-side contract.

### `ironclaw_extension_contracts`

- **Purpose:** the neutral vocabulary of what an installable extension is and exposes — surfaces, adapters, recipes, states, and verified-inbound evidence — shared by lanes, hosts, packages, and product without any of them importing the extension registry.
- **Owns:**
  - `ChannelAdapter` — a small trait an extension package implements once for inbound normalization, outbound rendering and delivery, and target resolution — plus its supporting vocabulary: normalized inbound messages, outbound envelopes and parts, delivery reports, target queries and candidates, and channel error shapes.
  - the channel-facing vendor attachment reference, distinct in name and shape from `ironclaw_common`'s generic attachment vocabulary.
  - `ToolAdapter` and `RestrictedEgress` — the model-callable-tool counterpart.
  - `Extension` and `ExtensionEntrypoint` — the manifest-bound entrypoint every extension package exposes, and the bindings it returns.
  - Channel manifest-surface descriptors, the auth recipe schema, and the memory manifest surface — the declarative shape a manifest compiles into.
  - `InstallationState` and `LifecyclePublicState` — the caller-visible lifecycle vocabulary. ✎ **Corrected 2026-08-01 (was: "and `AuthAccountState`"):** `AuthAccountState` stays in `ironclaw_auth`, beside the engine that drives it — moving the enum without `AuthAccountLastError` and `project_auth_account_state` would split a state machine from its projection, which is a domain call, not a contracts extraction (#6977; PROPOSAL §6.1.2).
  - `PreferenceTargetCodec` — the vocabulary a channel package needs to encode and decode a user's delivery preference without depending on product. ✎ **Corrected 2026-08-01 (was: "`PreferenceTargetCodec` and `ReplyTargetBindingRef`"):** `ReplyTargetBindingRef` is *turn* vocabulary and was already home in `ironclaw_host_api`'s `turn` module — which this file's `ironclaw_host_api` entry above already claims. Bringing it here would undo that consolidation and force the loop-hosting tier, product, and the assembly root onto this crate for turn vocabulary. The reason this crate was given it — the upward edges a channel package was forced into — is satisfied by `PreferenceTargetCodec` alone: the Telegram package dropped its product dependency outright on the strength of that one move (#6977; PROPOSAL §6.1.2).
  - the hosted-MCP registration vocabulary and the two bounded lifecycle identities it is structurally bound to. ✎ **Added 2026-08-01:** not in the original spec because the concept arrived with #6930 mid-wave; it is extension-tier by its own charter (registration input describing what an installable extension *is*), and it must sit wherever the lifecycle identities sit or the dependency between the two inverts (PROPOSAL §6.1.1/§6.1.2).
  - the sealed verified-inbound evidence constructors — mintable only from within the generic ingress verifier that performs signature or shared-secret verification.
- **Never contains:** the extension registry or installation records; lifecycle execution, binding orchestration, or ingress routing; vendor names; WASM or MCP mechanics; product workflow.
- **Public surface:** the adapter traits and vocabulary above. `ChannelAdapter` is implemented once per channel package; `ToolAdapter` similarly. Both are consumed generically by the extension host and by lanes that never need to know which vendor they are talking to.
- **Depends on:** `ironclaw_host_api`, `ironclaw_common`.
- **Never depends on:** the extension registry, the extension host, or any product crate; no HTTP framework, no WASM runtime.
- **Security & authority role:** the host-to-extension membrane. It owns inbound-verification evidence minting exclusively, so a channel package can misreport parsed content but can never forge verification or scope — the sealed constructor lives here, never in the package.
- **Why a separate crate:** it lets every lane, the generic extension host, every channel package, and the product-side extension manager share one vocabulary with no dependency on the registry or on product — the single contract that keeps "installable package" a four-way-separated responsibility instead of a tangle.

### `ironclaw_product_contracts`

- **Purpose:** the neutral product-boundary vocabulary — the `ProductSurface` membrane, its caller and descriptor types, product wire DTOs, and the product-side ports whose real implementations sit beside or below product.
- **Owns:**
  - `ProductSurface`, `BoundProductSurface`, `ProductSurfaceCaller`, and the invoke/query/stream DTOs that cross the membrane — the single generic entry point every transport (webui, the OpenAI-compatible adapter, a channel package) calls through.
  - the command, view, and capability descriptor *types* — the shapes a concrete command or view instantiates, not the frozen inventory of concrete commands itself, which stays with product.
  - product wire DTOs: lifecycle projections and operator menu vocabulary. ✎ **Corrected 2026-08-01 (was: "and the full event wire enumeration a transport streams to a client"):** there is no such enumeration to own. The 43-variant event enum this clause described was measured to have zero consumers outside its own crate — no transport ever streamed it, and the only live references were negative, chiefly an architecture test asserting the OpenAI-compatible routes must **not** stream it — so it was deleted rather than relocated (#6980 declined the move as an inventory correction; #6982 executed the deletion). The charter is unchanged: this crate is where product wire DTOs live. It simply has one fewer member than the spec assumed, and a reintroduction pin now guards the name (PROPOSAL §6.1.3).
  - product-side ports whose implementations live beside product: channel delivery resolution and reply-context sourcing, command admission, the operator's LLM-config, active-model, logs, service-lifecycle, and status services, lifecycle product service vocabulary, account-connection status sourcing, and channel-config product service.
- **Never contains:** the `ProductSurface` implementation itself; any handler, admission, or delivery logic; HTTP of any kind; projection reducers.
- **Public surface:** the `ProductSurface` membrane and the ports above. Every port here is defined once and implemented by exactly the crate that owns the behavior — product, operator, the extension host, the extension manager, or composition — never by more than one, and never by this crate.
- **Depends on:** `ironclaw_host_api`, `ironclaw_common`, `ironclaw_extension_contracts` for channel-facing DTO reuse.
- **Never depends on:** product, operator, the extension host, or any transport crate.
- **Security & authority role:** the compile-time enforcement of "a transport consumes DTOs and descriptors, never an implementation" — the discipline that keeps webui, the OpenAI-compatible adapter, and every channel package from reaching into product's internals.
- **Why a separate crate:** operator's ports and DTOs belong beside operator, not inside product; a channel package's delivery-resolution needs belong beside the channel, not inside product. Declaring all of it here, once, removes every reason for a transport or an operator surface to depend on product's full implementation just to see its own port.

## Family AGENTS.md requirements

Each family root states, for every crate beneath it, the same four things a reviewer needs without reading source: what admission test a new type must pass before it can live here; which two-or-more consumers justify it; which crate implements each port declared here, and where the boundary between declaring a port and implementing one sits; and the closed set of frameworks and cross-family dependencies this family may never acquire. `crates/contracts/AGENTS.md` states, specifically:

- The four-part admission test as the single gate for "does a new type belong here," with an explicit instruction to name the two-or-more consumers that justify a new type before adding it.
- The re-export discipline: every crate in this family exports its vocabulary module by module, never through a single flat wildcard — a reviewer should always be able to see which module a type comes from.
- The port-location rule in plain language: a port's definition lives in contracts; its implementations live wherever the crate that owns the behavior wires them; and no contracts crate re-exports another contracts crate's port under its own path.
- The dependency ceiling: no HTTP framework, database client, or WASM runtime, ever, in any crate in this family — and the short, closed list of which contracts crate may depend on which other one.
