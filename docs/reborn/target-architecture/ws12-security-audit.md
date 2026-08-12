# WS12 rows 5–6 — extension-journey re-verification and the §12.1 security spot-audit

**Date:** 2026-08-05
**Tree:** `0c6c0cfb9d853d941df78f48b273a1eba47a52ec` (the `program-closure` batch tree: `origin/main` `b2023bc8f` + the Round-1 folds — the #7154 defect train with the D-R loopback carve-out, the D-S await-edge ruling, and the WS12 mapping audit)
**Reviewer:** the `closure/ws12-security` agent, acting as the **second reviewer** CHECKLIST WS12 row 6 requires. The first reviewer is the record already in PROPOSAL §12.1a/b/c and §12.13 D-R; this document is the independent adversarial pass over it.
**Method:** every claim below was re-derived on this tree with a command that was actually run. Attacks were *executed*, not reasoned about: two sabotage files and one hostile-shape probe were planted, run, and reverted (`git status` clean before commit). Nothing outside this file and CHECKLIST lines 631–632 was modified.

## Verdicts

| # | Seam | Verdict |
|---|------|---------|
| 1 | Evidence-mint consolidation (§12.1a) | **HOLDS-WITH-RESIDUAL** — production seal measured intact; **one new residual recorded here** (F1) |
| 2 | Secrets direct-consumer tightening (§12.1b) | **HOLDS-WITH-RESIDUAL** — no value-reach bypass found; the recorded residue inventory undercounts by one crate (F2) |
| 3 | Host/verifier colocation (§12.1c) | **HOLDS** — no spoof found; fail-closed on every branch attacked |
| 4 | D-R literal-loopback carve-out (§12.13 D-R) | **HOLDS** — predicate is literal-only and every accepted shape is genuinely loopback; two side-channels closed by defense in depth |
| — | Row 5 extension journeys | **4 of 5 legs verified end-to-end**; leg 3 (gsuite + credential injection) is a **coverage hole** (F3) |

**No HOLE was found in any of the three §12.1 seams or in D-R.** The findings below are residuals and a coverage gap, each recorded rather than fixed (this audit's mandate is report-not-repair).

---

## Row 5 — extension journeys, re-verified

Every suite below was run on this tree. Postgres-parameterised cases could not run here: this machine has no Docker daemon, and the harness deliberately *fails* rather than skips (`"a Postgres skip is a failure per REL-3"`). That is an environment limitation of this audit host, not a defect — the Postgres lane belongs to CI and to WS12 row 4 (backend parity).

| Leg | Covering test | Command / result |
|---|---|---|
| **1. Slack inbound → turn → delivery** | `tests/integration/extension_delivery.rs::slack_final_reply_flows_through_the_real_delivery_coordinator` — real bundled Slack package installed through the production lifecycle tool; genuinely HMAC-signed `app_mention` POST through the production `extension_ingress_route_mount`; verified, normalized by the real `SlackChannelAdapter`, durably admitted, **real turn** on the real coordinator, reply durable in-thread, outbound `Delivered` attempt, `chat.postMessage` on the recorded wire with a host-injected bearer | `cargo test -p ironclaw_integration_tests --test reborn_integration_extension_delivery` → **19 passed** (`case_1_libsql` green; `case_2_postgres` env-blocked) |
| **1b. Slack verification + delivery segments** | `crates/extensions/ironclaw_extension_host/src/channel_host/e2e_tests.rs` — ~28 tests incl. `slack_events_rejects_forged_hmac_signature`, `slash_form_with_forged_signature_is_rejected`, `shared_channel_admission_follows_saved_channel_config` (unrouted shared channel fails closed: 0 turns; ✎ 2026-08-08: since renamed `shared_channel_message_is_served_by_presence` when the saved-config admission model was retired for presence-based admission) | covered by the crate lib run; real ingress + real adapter, `RecordingTurnCoordinator` substitutes for the turn engine (segment evidence, stated as such) |
| **2. Telegram inbound → turn → delivery** | `tests/integration/extension_delivery.rs::telegram_update_becomes_a_turn_and_a_coordinated_reply` — the strongest of the three: ingress registered by the **production** channel-host assembly (`VendorIngress::production`), wrong `X-Telegram-Bot-Api-Secret-Token` → 401 with no delivery, correct secret → 200, real turn, `sendMessage` + `deleteMessage` on the wire, attachment sub-journey included | same run → green on libsql |
| **3. gsuite tool call + credential injection** | **Split across two halves that no committed test joins.** Half A (real gsuite handler → staged credential): `crates/app/ironclaw_composition/tests/gsuite.rs::bundled_gsuite_handlers_stage_selected_account_secret_before_egress` and `crates/extensions/ironclaw_extension_support/tests/gsuite_core.rs::gsuite_handler_uses_selected_credential_handle_for_runtime_egress`. Half B (staged obligation → chokepoint → token on the wire): only against **GitHub** (`tests/integration/secret_injection.rs::injects_credential_onto_github_egress`) and **Slack** (`tests/integration/extension_runtime.rs`) | `cargo test -p ironclaw_composition --test gsuite` → **11 passed**; `cargo test -p ironclaw_extension_support --test gsuite_core` → **46 passed**; `cargo test -p ironclaw_integration_tests --test reborn_integration_secret_injection` → **14 passed**. **Both halves green; the join is missing — see F3.** |
| **4. Pairing (generic WebGeneratedCode seam)** | `tests/integration/extension_delivery.rs::unbound_telegram_actor_pairs_via_web_minted_code_then_turns_attribute_to_the_paired_user` — unbound verified DM fails closed to a connect nudge (no turn, no operator inheritance); code minted by the production pairing service; verified webhook `/start@bot <code>` serviced by the pre-admission interceptor; durable binding; the **same** actor's next DM admits a real turn whose scope subject *is* the paired user; real protected `pairing/status` + `pairing/unpair` routes revoke | same run → green. Seam unit coverage: `cargo test -p ironclaw_extension_host --lib channel_pairing` → **16 passed** (incl. `concurrent_caller_admission_has_exactly_one_pairing_winner`, `caller_admission_isolates_foreign_installations_and_wrong_users`, `connection_probe_fails_closed_and_sanitizes_the_backend_error`) |
| **5. Lifecycle install / config / activate / remove** | Joint sequences: `tests/integration/group_extensions/scenario_credential_extension_lifecycle_state_machine.rs` and `scenario_slack_channel_lifecycle_state_machine.rs` (install→configure→connect→use→remove→reconfigure→reconnect→use, with lifecycle phase and tool dispatchability asserted to flip together). Activation: `crates/extensions/ironclaw_extension_host/tests/lifecycle_contract.rs` | `cargo test -p ironclaw_integration_tests --test reborn_group_extensions` → **15 passed**; `cargo test -p ironclaw_extension_host --features test-support --test lifecycle_contract` → **10 passed** |
| **5-FC. Lifecycle fail-closed** | Config: `channel_config.rs::save_rejects_unknown_field_handles_and_stores_nothing` (no partial write), `::effective_config_fails_closed_when_admin_configuration_is_unavailable`. Activate: `extension_lifecycle_capabilities.rs::standalone_extension_activate_returns_auth_gate_for_missing_extension_credentials`, `::…_when_account_lacks_required_scope`, `::…_maps_corrupt_configured_account_to_backend`. Install: `extension_v2_lifecycle_fails_closed_before_install_for_unknown_required_host_port`. Activation refusals: `declared_tool_without_bound_adapter_fails_activation`, `duplicate_capability_across_extensions_fails_activation`, `channel_activate_runs_and_its_failure_aborts` | `cargo test -p ironclaw_extension_host --lib channel_config` → **10 passed**; `cargo test -p ironclaw_extension_manager --lib extension_lifecycle_capabilities` → **25 passed**; activation refusals inside the `lifecycle_contract` run above |

**Row 5 verdict: legs 1, 2, 4 and 5 are verified end-to-end on committed suites; leg 3 is verified in two halves that are never joined (F3).** The owner's fail-closed principle ("misconfig → guard/reject, never test-degraded") is directly pinned, most sharply by `every_store_failure_surfaces_as_backend_rather_than_a_silent_success` and `unknown_duplicate_missing_and_oversized_values_fail_closed`.

---

## Row 6 — the §12.1 spot-audit

### Seam 1 — evidence-mint consolidation (§12.1a) — **HOLDS-WITH-RESIDUAL**

**Attack 1: forge verified evidence over the wire (serde back-door).** Refuted by construction. `ProtocolAuthEvidence` hand-writes `Deserialize` (`crates/contracts/ironclaw_host_api/src/product_adapter/auth.rs:281-333`) and rejects any envelope whose `kind` is not `"failed"`, with the message *"only `failed` may cross trust boundaries"*. The `Verified` variant's payload carries a `HostAuthSeal` whose constructor is module-private, and the enum `ProtocolAuthEvidenceKind` is itself private, so no downstream crate can replay a seal from one value into another. No `Default`, no public tuple constructor.

**Attack 2: obtain a grant while evading the census — the two evasions §11.2.5 records as open.** I planted a single production file combining **both** named shapes at once — an import alias *and* a multiline `impl` header — for **both** grant traits:

```rust
use ironclaw_host_api::product_adapter::auth::ChannelIngressVerifier as V;
struct Rogue;
impl
    V
    for
    Rogue
{
}
```

**Caught, with exact file:line, for both traits:**

```
`ChannelIngressVerifier` may be implemented ONLY in `ironclaw_extension_host` …
Offenders: ["crates/domains/ironclaw_trace_commons/src/zz_sabotage_delete_me.rs:9: impl V for Rogue"]
`HostProtocolAuthenticator` may be implemented ONLY in `ironclaw_webui` …
Offenders: ["crates/domains/ironclaw_trace_commons/src/zz_sabotage_delete_me.rs:16: impl H for Rogue"]
```

**Both recorded evasions are CLOSED on this tree** — see "Known-evasions re-verification" below.

**Attack 3: call a mint function from a crate that owns none.** A production file in `ironclaw_trace_commons` calling `mark_request_signature_verified` was caught by `mint_functions_are_named_only_by_their_owners_and_sanctioned_minters` on **both** the `use` line and the call site.

**Attack 4 (the one that found something): the `test-support` feature as a privilege boundary.** §12.1a's own generalizable finding is *"treat 'a cargo feature gates this' as an unproven claim until measured; a feature that any sibling manifest can unify on is not a privilege boundary."* That finding applies verbatim to the feature that survived: `ProtocolAuthEvidence::test_verified` / `::test_verified_for_tenant` are gated by `#[cfg(any(test, feature = "test-support"))]` (`auth.rs:395-407`) and require **no grant at all**. The seal's own failure message points readers at them.

Measured, in both directions:

- **The shipped artifact is safe.** `cargo tree -p ironclaw -e normal -f '{p}|{f}'` reports `ironclaw_host_api v0.1.0 (…)|` — **empty feature set**. `test_verified` does not exist in the production binary, so a production-source call fails to compile in the release build. Every enablement of the feature is under `[dev-dependencies]` (root workspace package line 115, `ironclaw_wasm` line 28, `ironclaw_product_contracts` line 59).
- **The scan half does not cover it.** A production source file calling `ProtocolAuthEvidence::test_verified` was planted alongside attacks 2–3 and produced **no offender** — the constructors are absent from `CHANNEL_MINT_FNS`, `HOST_MINT_FNS` and `RETIRED_MINT_FNS`. In lanes where the feature unifies on (workspace `cargo test`, `cargo clippy --all-features`) such a call compiles green and nothing flags it.
- **Nothing pins the feature's placement.** There is no assertion anywhere that `test-support` appears only in `[dev-dependencies]` — the discipline is prose in manifest comments (*"Never enabled by a shipped artifact"*). Contrast the retired `host-auth-mint`, which has three dedicated tests refuting it across every manifest, every script and every workflow.

→ **F1 (new residual).** One manifest line moving `test-support` from `[dev-dependencies]` to `[dependencies]` would make an ungranted mint constructor reachable from production code workspace-wide, and **no test would fail**. This is not a live hole — the shipped binary is measured feature-free, and the property §12.1a exists for (*a package or a product handler cannot mint at all*) holds today. It is the same class as the `host-auth-mint` finding, one level over, and is recorded here because §12.1a's risk statement should not read as stronger than what shipped.

**Residual re-verification: `seal_verified_inbound`.** Still real and still accurately recorded. The signature at `auth.rs:377-387` still takes a full `AuthRequirement`, so a `VerifiedInboundGrant` holder can attest a bearer-shaped requirement. The holder is `VerifiedEvidenceMint` in `ironclaw_extension_host` — trusted host code by charter. §12.1a's framing is exact.

**Suites run:** `reborn_sealed_evidence_mint_ratchet` **19 passed** (the doc figures of 10 and 17 are both stale); `ironclaw_host_api --test protocol_auth_evidence_seal` **4 passed**; `ironclaw_extension_contracts --test verified_inbound_seal` **3 passed**.

### Seam 2 — secrets direct-consumer tightening (§12.1b) — **HOLDS-WITH-RESIDUAL**

**Attack: reach a raw secret value from a non-sanctioned crate.** Not achieved.

- **Type-level.** `SecretMaterial` is `pub use secrecy::SecretString as SecretMaterial` (`crates/substrates/ironclaw_secrets/src/lib.rs:41`). Its `Debug` renders redacted and it has no `Display`, so a `{:?}` into a log or an error payload cannot leak the value; reaching the bytes requires an explicit `ExposeSecret::expose_secret`. The egress cache additionally pins the carrier at compile time: `credential.rs:617-622` is a `const _: fn(&CredentialCacheEntry)` that type-checks the field against a `ZeroizeOnDrop` bound, so downgrading it to `Option<String>` stops the crate compiling rather than waiting for a test.
- **Error payloads.** `sanitized_secret_error` (`credential.rs:360-374`) maps all eight `SecretStoreError` variants to fixed strings, deliberately dropping the substrate's `Display` (including `BackendMisconfigured { reason }`, the one that carries backend detail). Verified exhaustively by `sanitized_secret_error_maps_every_variant_to_stable_reason`.
- **Consumer set.** Enumerated every manifest edge on `ironclaw_secrets` by section. `ironclaw_operator` has **no** edge in either section — only a comment recording its removal — so §12.1b's operator half is discharged as written, and its boundary rule is armed (`reborn_dependency_boundaries.rs:4147`, inside `ironclaw_operator`'s `forbidden` list). `ironclaw_webui` holds it under `[dev-dependencies]` only, as §12.1b corrects.
- **Exposure sites in the products/extensions tier** are all hash-or-length uses, never value egress: `admin_configuration_service.rs` uses `expose_secret()` for `.len()`, `.is_empty()`, and SHA hashing; `product_auth/oauth.rs` hashes PKCE verifiers and authorization codes.

→ **F2 (recorded-state correction).** §12.1b states that after the WS3 slice `ironclaw_extension_manager` *"is the only `products`-layer crate with the edge"*. Measured on this tree that is **false by one**: `ironclaw_assistant` (`layer = "products"`, `Cargo.toml:14`) carries `ironclaw_secrets` as a **normal** dependency (`Cargo.toml:54`, between `[dependencies]` at 27 and `[dev-dependencies]` at 71). Its single production use site is `src/admin_user_directory.rs:25`, which names `SecretMaterial`/`SecretMetadata`/`SecretStoreError` as **vocabulary in a port declaration** (`AdminSecretProvisioner`) — `list` returns metadata, `put` *accepts* material, `delete` returns a bool, and the crate calls `expose_secret` nowhere. So this is an inventory/doc-truth error, **not** a value-reach bypass, and it is the same class of finding §12.1b is itself made of ("the residue is a crate this clause could not have named"). It should join #7095's inventory.

### Seam 3 — host/verifier colocation (§12.1c) — **HOLDS**

**Attack: get an inbound admitted as verified without the colocated verifier running.** Every route I could construct fails closed.

- **Only one production caller mints.** `VerifiedEvidenceMint::mint` is a private fn whose sole call site is `GenericChannelInboundSink::admit` (`extension_ingress.rs:500`). Workspace-wide, `InboundSink::admit` has exactly **one** production caller — the registry forwarder at `extension_ingress.rs:327`. (A second apparent caller, `crates/app/ironclaw_cli/src/runtime/native_extensions.rs:202`, is inside a `#[tokio::test]`; I checked it specifically because an app-tier direct `admit` would have been a bypass.)
- **The router verifies unconditionally, first.** `IngressRouter::verify_and_dispatch` (`ingress/router.rs:349-396`) runs `verify_recipe` **before** the adapter and before admission: secrets unavailable → `503`, verification failed → `401`. Candidates are `drop`ped before any adapter work, and the headers the recipe consumed are **stripped from what the adapter sees** (`:423-428`) — so an adapter cannot even observe, let alone re-assert, a trust marker.
- **`verify_recipe` has no fail-open branch.** Empty candidate list → `NoCandidates`; more than `MAX_VERIFICATION_CANDIDATES` → `TooManyCandidates`; more than one match → `Ambiguous`; `resolve_exactly_one` demands exactly one (`ingress/verifier.rs:114-167`).
- **The sharpest attack — a manifest declaring no verification.** `IngressVerificationRecipe::None` exists, and `VerifiedEvidenceMint` has no matching "unverified" variant, so I checked what the gap does. `evidence_mint_for_verification(&None)` returns `None` (`channel_host.rs:99`), and the graph builder then **refuses to build the registration at all** (`channel_host.rs:570-572`, `return Ok(None)`) — no mint, no route, nothing to spoof. Pinned by `channel_host.rs:1081`.
- **Client-supplied trust markers** have no reception point: evidence is derived from the recipe the router just executed, never from request content.

**Suites run:** `ironclaw_extension_host --features test-support --test ingress_router_contract` **20 passed**; the Slack/Telegram forged-signature rejections listed in row 5.

### Seam 4 — the D-R literal-loopback carve-out (§12.13 D-R, this batch's change) — **HOLDS**

**Is the predicate literal-only?** Yes. `is_loopback_host` (`crates/domains/ironclaw_trace_commons/src/onboarding/invite.rs:165-171`) is `bare == "localhost" || bare.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())`. No resolver, no DNS, no allocation of a hostname class.

**Hostile-shape probe.** I ran the production pairing — `url::Url::parse(...).host_str()` fed to `is_loopback_host`, exactly what `apply_credential_injection` does — over 38 hostile shapes. Selected results:

| Shape | `host_str()` | Accepted? | Assessment |
|---|---|---|---|
| `http://127.0.0.1.evil.com/` | `127.0.0.1.evil.com` | **no** | suffix attack refused |
| `http://127.0.0.1@evil.com/` | `evil.com` | **no** | userinfo trick refused — the parser yields the *real* host |
| `http://localhost%2eevil.com/` | `localhost.evil.com` | **no** | encoded-dot refused |
| `http://0.0.0.0/`, `http://[::]/` | `0.0.0.0`, `[::]` | **no** | wildcard is not loopback |
| `http://169.254.169.254/`, `http://10.0.0.5/`, `http://[fe80::1]/` | — | **no** | cloud-metadata / RFC1918 / link-local refused |
| `http://127.1/`, `http://0177.0.0.1/`, `http://2130706433/`, `http://0x7f000001/` | all → `127.0.0.1` | **yes** | **safe** — the `url` crate *canonicalises* these to `127.0.0.1` before the predicate sees them; every one genuinely denotes loopback |
| `http://127.0.0.2/`, `http://127.255.255.254/` | as written | **yes** | safe — 127.0.0.0/8 is loopback |
| `http://[::1]/`, `http://[0:0:0:0:0:0:0:1]/` | `[::1]` | **yes** | safe |
| `http://[::ffff:127.0.0.1]/` | `[::ffff:7f00:1]` | **no** | v4-mapped loopback is refused (Rust's `Ipv6Addr::is_loopback` is false for it) — **fails closed**, a usability nit only |
| `http://localhost./` (trailing dot) | `localhost.` | **no** | fails closed |
| `http://127.0.0.1%00.evil.com/`, `http://[::1]%2eevil.com/` | — | parse error | refused before the predicate |

**Every accepted shape is genuinely loopback.** The obfuscated integer/octal/hex forms are accepted *because the parser has already normalised them*, and — this is the load-bearing detail — the request that goes out is the **same parsed URL**: `apply_credential_injections` writes `request.url = url.to_string()` (`credential.rs:175-177`) after the guard has run, so there is no "guard checks the normalised URL, transport sends the raw one" differential. The sanitizer (`egress/sanitize.rs:81`) and the network target resolver (`ironclaw_network/src/url_target.rs:69`) both parse with the same `url` crate (2.5.8), so no parser differential exists anywhere along the chain.

**Two side-channels I probed, both closed by defense in depth downstream:**
- `evil.com@127.0.0.1` and `user:pw@127.0.0.1` *are* accepted by the predicate (correctly — the destination genuinely is loopback), and `ironclaw_network::url_target` rejects **all** userinfo outright (`NetworkTargetUrlError::UserinfoDenied`, `url_target.rs:70-72`).
- `ftp://127.0.0.1` passes the credential guard (the check is `scheme != "https" && !literal_loopback`, so any non-https scheme toward loopback is admitted — a widening relative to the pre-D-R absolute guard). Unreachable in production: the same resolver admits only `http`/`https` (`UnsupportedScheme`, `url_target.rs:73-77`).

**The negative regression drives the production chokepoint through the caller.** `host_http_egress_refuses_to_attach_a_credential_over_plaintext_http` (`crates/kernel/ironclaw_host_runtime/src/services/tests.rs:511-600`) calls `egress.execute(request)` on the **configured egress port** — not the `apply_credential_injection` helper — so it crosses policy authorization and the staged-obligation lookup. It loops over **all four** target shapes (`Header`, `QueryParam`, `PathPlaceholder`, `BodyJsonPointer`), gives each a well-formed request so the refusal is attributable to the scheme guard rather than a shape error, opens the *policy* allowlist to plaintext so the refusal under test is the credential guard, and asserts both the `HTTPS` reason **and** that the recording network stayed empty. The host is a deliberately non-loopback public name. Its sibling `host_http_egress_attaches_a_credential_over_literal_loopback_http` (`:611-662`) pins the exception with the credential actually injected. `cargo test -p ironclaw_host_runtime --lib host_http_egress` → **7 passed**.

**Redaction / no-credential-in-errors on the refusal path.** Holds. Every `RuntimeHttpEgressError::Credential { reason }` in `credential.rs` is a static literal — there is not one `format!` reason string in the file — so no value can reach an error payload. On refusal, `restore_staged_secrets` returns the material to the store and `PipelineError::pre_transport_keep_staged_secrets` deliberately does **not** discard staged secrets (`egress/pipeline.rs:257`, `egress/mod.rs:254-260`), so a legitimate retry still works. `redaction_values` is extended only *after* a successful injection (`credential.rs:173`), so the refused credential never enters the redaction set — correct, since it never entered the request either.

**One hardening nit, not a leak.** The credential is resolved into `SecretMaterial` and `expose_secret()` borrowed (`credential.rs:165`) *before* `apply_credential_injection` runs the URL guard. The house rule's binding sentence is *"Never inject credentials until the resolved destination passes those checks"* (`.claude/rules/safety-and-sandbox.md:46-47`) — satisfied, since injection is strictly after the guard. But the guard is a pure function of the URL and could run before any secret is resolved, saving a store round-trip on the refusal path. No exposure results either way: the value stays inside the zeroizing carrier and is restored.

**D-R verdict: HOLDS.** The carve-out is what it claims to be — literal-loopback only, one shared predicate, both perimeter sides test-frozen. D-R's own recorded residual (a generic chokepoint carrying a carve-out sized for one first-party consumer) is accurate and unchanged.

---

## Known-evasions re-verification

The mission asked whether the two recorded scan evasions and the `seal_verified_inbound` residual are still accurately recorded. Measured:

| Recorded item | Recorded where | State on this tree |
|---|---|---|
| Import-alias evasion of `assert_sole_implementor` | PROPOSAL §11.2.5, §12.1a; CHECKLIST:552, :597 | **CLOSED.** `implementor_names` resolves in-file `use … as …` aliases; sabotage-proven above. Self-tested by `alias_resolution_binds_only_the_renamed_trait`. |
| Multiline-`impl`-header evasion | same | **CLOSED.** `impl_headers` collapses headers onto one line; sabotage-proven above. Self-tested by `impl_header_extraction_cannot_silently_degrade`. |
| Fail-open `fs::read_to_string(..).unwrap_or_default()` reads | PROPOSAL §11.2.5 | **CLOSED.** `read_source` (`:159-167`) now panics with *"a source this census cannot read must fail the gate, not scan as empty"*; `collect_production_rs` panics on an unreadable directory. Pinned by `an_unreadable_source_fails_the_census` and `an_unreadable_directory_fails_the_walk`. |
| `collect_production_rs` skips fewer dirs than `collect_manifests` (no `node_modules`) | PROPOSAL §11.2.5 | **CLOSED.** `node_modules` is in the skip set (`:194-197`). |
| `ProtocolAuthEvidence::seal_verified_inbound` accepts a full `AuthRequirement` | PROPOSAL §12.1a, `auth.rs:370-376` | **STILL REAL, accurately recorded.** Unchanged signature; holder is trusted host code by charter. |

**Doc-truth consequence.** PROPOSAL §11.2.5 and §12.1a still describe the two evasions and the fail-open reads as live and *"owed to WS10"*, and CHECKLIST:552 carries them as an open guardrail row while CHECKLIST:597's tick says *"the scan implementing half of it has two named holes."* **On this tree they are fixed** — and fixed with the negative fixtures that row demanded, plus hardening those entries never asked for: the census now derives its scan roots from `[workspace] members` (so `tools/` is covered), resolves the owning crate through the crate inventory rather than the path prefix, and carries two fail-closed floors (`files.len() > 500`, `headers_seen > 1000`) so a normalizer that stopped parsing reds the build instead of reporting a clean census. **Those three recording sites now understate the tree's strength.** Not fixed here (this audit may not edit them) — flagged for the coordinator; see F4.

Secondary doc-truth note: the ratchet holds **19** `#[test]`s. §12.1a says 10, CHECKLIST:597 says 17, PROPOSAL §11.2 line 1102 already corrected 10 → 17. All three are now stale by the same drift.

---

## Findings register

| # | Finding | Class | Severity | Owner suggestion |
|---|---|---|---|---|
| **F1** | `ProtocolAuthEvidence::test_verified` / `::test_verified_for_tenant` are ungranted mint constructors governed only by the `test-support` cargo feature. They are absent from every mint-name table, so a production-source call is invisible to the scan half; and nothing anywhere pins `test-support` to `[dev-dependencies]`. The shipped binary is measured feature-free, so this is not live. | New residual on §12.1a | **Medium** (defense-in-depth; not exploitable in the shipped artifact) | Two cheap assertions in `reborn_sealed_evidence_mint_ratchet`, mirroring what already exists for the retired `host-auth-mint`: (a) add the two constructors to a governed name table so a production call site is an offender; (b) assert `test-support` appears in no `[dependencies]` table workspace-wide. |
| **F2** | §12.1b's residue statement ("the only `products`-layer crate with the edge") undercounts by one: `ironclaw_assistant` holds a normal `ironclaw_secrets` dependency for port vocabulary in `admin_user_directory.rs`. No value reach; no `expose_secret` in the crate. | Recorded-state correction on §12.1b | **Low** (inventory accuracy) | Add to #7095's inventory; decide port-vocabulary narrowing with the extension_manager case rather than separately. |
| **F3** | **Journey coverage hole (row 5, leg 3).** No committed test drives a model-callable google/gsuite tool through a real runtime and asserts the OAuth token landed on the outbound HTTP request. Half A stops at a `RecordingEgress` request object; half B proves the wire only for GitHub and Slack. The nearest real-runtime gsuite dispatch (`standalone_gsuite_installs_activates_and_dispatches_through_host_runtime`) deliberately seeds a *missing* handle and asserts the failure path; `scenario_uninstalled_tool_call_denied_until_active.rs` reaches `gmail.list_messages` dispatch but asserts no header. | Coverage gap | **Medium** | `tests/integration/extension_runtime.rs::slack_tools_invoke_through_the_generic_dispatcher_with_recorded_egress` is a ready-made template — the same shape for one gmail capability closes it. |
| **F4** | PROPOSAL §11.2.5, §12.1a and CHECKLIST:552/:597 record the two census evasions and the fail-open reads as live and owed to WS10; all four are closed on this tree. The recordings now understate the seal. The ratchet's test count (19) is stale in three places. | Doc-truth (docs behind the tree) | **Low** | Amend the four sites; CHECKLIST:552's sealed-mint sub-bullet can close. |

**None of F1–F4 is a HOLE.** Each seam either holds outright or holds with a residual that is now recorded — F1 and F2 by this document, F3 as an acknowledged journey gap, F4 in the direction of the docs being *more* pessimistic than the code.

---

## Reproduction

```bash
# Row 5 journeys (libsql; Postgres cases need a Docker daemon)
cargo test -p ironclaw_integration_tests --test reborn_integration_extension_delivery
cargo test -p ironclaw_integration_tests --test reborn_group_extensions
cargo test -p ironclaw_integration_tests --test reborn_integration_secret_injection
cargo test -p ironclaw_composition --test gsuite
cargo test -p ironclaw_extension_support --test gsuite_core
cargo test -p ironclaw_extension_host --features test-support --test lifecycle_contract
cargo test -p ironclaw_extension_host --lib channel_config
cargo test -p ironclaw_extension_host --lib channel_pairing
cargo test -p ironclaw_extension_manager --lib extension_lifecycle_capabilities

# Row 6 seams
cargo test -p ironclaw_architecture_tests --test reborn_sealed_evidence_mint_ratchet   # 19
cargo test -p ironclaw_host_api --test protocol_auth_evidence_seal                     # 4
cargo test -p ironclaw_extension_contracts --test verified_inbound_seal                # 3
cargo test -p ironclaw_extension_host --features test-support --test ingress_router_contract  # 20
cargo test -p ironclaw_host_runtime --lib host_http_egress                             # 7

# F1's measurement — the production binary's feature set for the mint owner
cargo tree -p ironclaw -e normal -f '{p}|{f}' | rg ironclaw_host_api
```

The two sabotage files and the hostile-shape probe were temporary, are described above in full, and were removed before this commit.
