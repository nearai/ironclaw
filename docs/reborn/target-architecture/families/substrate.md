# Family: `crates/substrate/` — privileged mechanism substrates

**Layer(s):** `substrates` (all five crates; one layer above `contracts`) · **Crates (target):** 5 — `ironclaw_filesystem`, `ironclaw_secrets`, `ironclaw_network`, `ironclaw_safety`, `ironclaw_observability` · **Security posture:** each crate is a mediated *mechanism* the kernel invokes on behalf of an already-decided effect; none of the five makes an authority decision itself — containment, custody, policy-enforcement, and detection primitives only, fail-closed by local invariant, never by ambient trust.

*Authority: PROPOSAL.md §6.2 (family role + all five crate entries), §5 (tree), §8 (dependency model), §9 rows 7–11, §12.1/§12.10 (risks). CURRENT-state citations are to this session's read of `dde662d5a`.*

## Identity — what this family IS

Substrate holds the durable, reusable *mechanisms* the kernel mediates: storage fabric, secret storage, network policy plus transport, safety scanning, and cross-cutting tracing (PROPOSAL §6.2 family role, verbatim intent). A crate belongs here iff it is a backend-generic mechanism with real containment and fail-closed local invariants; it does not belong here if it makes an authority decision (that is kernel's job) or owns domain record grammar (that is domains' job).

CURRENTLY this is five crates totaling 31,416 lines measured this session:

- `ironclaw_filesystem` (14,840 ln, 20 files)
- `ironclaw_secrets` (7,290 ln, 7 files)
- `ironclaw_safety` (7,226 ln, 11 files)
- `ironclaw_network` (1,937 ln, 9 files)
- `ironclaw_observability` (123 ln, 1 file)

This is the family with the widest size spread in the workspace by design — each crate is isolated because of a genuine driver/OS/regex cone, not a conceptual split: `filesystem` alone pulls `libsql`+`deadpool-postgres`+`tokio-postgres`; `secrets` pulls `aes-gcm`+the OS keychain (`security-framework` on macOS, `secret-service` on Linux); `network` pulls `reqwest`; `safety` pulls `aho-corasick`+`regex`. TARGET: all five retained, two narrowed (`secrets`, `network` shed unwired/misplaced subsystems), none merged or split — this is the one family in the target tree that changes least.

## What makes it distinct

- **vs `contracts/`:** contracts is vocabulary with zero I/O; substrate is the mechanism that vocabulary describes. Proof: `host_api` (contracts) has 0 internal deps and 0 external framework/DB deps, while `filesystem` (substrate) alone pulls the `libsql`+`deadpool-postgres`+`tokio-postgres` stack. A substrate crate takes `host_api` types as input and does real work; a contracts crate never does work at all.
- **vs `domains/`:** domains own record *grammar* and durable service *identity* (a trigger record, a thread's transcript, a memory item); substrate owns the storage/policy/safety *mechanism* domains are built on top of. The line: if a crate answers "what is true about this entity," it is domains; if it answers "how do bytes, secrets, network calls, or redaction actually happen," it is substrate — domains call `ScopedFilesystem`, they never reimplement it.
- **vs `kernel/`:** kernel decides whether an effect is authorized; substrate performs the effect once kernel has decided. `filesystem`/`secrets`/`network` never authorize — each enforces only its own local invariant (mount containment, one-shot lease consumption, private-IP denial) — the grant to reach them at all comes from `kernel/ironclaw_capabilities`. Today's direct-consumer finding (`auth`, `webui`, `operator` all hold a normal Cargo dep on `ironclaw_secrets`, confirmed this session) is exactly the seam PROPOSAL tightens, because it lets product-tier crates reach a substrate mechanism without kernel mediation in between.
- **vs `events/`:** both declare `layer = substrates`, but the contract shape differs — `events` owns canonical redacted evidence and durable log *traits*, never a mutable resource with a lease or a mount; substrate (`filesystem` specifically) is what `events`' own backend crate (`event_store`) is *built on*. Filesystem is the fabric; events is one specific append-only consumer of a slice of it.
- **vs `lanes/`:** lanes never depend on substrate crates directly — the forbidden-edge matrix states it explicitly ("mediated services arrive by injection," PROPOSAL §8.2). A `wasm` or `mcp` lane receives a narrowed mount, a staged one-shot secret, or a policy-scoped egress handle from `kernel/host_runtime`; it never holds a live `ironclaw_filesystem`/`ironclaw_secrets`/`ironclaw_network` handle of its own. This is the sharpest boundary in the whole dependency model — it is the operational definition of "an already-authorized invocation."

## What belongs here / What must never be here

**Belongs here:**
- A backend-generic mechanism with a real driver, OS-integration, or pattern-matching cone that would burden every consumer if inlined.
- Containment and compare-and-swap primitives (`ScopedFilesystem`'s `MountView` enforcement, `cas_update`'s 32-retry/15s-timeout floor).
- One-shot lease/consume primitives (`SecretStorePort::lease_once`/`consume`).
- Hardened egress transport and DNS/private-IP policy enforcement.
- Pattern-based detection, validation, and redaction (prompt injection, secret leaks, credential material).
- Zero-cost-when-off cross-cutting instrumentation (the observability macros).

**Must never be here:**
- Any authority *decision* — a substrate crate that started deciding who may call it would duplicate `kernel/ironclaw_authorization`'s job with none of its fail-closed guarantees.
- Domain record schemas or service identity (a thread, a trigger, a memory item) — those are `domains/`'s job, built on top of this family.
- Product or vendor behavior, branching, or naming — every crate in this family is deliberately vendor-blind.
- Ambient credentials or ambient network reachability handed to an unmediated caller — every credential/egress path in this family is scoped, leased, or policy-checked per call, never a standing client.
- Demo/placeholder backends running as if they were production (`HsmBackend`'s own doc comment: "It is not a security boundary," `hsm.rs:28-29`).

## Dependency direction

- **May depend on (internal):**
  - `ironclaw_host_api` + `ironclaw_observability` — all five crates, at the family floor.
  - `ironclaw_filesystem` additionally depends on `ironclaw_safety` for exactly one function (`is_sensitive_path`, `local.rs:8,490`).
  - `ironclaw_secrets` additionally depends on `ironclaw_filesystem`.
  - No substrate crate depends on another substrate crate beyond those two charted exceptions — this is deliberate: a lattice of substrate-on-substrate deps would recreate the driver-cone leakage the family exists to prevent.
- **Who may depend on it:**
  - `kernel/` is the primary consumer — `host_runtime` mediates `filesystem`/`secrets`/`network` for every crate above it, so most upper-tier access to this family is indirect.
  - `domains/` holds direct `RootFilesystem`/`ScopedFilesystem` access (backend-neutral persistence is the whole point of the storage-placement hybrid rule).
  - `events/` (`event_store`) backs onto `filesystem`.
  - `app/` (`ironclaw_composition`) selects backends and constructs the concrete implementations.
  - Per the forbidden-edge matrix, no `lanes/`, `loop/`, `extensions/`, or `product/` crate may hold a normal dep on a substrate crate directly — mediated services arrive by injection only.
- **Notable inversions:** none *within* substrate — this family has no ports it doesn't itself implement; it is the implementation tier the ports above it target. The interesting inversion runs the other direction: `RootFilesystem` is itself the target of a dependency inversion (PROPOSAL §8.1 rule 3, "`RootFilesystem` (filesystem → domain adapters)") — higher crates hold `Arc<dyn RootFilesystem>` against this crate's trait, never a concrete backend type, so swapping Disk→LibSql→Postgres never touches a domain crate.

## Security & authority role

Every crate in this family executes a responsibility `docs/reborn/contracts/kernel-boundary.md:20` lists as kernel-mediated ("filesystem mount/path authority, network policy+egress, secrets lease/one-shot injection") without deciding, itself, who may invoke it:

- `filesystem` enforces path containment once handed a `MountView`.
- `secrets` enforces one-shot consumption once handed a lease.
- `network` enforces policy once handed a `NetworkPolicy`.
- `safety` enforces detection/redaction rules that are data, not authority.

The family's sharpest current gap is the secrets direct-consumer set — `auth`, `webui`, and `operator` all hold a normal dependency on `ironclaw_secrets` today (confirmed this session), bypassing the kernel mediation path that every other upper-tier crate uses. PROPOSAL narrows that set to one (§6.2.2).

## Crate specifications

### `ironclaw_filesystem`

- **Path & disposition:** `crates/substrate/ironclaw_filesystem` — retain (PROPOSAL §9 row 7; §6.2.1).
- **Purpose:** the universal storage-dispatch fabric — `RootFilesystem`, `ScopedFilesystem`+`MountView` enforcement, the mount catalog, the CAS floor, and the disk/libSQL/Postgres/in-memory backends.
- **Target contents:** grounded in the CURRENT tree (14,840 lines, 20 files, measured this session). Stays:
  - `root.rs` (`RootFilesystem` trait, ~24 methods per PROPOSAL — a wide async trait with defaulted convenience methods over a smaller required core, confirmed live).
  - `scoped.rs`+`scoped/tests.rs` (`ScopedFilesystem`, `MountViewResolver`).
  - `catalog.rs` (`CompositeRootFilesystem`, `MountDescriptor`, `PathPlacement`).
  - `cas.rs`+`cas/tests.rs` (`cas_update`; `FILESYSTEM_CAS_RETRIES = 32`, `FILESYSTEM_APPLY_TIMEOUT = 15s`, `FILESYSTEM_CAS_BACKOFF_BASE = 2ms`/`_MAX = 50ms` — all confirmed live at `cas.rs:95-97`).
  - `backend.rs` (`EventRecord`, `StorageTxn`); `record.rs` (`CasExpectation`/`ContentType`/`Entry`/`RecordKind`/`RecordVersion`/`SeqNo`/`VersionedEntry`); `index.rs` (`Filter`/`IndexKey`/`IndexKind`/`IndexName`/`IndexSpec`/`IndexValue`/`Page`); `types.rs` (`BackendCapabilities`/`BackendId`/`BackendKind`/`Capability`/`ContentKind`/`DirEntry`/`FileStat`/`FileType`/`FilesystemError`/`FilesystemOperation`/`IndexConflictReason`/`IndexPolicy`/`StorageClass`/`TxnCapability`).
  - `in_memory.rs` (`InMemoryBackend`), `local.rs` (`DiskFilesystem`), `libsql.rs`+`libsql_pool.rs` (`LibSqlRootFilesystem`), `postgres.rs` (`PostgresRootFilesystem`), `vector.rs`.
- **Migration delta:**
  - `db.rs` (the transitional legacy-bytes-plane module) stays present with its removal still only *scheduled*, not executed by this move — PROPOSAL is explicit this is a standing item, not resolved here.
  - `hsm.rs` (`HsmBackend`) is gated or deleted per §2.6; its own doc comment already self-disqualifies it from production: "The placeholder stores ciphertext in process memory so the trait can be exercised end-to-end in tests. It is not a security boundary" (`hsm.rs:28-29`, confirmed live).
  - `fault.rs` (`FaultInjecting` decorator) stays behind the `test-support` feature, unchanged — confirmed already gated via `#[cfg(feature = "test-support")]` on both the module and its re-export (`lib.rs:22-24,43-44`).
- **Owns:** the trait, backends, CAS routing, and record/index vocabulary above.
- **Must never contain:** domain DTOs or policy; TLS policy (stays with `event_store`/composition); backend-*selection* decisions (composition's job, not this crate's); demo backends running as if they were production-safe.
- **Allowed internal deps:** `host_api`, `observability`, `safety` (single documented predicate `is_sensitive_path`, confirmed at `local.rs:8` import / `local.rs:490` call site).
- **Forbidden:** everything above substrates.
- **Public contracts & ports:** `RootFilesystem` (wide async trait; PROPOSAL flags this width as a named risk at §12), `ScopedFilesystem`, `MountDescriptor`, `cas_update`, `Entry`/`CasExpectation`, `IndexSpec`. Six in-crate implementations (Disk/LibSql/Postgres/InMemory/Hsm-placeholder/Composite) plus the test-only `FaultInjecting` decorator, plus four out-of-crate adapters (`host_runtime`'s `MountScopedRootFilesystem`, `skills`'s `SkillManagementRootFilesystem`, two in `memory_native`) — the dependency-inversion target named in the family's Dependency direction section.
- **Security & authority role:** **security/authority** (path containment and mount authority are kernel-listed responsibilities executed here) and runtime/artifact isolation (the DB-driver cone that keeps `libsql`/`deadpool-postgres`/`tokio-postgres` out of the other ~32 crates that would otherwise need it directly).
- **Why a crate (not a module):** criteria 1, 2, 4 — one contract, 33 verified normal-dependency consumers this session (`grep -rl "ironclaw_filesystem = " crates/*/Cargo.toml`), multiple production backends, and driver isolation a module could never provide (a module shares its whole crate's dependency graph; a 30-consumer fan-in makes that unacceptable here).
- **Enforcement:**
  - `filesystem_contract.rs`, `catalog_contract.rs`, `db_root_filesystem_contract.rs`, `concurrent_cas_storm.rs`, `postgres_delete_if_version_race.rs` (all confirmed live under `crates/ironclaw_filesystem/tests/`).
  - The `storage-placement.md` hybrid rule plus `reborn_virtual_roots_match_storage_placement_contract` (pins `VIRTUAL_ROOTS` to `host_api::path`).
  - NEW persistence-idiom rule (§11.2.6) restricting `libsql`/`deadpool-postgres`/`tokio-postgres` dependents to this crate and `event_store`, shrink-only allowlist seeded `{triggers, hooks}`.
- **Open questions (§12.10):** whether to hoist `sensitive_paths` out of `ironclaw_safety` given the documented `filesystem→safety` edge is exactly one function call (`is_sensitive_path`) — PROPOSAL records the option without deciding it (§6.2.1: "with the §12.10 option of hoisting `sensitive_paths` noted").

### `ironclaw_secrets`

- **Path & disposition:** `crates/substrate/ironclaw_secrets` — retain, narrow (PROPOSAL §9 row 8; §6.2.2).
- **Purpose:** scoped encrypted secret metadata/storage, one-shot leases, and the credential broker.
- **Target contents:** grounded in the CURRENT tree (7,290 lines, 7 files, fully reconciled this session: `lib.rs` 1,707 + `crypto.rs` 378 + `keychain.rs` 630 + `legacy_store.rs` 77 + `secret_store.rs` 3,086 + `placeholder.rs` 614 + `placeholder/tests.rs` 798). Stays:
  - `lib.rs` — core types `SecretMaterial`/`SecretMetadata`/`SecretLeaseId`/`SecretLeaseStatus`/`SecretLease`/`SecretStoreError`, plus the three public traits `CredentialAccountStore` (`lib.rs:538`), `CredentialSessionStore` (`lib.rs:557`), `SecretStorePort` (`lib.rs:1099`).
  - `crypto.rs` (`SecretsCrypto`, AAD helpers, master-key validation).
  - `keychain.rs` (OS keychain integration — `security-framework` on macOS, `secret-service` on Linux, both confirmed as target-gated deps in `Cargo.toml`).
  - `secret_store.rs` — `SecretStore<F>` (the generic struct implementing `SecretStorePort` at `secret_store.rs:416`, with `lease_once`/`consume` at `secret_store.rs:519`/`548`) and `CredentialBroker<F>` (`secret_store.rs:748`).
- **Migration delta:**
  - `placeholder.rs`+`placeholder/tests.rs` (1,412 lines total — `CredentialPlaceholderRegistry`/`CredentialSessionLease`, its own doc comment: "The egress proxy (W6-EGRESS-PROXY, not built yet) swaps the placeholder for a live `CredentialSession`... at request time," `placeholder.rs:6-7`) is deleted-until-built per §2.6 — an unwired subsystem, not a live one.
  - `legacy_store.rs` (77 ln, `SecretError`) is a compatibility shim; its disposition is folded into the crate's ordinary narrowing, not called out separately.
- **Owns:** `SecretStorePort` (`lease_once`/`consume` CAS one-shot), `SecretStore<F>`, `CredentialBroker<F>`, crypto/AAD, OS keychain master-key integration.
- **Must never contain:** runtime injection (staging/handoff is `host_runtime`'s obligations job, not this crate's), provider HTTP, product/vendor flows, the unwired placeholder egress-proxy subsystem (deleted per above).
- **Allowed internal deps:** `filesystem`, `host_api`.
- **Forbidden:** everything above substrates.
- **Public contracts & ports:** `SecretStorePort`, `CredentialAccountStore`, `CredentialSessionStore`, the full lease vocabulary above.
- **Direct-consumer rule (tightened — a deliberate contract change, PROPOSAL §3 note + §6.2.2):**
  - CURRENT Cargo-level normal-dependency consumers of `ironclaw_secrets`, confirmed this session: `auth`, `extension_host`, `host_runtime`, `operator`, `reborn_cli`, `reborn_composition`, `webui` (`ironclaw_safety`'s edge is dev-only — a documented pin between its leak detector and `CREDENTIAL_PLACEHOLDER_PREFIX`, not a real dependency).
  - Of these, PROPOSAL names three as reaching the store "directly beside the kernel path" and states the target explicitly: **keep** `auth` ("the engine is the documented owner of token custody flows") and **remove** the `webui` and `operator` direct edges — their secret needs route through `product_contracts` ports implemented by composition-wired services instead.
  - `host_runtime`, `extension_host`, `reborn_cli`, and `reborn_composition` are not named as changing (host_runtime is the mediator; the CLI/composition pair sees everything by the app-tier's own "any" rule, §8.2).
- **Security & authority role:** **security/authority** — secret custody; the invariant that "the raw value appears only at one-shot consumption" is this crate's entire reason to exist.
- **Why a crate (not a module):** criteria 1, 2 — a custody contract that must keep crypto/keychain dependencies out of every other crate; the direct-consumer tightening above is exactly the kind of boundary a module (which shares its host crate's full access) could never enforce.
- **Enforcement:**
  - `secret_store_contract.rs`, `boundary_contract.rs` (both confirmed live under `crates/ironclaw_secrets/tests/`).
  - NEW boundary rule (§11.2, direct-consumer rule) restricting normal deps on this crate to `auth` + kernel/app-tier crates once `webui`/`operator` are re-pointed.
  - The leak-detector cross-pin test (`ironclaw_safety`'s `sandbox_credential_placeholder_prefix_matches_registry`, documented in `ironclaw_safety/Cargo.toml`'s dev-dependency comment) keeps the two crates' credential-placeholder vocabulary from silently drifting apart.

### `ironclaw_network`

- **Path & disposition:** `crates/substrate/ironclaw_network` — retain, narrow (PROPOSAL §9 row 9; §6.2.3).
- **Purpose:** the network policy boundary and hardened outbound transport — target/method policy, DNS/private-IP enforcement, redirect/limit hardening.
- **Target contents:** grounded in the CURRENT tree (1,937 lines, 9 files, measured this session: `lib.rs` 35, `resolver.rs` 67, `error.rs` 104, `url_target.rs` 131, `policy.rs` 211, `types.rs` 218, `egress.rs` 227, `transport.rs` 463, `test_rewrite.rs` 481). Stays:
  - `policy.rs` (`StaticNetworkPolicyEnforcer`, `target_matches_pattern`).
  - `url_target.rs` (URL hardening — `NetworkTargetUrlError`, credential-in-path detection).
  - `egress.rs` (`NetworkHttpEgress`/`NetworkHttpTransport`/`PolicyNetworkHttpEgress`).
  - `resolver.rs` (`NetworkResolver` — DNS + private/reserved-IP denial).
  - `transport.rs` (`ReqwestNetworkTransport`).
  - `types.rs` (`NetworkHttpRequest`/`NetworkHttpResponse`/`NetworkRequest`/`NetworkTransportRequest`/`NetworkUsage`, `DEFAULT_RESPONSE_BODY_LIMIT`); `error.rs` (`NetworkHttpError`).
- **Migration delta:** `test_rewrite.rs` — 481 lines, **25% of the crate's current mass** — moves behind the `test-support` feature. It is production-compiled today, and its `default_policy_http_egress()` constructor is called from production code at `crates/ironclaw_reborn_composition/src/factory.rs:833` (confirmed live this session) despite the `test_rewrite` module name — the exact "composition's use of a test-support-named constructor as its production egress path" PROPOSAL flags for correction.
- **Owns:** `StaticNetworkPolicyEnforcer`, URL hardening, `NetworkHttpEgress`/`NetworkHttpTransport`/`NetworkResolver`+`ReqwestNetworkTransport`.
- **Must never contain:** credential injection (that is `host_runtime`'s obligations job), lane behavior, vendor allowlists (those are manifest data, not code here).
- **Allowed internal deps:** `host_api`.
- **Forbidden:** everything above substrates.
- **Public contracts & ports:** `NetworkHttpEgress`, `NetworkHttpTransport`, `NetworkResolver`. One production implementation each — `PolicyNetworkHttpEgress` and `ReqwestNetworkTransport`.
- **Security & authority role:** **security/authority** — sole owner of egress policy; keeps `reqwest`/TLS out of the kernel's dependents except through this one seam. CURRENT Cargo-level normal-dependency consumers, confirmed this session: `extension_host`, `host_runtime`, `reborn_composition` — consistent with PROPOSAL's characterization that only `host_runtime` calls it in production while composition constructs it (a Cargo-level dependency on the crate's *types* does not imply the dependent invokes its transport function at runtime).
- **Why a crate (not a module):** criteria 1, 2 — sole egress-policy owner; a module here would put `reqwest`+TLS in the build graph of every crate that needed even the policy types.
- **Enforcement:** `boundary_contract.rs`, `network_policy_contract.rs`, `network_http_egress_contract.rs` (all confirmed live under `crates/ironclaw_network/tests/`); note the root `CLAUDE.md:155` claim of a `NetworkPolicyDecider` trait is stale — no such trait exists anywhere in this crate (confirmed via full-crate grep this session) — a guidance fix tracked at §11.5, not a code change.

### `ironclaw_safety`

- **Path & disposition:** `crates/substrate/ironclaw_safety` — retain (PROPOSAL §9 row 10; §6.2.4).
- **Purpose:** dependency-light prompt-injection detection, validation, leak scanning, credential detection, and display redaction.
- **Target contents:** grounded in the CURRENT tree (7,226 lines, 11 files, fully reconciled this session: `credential_detect.rs` 651 + `display_redaction.rs` 862 + `leak_detector.rs` 2,053 + `lib.rs` 726 + `policy.rs` 535 + `prompt_validation.rs` 120 + `provider_validation.rs` 324 + `redaction.rs` 149 + `sanitizer.rs` 726 + `sensitive_paths.rs` 304 + `validator.rs` 776). Every module is a private `mod` with module-qualified re-exports at `lib.rs` (not a wildcard prelude — already the disciplined pattern PROPOSAL wants `host_api` to adopt). Stays:
  - `SafetyLayer` (`lib.rs:91`, unifies `sanitizer`+`validator`+`policy`+`leak_detector`).
  - `Sanitizer` (`sanitizer.rs`, `InjectionScanner` impl); `Validator` (`validator.rs`).
  - `LeakDetector` (`leak_detector.rs`, `LeakScanner` impl, the largest single file in the crate at 2,053 ln).
  - `credential_detect.rs`; `sensitive_paths.rs` (`pub mod`, the one module `filesystem` reaches into).
  - `display_redaction.rs`; `policy.rs` (`Policy`/`PolicyRule`/`Severity`); `redaction.rs`; `prompt_validation.rs`; `provider_validation.rs`.
  - The fuzz harness (`ironclaw_safety/fuzz/`, 5 targets confirmed: `fuzz_safety_sanitizer`, `fuzz_config_env`, `fuzz_safety_validator`, `fuzz_credential_detect`, `fuzz_leak_detector`) stays as an excluded-tooling sibling package.
- **Migration delta:** none structural — this is one of the family's purely-retained crates. The internal duplicate-pipeline finding (two independent wrapping/redaction pipelines: `safety::redaction`+`safety::display_redaction` vs `host_api::credential_redaction` vs `prompt_envelope::wrap_untrusted` vs `safety::wrap_external_content` at `lib.rs:287`) is named as cleanup work but explicitly not resolved by this move (§12.10).
- **Owns:** `SafetyLayer`, `Sanitizer`, `Validator`, `LeakDetector`, `credential_detect`, `sensitive_paths`, display redaction.
- **Must never contain:** sandbox execution, credential storage, network policy, dispatch — this crate detects and redacts, it never enforces containment or holds material itself.
- **Allowed internal deps:** none internal at the normal-dependency tier (the `ironclaw_secrets` edge visible in `Cargo.toml` is a documented dev-dependency only, pinning `leak_detector.rs`'s `sandbox_credential_placeholder_prefix_matches_registry` test against `CREDENTIAL_PLACEHOLDER_PREFIX` so the two crates' credential-placeholder vocabulary cannot silently drift).
- **Forbidden:** all internal at the normal tier.
- **Public contracts & ports:** `SafetyLayer`, `Sanitizer`/`InjectionScanner`, `Validator`, `LeakDetector`/`LeakScanner`, `Policy`, plus the free functions in `credential_detect`/`redaction`/`display_redaction`/`prompt_validation`/`provider_validation`/`sensitive_paths`.
- **Security & authority role:** security mechanism substrate — the crate that turns "does this text look like an attack / a leaked secret / a sensitive path" into a typed, testable answer that callers (kernel obligations, filesystem, memory, hooks) act on.
- **Why a crate (not a module):** criteria 1, 2 — 16 consumers (PROPOSAL §6.2.4) isolating the `aho-corasick`/`regex` cone from every crate that only needs one detection function.
- **Enforcement:** inline `#[cfg(test)]` suites throughout (no separate `tests/` directory — confirmed this session) plus the two Criterion benches (`safety_check`, `safety_pipeline`) plus the 5-target fuzz harness; the leak-detector/secrets cross-pin test named above.
- **Open questions (§12.10):** the internal duplicate-pipeline cleanup — two redaction families inside this crate, plus the overlap with `prompt_envelope` and `host_api::credential_redaction` — is explicitly named as unresolved cleanup work, not decided here (§6.2.4: "Internal duplicate-pipeline cleanup... is §12.10"); it is the same open question `prompt_envelope`'s entry in `families/contracts.md` records from the other side.

### `ironclaw_observability`

- **Path & disposition:** `crates/substrate/ironclaw_observability` — retain as-is (PROPOSAL §9 row 11; §6.2.5).
- **Purpose:** zero-cost-when-off latency-trace macros over the `ironclaw_latency` tracing target.
- **Target contents:** unchanged single-file crate (123 lines, `lib.rs` only, confirmed live):
  - Three macros — `live_latency_trace!`, `live_latency_trace_ok!`, `live_latency_trace_error!` (all `#[macro_export]`).
  - Their helpers `elapsed_ms`, `live_latency_enabled` (gated on `tracing::enabled!(target: "ironclaw_latency", ...)`), `live_latency_started_at`.
  - `pub use tracing`, documented in PROPOSAL as "a deliberate macro-hygiene tradeoff" (the macros expand to `$crate::tracing::trace!`, so consumers never need their own `tracing` import to use them).
- **Migration delta:** `json_value_bytes` (`lib.rs:28-33`, confirmed live — a `serde_json::Value`-byte-counting helper unrelated to latency tracing) moves to its caller as gravity-well hygiene; it is the one function in the crate that doesn't belong to the tracing-macro charter. A guidance file is added — none exists today (no `.md` file in the crate directory, confirmed this session).
- **Owns:** the three macros + their helpers + the `tracing` re-export.
- **Must never contain:** state, policy, sinks, or (after the move above) off-topic byte-counting utilities.
- **Allowed internal deps:** none. **Forbidden:** all internal.
- **Public contracts & ports:** the three macros; no traits.
- **Security & authority role:** none — this is the family's only crate with zero security-relevant surface; its only "authority" is deciding whether a trace fires, which is an observability toggle, not a privilege.
- **Why a crate (not a module):** criterion 2 — leaf macro surface with 7 confirmed normal-dependency consumers this session (`first_party_extensions`, `filesystem`, `host_runtime`, `loop_host`, `reborn_composition`, `runner`, `turns`); folding it into any one consumer would force the other six to depend on that consumer just for a tracing macro.
- **Enforcement:** two inline unit tests (`json_value_bytes_matches_serialized_value_length`, `json_byte_counter_saturates_on_write`, both confirmed live) exercise the byte-counter being relocated; no architecture-test coverage exists for this crate today, and none is proposed beyond the family-wide layer/dependency checks — its surface is too small to need more.

## Family AGENTS.md obligations

Per PROPOSAL §6.2's family intro and §11.4, `crates/substrate/AGENTS.md` must state, verbatim or near-verbatim:

- Each crate's mediation story — who may call it directly (kernel services, `app/ironclaw_composition`) and which callers must instead go through host mediation — since this is the one thing every crate in the family answers differently and the direct-consumer tightening (`secrets`) shows what happens when it drifts.
- The "mechanism, not authority" line: a substrate crate enforces its own local invariant (containment, one-shot consumption, policy match) but never decides whether the caller was entitled to invoke it — that decision always comes from `kernel/` before the call reaches here.
- The driver/OS/regex-cone isolation rationale per crate, so a reviewer sees *why* five crates exist instead of one `ironclaw_substrate` — each crate's `Cargo.toml` dependency list is the enforcement mechanism, this file is the explanation.
- The two charted substrate-on-substrate exceptions (`filesystem→safety` for `is_sensitive_path`; `secrets→filesystem`) as the *only* sanctioned internal edges within the family, with a pointer to the open question (§12.10) about hoisting `sensitive_paths`.
- The persistence-idiom pointer: outside this family's `filesystem` and the `events/` family's `event_store`, no crate may depend on `libsql`/`deadpool-postgres`/`tokio-postgres` (§11.2.6) — this file is where a crate owner discovers that rule applies to them.
- A pointer to each crate's own guidance file (`filesystem/CLAUDE.md`, `secrets/CLAUDE.md`, `network/CLAUDE.md` exist today; `safety/AGENTS.md` exists; `observability/AGENTS.md` does not and must be created as part of its move, per §11.4's explicit "six guidance-less crates" list).

## Current → target summary

| Target crate | Current name/location | Disposition |
|---|---|---|
| `substrate/ironclaw_filesystem` | `crates/ironclaw_filesystem` | retain + move (`HsmBackend` gated/deleted; `db.rs` removal stays scheduled; §6.2.1) |
| `substrate/ironclaw_secrets` | `crates/ironclaw_secrets` | retain-narrow + move (placeholder subsystem deleted-until-built; `webui`/`operator` direct edges removed; §6.2.2) |
| `substrate/ironclaw_network` | `crates/ironclaw_network` | retain-narrow + move (`test_rewrite.rs` behind `test-support`; §6.2.3) |
| `substrate/ironclaw_safety` | `crates/ironclaw_safety` | retain + move (redaction-family unification tracked, not resolved; §6.2.4) |
| `substrate/ironclaw_observability` | `crates/ironclaw_observability` | retain + move (`json_value_bytes` evicted; guidance file added; §6.2.5) |
