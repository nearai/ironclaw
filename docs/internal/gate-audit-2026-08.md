# Gate & Ratchet Audit — which ones earn their cost? (2026-08-07)

An audit of every mechanical enforcement surface in this repository — the 37
architecture-test gate files (27,149 lines), the five in-crate module-charter
gates, ~80 CI scripts, and the committed baselines/budgets they read — prompted
by PR #7157 going red in CI six consecutive times across four distinct gates.

Method: every number below comes from a command run during the audit (quoted
inline or in §6's sabotage log). Gates were **sabotage-tested** — the violation
each claims to catch was introduced against the live tree, the failure (or the
silent pass) observed, and the tree restored. Git/GitHub history was mined per
gate for catches vs. bookkeeping. Where a claim comes from reading rather than
running, it says so.

---

## 0. The answer

**Most of these gates are real, armed, and cheap. The pain is not the gates —
it is (a) two LOC ceilings whose counting rule lets test-relocation mint
headroom while their pins sit at zero slack, and (b) a CI shape that reveals
one broken gate per ~1-hour round-trip when a single local command could have
reported all of them in five minutes before the push.**

The evidence in both directions, compressed:

- **The gates catch real things.** The WebUI and assistant charter gates
  caught five genuine defects on #7157 alone (2 unowned handlers + 3 stale
  rows). The same-layer edge inventory has five documented catches of stale
  rows the diff authors hadn't noticed. The origin-gate matrix caught memory
  manifests shipping without `origin_gate_matrix` (fail-closed, #6345 era).
  Two ratchets drove their debt to zero and were deleted (#6430: −3,547
  lines; #7202: last port-inversion exception). The panic gate is
  self-ratcheting in both directions and its baseline is clean today
  (measured: 0 stale of 50 entries, 1,266 files, 7.3s).
- **The repo's counter-history justifies paranoia.** Twelve-plus
  green-but-inert gates have been found and fixed (issue #6963's six
  path-keyed gates "green while measuring nothing"; a 204-test module no lane
  had ever run with five assertions drifted red, commit `292c83b5a8`;
  fail-open reads inside the enforcement crate itself, `16bab10248`). Gates
  that are deleted rather than hardened have historically rotted into false
  confidence. This audit found and fixed five more instances of that class
  (§5).
- **The tax is real and measurable.** 226 commits touched the arch-test
  directory in ~3 months; 67 feat/fix commits had to edit existing gate
  files as a side requirement, against ~8–10 commits whose message documents
  a gate catching a substantive defect. On #7157, the two size gates
  extracted **zero production improvement** — both fixes were byte-verified
  test-only relocations — at the cost of two full red-CI round-trips.
  (The zero-slack half of that tax is repaired in this PR at owner
  direction — §3.2; the counting-rule half remains the top open
  recommendation.)

Per-gate verdicts are §2; the ranked shortlist is §3; CI ergonomics
(deliberately separated from gate value) is §4.

---

## 1. What actually happened on PR #7157 (reconstructed from CI logs)

Six consecutive red runs, four distinct gates (branch `channel-delivery-tool`;
check-run logs were still available and were read directly, so this is exact):

| Run head | What failed | Fixed by |
|---|---|---|
| `178fc279eb` | **Assistant `reborn_services` charter** — the map still named pre-rename `OUTBOUND_PREFERENCES_SET_*` symbols | `d7251aec68` (map row rewritten) |
| `d7251aec68` | **Contracts size ceiling** — loop_contracts over its ratcheted ceiling after merging main | `cc5a4acd94` (ceiling 13,850 → 13,949, rationale in-file) |
| `cc5a4acd94` | composition behavior test (not a gate) | `374840b284` |
| `374840b284` | **Composition mass budget** (Code Style) + **WebUI handlers charter** (first firing) + fmt/clippy | `df12b4a8ae` |
| `f1e7fafb71` | **Contracts size ceiling again** — a real +105-line fix pushed loop_contracts to 14,032 vs 13,949 | `d95fccf5a7` |
| `d95fccf5a7` | **WebUI handlers charter still red** | `36024374d9` |

The two size-gate resolutions, verified byte-for-byte:

- `df12b4a8ae`: `runtime/approval.rs`'s 417-line inline `#[cfg(test)]` module
  split verbatim to `runtime/approval/tests.rs`. Production lines 1–241
  identical before/after; moved body token-identical modulo two rustfmt
  re-wraps. Composition's measured count dropped 260 lines **with zero
  behavior evicted** — and the ceiling was re-ratcheted *down* (40,692 →
  40,432), so the relocation was at least banked.
- `d95fccf5a7`: the 919-line inline test module in
  `loop_contracts/src/runtime_context.rs` split identically; production lines
  1–536 identical; ceiling recaptured down 13,949 → 13,115.

The charter failures, by contrast, were genuine: `get_notification_channels` /
`set_notification_channels` had no owner row, and the row they belong to still
named three handlers the PR deleted. The branch also paid composition-budget
bookkeeping on two *further* commits (`c9fad3be0d`, `d39e18766e`) — three
budget touches on one PR.

Post-audit coda: after this audit's cutoff the branch folded main again and
the size ceiling fired **again** — main's #7361/#7363 added 66 lines to
`loop_contracts`' `instruction_bundle.rs`, blowing the 13,115 recapture;
re-captured to 13,181 at `5dde7c3370`. Nothing the branch wrote tripped it.
This recurrence is the §3.2 exhibit: the upward check carries no tolerance,
so main-side growth reds every open branch at its next fold, by construction.

Why the failures surfaced one at a time rather than together is §4.1.

---

## 2. Inventory and verdicts

Verdict key: **KEEP** (armed, earning its cost) · **KEEP-FIX** (armed, with a
named defect or friction to fix) · **MERGE** (subsumed by a sibling) ·
**DEMOTE** (value is documentation, not defense — keep only with that
understanding) · **DELETE**. Sabotage results marked ⚡ are in §6's log; every
gate file below was read end-to-end by the audit.

### 2.1 Architecture-test gates (`crates/app/ironclaw_architecture_tests/tests/`, 37 files, one CI bucket)

| Gate | Protects (one line) | Verdict | Key evidence |
|---|---|---|---|
| `reborn_dependency_boundaries.rs` (5,963 ln, 41 tests) | Layer matrix, 36 per-crate `BoundaryRule`s, CLI exact-dep pin, host_api zero-deps, trusted-trigger seals | **KEEP** (workhorse) with 4 sub-findings below | 140 commits/3mo — highest churn in repo; ~730 forbidden entries incl. ~60 deliberate reintroduction pins |
| — `reborn_contracts_crates_carry_a_checked_size_ceiling` | Logic accreting in the contracts tier (§11.2.3), two-sided (growth + banked slack) | **KEEP-FIX** | ⚡ both jaws armed: +1 line in host_api → red; −1 line in common → red. **The tolerance is asymmetric in code**: `TOLERANCE = 400` binds only the banked-slack direction; the growth check is a bare `lines > ceiling`, so "set to current, not padded" makes every ceiling a hard cap at observed — main-side growth reds every open branch at its next fold (#7157 re-captured loop_contracts **four times, ~once per fold**; see §3.2). **5 of 6 crates within single-digit lines of a jaw today** (host_api slack 0; extension_contracts 6; product_contracts 5; common and prompt_envelope at exactly the 400 banked-slack edge). Counter includes inline `#[cfg(test)]` mass — 26–41% of each crate's counted lines (~17.1k relocatable lines across the six, measured with the panic gate's own lexer). 4 ceiling raises in the gate's first 3 days; #7235's raise **overwrote** #7230's rationale comment. **Repaired in this PR** (owner-directed): `GROWTH_TOLERANCE = 150` on the upward jaw + all six pins re-captured to current; ±1-line probes now pass, +151 still fails |
| — boundary-rule workspace-membership check | A rule whose crate leaves the workspace failing open (PR #3212 class) | **KEEP-FIX** | Directory-basename inventory matching skips `ironclaw_slack_extension`/`ironclaw_telegram_extension` (packages live at `packages/slack|telegram/`) — the #3212 fail-open is still open for exactly those two crates (`assert_no_normal_workspace_deps` returns silently on a metadata-absent crate) |
| — `reborn_product_api_crates_do_not_bind_http_ingress` | Product crates binding TCP/serving HTTP | **KEEP-FIX** | Documented-but-standing gap: webui deliberately uncovered (its `lib.rs:226/244` carry `TcpListener::bind`/`axum::serve`); a dangling comment describes a rule entry that was never added; contradicts the webui BoundaryRule's charter text. Needs the owner ruling the gate itself asks for |
| — `hosted_mcp_discovery_...` | Ambient startup reconciliation of hosted MCP | **DEMOTE or harden** | Two exact fn-name strings checked in two files; a rename evades; no vacuity probe |
| — everything else in the file | (per-test) | **KEEP** | Fail-closed floors throughout (`checked >= 30/60`, `> 500 files`); vacuity probes (`ScriptRuntime`/`McpRuntime` markers) |
| `reborn_composition_boundaries.rs` (1,186 ln) | Composition stays assembly: service-shaped API, pub-use snapshot, no prompt content, registrar-only hooks | **KEEP** with 3 sub-findings | 13 scanner self-tests incl. symlink attacks |
| — `no_substrate_crate_depends_on_composition_root` | Nothing below app → composition | **MERGE** (recommend) | Fully subsumed by the layer matrix + ~15 BoundaryRule lists. Held a nonexistent crate (`ironclaw_storage`) and two duplicates, silently skipped — **fixed and armed in this PR** ⚡ |
| — `composition_public_pub_use_entries_name_their_consumer` | Every re-export names its consumer | **DEMOTE** (it is documentation) | The check is substring presence of `consumer:`/`pinned by:` in a comment; neither is verified to exist. Live entries say "pinned by: `ironclaw_cli` build" (free text). Fine as forced documentation; not a defense |
| — `composition_crate_installs_installed_tier_only_through_registrar` | Hook trust-ceiling bypass | **KEEP-FIX** | `strip_test_module` truncates at the FIRST inline `#[cfg(test)]` module — production code placed after one escapes the scan (fail-open by file layout); `use HookTrustClass as T;` alias also evades |
| `reborn_transport_product_boundary.rs` | Transports consume the product boundary, not the product crate (100-row frozen webui residue) | **KEEP-FIX → fixed** | ⚡ **Sabotage-confirmed fail-open**: `use ironclaw_assistant::{m::{X}};` recorded zero symbols (first-`}` truncation) and passed; plain-path spelling failed. **Parser fixed + regression fixtures in this PR**; sabotage re-run now red |
| `reborn_extension_host_port_inversion.rs` (1,131 ln) | WS2 host→product edge death; residue ledger | **KEEP** | Residues at zero; ledger exists because scope "was sized five times and was wrong five times" (#7092/#7143/#7145). Note: blesses the dev-dep product edge the operator gate refuses — the two files never reconcile the policy |
| `reborn_operator_port_inversion.rs` | products→products edge invisible to the matrix | **KEEP** | Strictest of the three (all dep kinds); 28 DTO pins spot-verified |
| `reborn_extension_manager_split.rs` | Host/manager split | **KEEP** | Witness checks are substrings ("presence is not retention" — self-acknowledged) |
| `reborn_loop_port_location_scan.rs` | `Loop*Port` single-home | **KEEP** | All 13 owner rows verified live; naming-anchored (a port not spelled `Loop…Port` is ungoverned — by design) |
| `reborn_extension_contract_location_scan.rs` / `reborn_product_contract_location_scan.rs` | One-home/one-import-path per tier | **KEEP** | All frozen names + collision exemptions verified live; three near-identical copies of the scan machinery (drift risk noted in-file) |
| `reborn_persistence_driver_boundary.rs` | DB-driver cones equal-in-both-directions; event_store driver privacy | **KEEP** | ⚡ armed (probe red with exact line); all four allowlists verified equal to live manifests. One flagged debt row: turn_runner's normal `libsql` dep whose only usage is a test module |
| `reborn_registration_pipeline_boundary.rs` | Hosted-MCP vocabulary out of shared lifecycle | **KEEP** | The suite's fail-closed reference (7 of 11 tests are sabotage fixtures; #6963's "visited zero files" incident drove the rework) |
| `reborn_runner_sheds.rs` | WS3/WS4 model-gateway shed stays shed | **KEEP** | All 29 moved items verified in loop_host, zero in runner |
| `reborn_authorized_seal_ratchet.rs` | Only the kernel implements `CapabilityAuthorizer` | **KEEP-FIX** | Line-based `contains("CapabilityAuthorizer for")` — alias (`use … as A; impl A for`) and line-break evasions its younger sibling (`sealed_evidence_mint`) explicitly closed for its own traits were never retrofitted here |
| `reborn_sealed_evidence_mint_ratchet.rs` (1,938 ln) | Evidence-mint authority (13 closed paths) | **KEEP** (exemplary) | Born from a measured vacuous feature-seal; extended after a planted forgery passed every scan (WS12 F1); collapsed-header + alias-resolving matcher; >1000-impl-header floor |
| `reborn_origin_gate_matrix_ratchet.rs` | The 17-capability ungated allowlist (security) | **KEEP** | Size pin never moved; imports the REAL constant from host_api; manifest floor met exactly at 14 |
| `reborn_service_method_freeze_ratchet.rs` | `ProductSurface` = exactly 3 methods | **KEEP** | ~70 frozen facade methods driven to 3; failed loudly when the trait moved crates (worked as designed) |
| `reborn_deployment_mode_branching_ratchet.rs` | Composition never branches on profile | **KEEP-FIX** | Literal-token scan: `Self::Variant` inside a composition impl (16 such refs live today, currently benign) and `use … as P` aliasing are invisible; the CLI's production variant-mapping is outside the scan root |
| `reborn_deployment_mode_typename_ratchet.rs` / `reborn_standalone_typename_ratchet.rs` | Mode names never become types | **KEEP** | Standalone list ran 37 → 0 (the cleanest ratchet arc in the suite); two stale `RebornLocal*` entries carry an expired retirement note |
| `reborn_capability_dto_collapse_ratchet.rs` | Retired capability mirror-DTOs stay dead | **KEEP** (header **fixed in this PR**) | Header promised an empty-allowlist assertion and file deletion that don't exist; two originally-frozen names silently left governance (recorded in the new header) |
| `reborn_struct_test_support_ratchet.rs` | Frozen dead-code/test-support member census (79 paths / 276 members, equality both ways) | **KEEP-FIX** | Census counts only field/method attributes: item-level `#[allow(dead_code)]` (struct/mod/impl) and free functions escape entirely. The equality conversion (#7170) caught real untracked slack (#7147) |
| `reborn_manifest_reparse_gate.rs` | Raw manifest reparse confined to 2 sites | **KEEP** (stale note fixed in this PR) | 6 → 2 entries historically; literal-needle dodge (`use … as R; R::from_toml`) exists |
| `reborn_memory_retired_vocabulary.rs` | #3537 retired memory vocabulary at zero | **KEEP** (floor **added in this PR**) | Had no partial-tree floor unlike its explicit twin; was born with an already-dead sanctioned path (its own header records the rot) |
| `reborn_retired_failure_vocabulary.rs` / `reborn_retired_taxonomy.rs` | Retired vocabularies at zero | **KEEP** | Taxonomy's sanctioned-path staleness check exists because two paths rotted invisibly pre-#6996 |
| `reborn_contracts_vendor_census.rs` | Vendor names in contracts tier: exact census (16/2, 91/9, 1/1 — re-derived exactly during audit) | **KEEP** (exemplary) | Equality in both directions; the 91-occurrence `llm_costs` row is a recorded owner-decision queue item, not an oversight |
| `reborn_same_layer_edge_inventory.rs` (1,528 ln) | Same-layer edges the matrix can't see (70 rows, equality) | **KEEP** (strongest churn-to-value ratio in evidence) | Five documented real catches by its own arms; one reviewed growth (72→74) paid for a 2,178-line eviction |
| `reborn_crate_inventory.rs` | The path-resolution meta-layer every gate resolves through | **KEEP** | Its predicted disaster (WS7 family move) happened; ~450 literals survived via the inventory |
| `reborn_cross_crate_include_scan.rs` | `include_str!` dependency-graph bypass | **KEEP-FIX** | `REPORT_ONLY = true` since birth; the named flipping PR (#7094) merged without flipping it; only 16 of ~104 findings are ratcheted; the repo-root-asset half is unbounded; the reclassification loophole (drop the target's `Cargo.toml`) already absorbed most of the WS0 inventory once |
| `reborn_extension_specificity.rs` (2,174 ln) | No vendor names in generic code (derived vocabulary; 119-pair debt list, equality) | **KEEP** (stale entry **fixed in this PR**) | Debt list monotonically shrank 130 → 119; 84 permanent carve-outs are path-unbounded for their 13 terms (growth is review-gated only); highest churn tax in the suite (34 commits, mostly move repoints) |
| `reborn_process_storage_scan_gate.rs` | Request paths never enumerate collections (#7050 perf class) | **KEEP** | Matcher hardened after a demonstrated reformat evasion |
| `reborn_restructure_baselines.rs` | The two shell-gate ratchets stay armed (tamper alarm) | **KEEP** | Its nudge assertion demonstrably fired (refused a 371-line unrecorded ceiling move) |
| `reborn_tracing_target_syntax.rs` | `tracing!(target = …)` field-form drift (#7146: 120 wrong sites) | **KEEP** (exemplary) | Measures its language premise through a real subscriber rather than asserting it in a comment |
| `reborn_build_script_roots.rs` | Build scripts deriving repo root by counted parent hops | **KEEP** | Born from a real silent-skill-loss near-miss; 5-spelling denylist (novel spellings pass) |
| `telegram_extension_gates.rs` | Retired telegram taxonomy tombstones + file budget | **KEEP** (dead exclusions **fixed in this PR**) | `payload.rs` at 983 of the 999-line budget — will bind soon; unreadable files skip silently (contrary to suite doctrine) |
| `reborn_ratchet_support_scanners.rs` + `ratchet_support/` | Shared-scanner regression fixtures | **KEEP** (false lane claim **fixed in this PR**) | ~19 sibling gates still roll private walkers (consolidation incomplete, self-noted) |
| `reborn_conversations_threads_attachments.rs` | conversations/threads naming-trap severance | **KEEP** | Statement-scoped re-export matcher exists because CodeRabbit caught the exact-string version failing open on #7018 |

### 2.2 Module-charter gates (in their crates; run in that crate's CI bucket)

All five were **re-verified exact against the live tree during the audit: 0
unassigned, 0 stale, 0 duplicated entries.** Three verified real catches in
~3 days of life on main (MCP's arming closed a live inline-mint violation;
#7235 was forced to add the `inspector` row in-commit; #7157's charter catch
above). Zero observed false fires — every source-only body edit passed silently.

| Charter | Size | Verdict | Notes |
|---|---|---|---|
| WebUI `handlers_module_charter` | 19 owners / 224 items | **KEEP** | Caught #7157's five. Blind to top-level `mod`/`pub use` (live example at `handlers.rs:17-18`, mitigated by the directory walk). Local run needs `SKIP_FRONTEND_BUILD=1` (~82s warm, measured) |
| Assistant `reborn_services_module_charter` | 20 owners / 551 items | **KEEP** | The most complete scanner (mods charted, cfg-test exempt). Heaviest local compile of the five (pulls composition test-support) for a text-diff test |
| LLM `module_charter` | 10 owners / 48 files | **KEEP-FIX** | File-granular; **no owner-count floor** — could legally collapse to one `everything` row (product siblings pin ≥11) |
| Auth `module_charter` + severance | 4 owners / 45 files | **KEEP** | The severance half (engine ↮ product_auth, real lexer) is a genuine boundary test living outside the arch crate |
| MCP `module_charter` | 6 modules / 2 grandfathered mints | **KEEP-FIX** | Weakest mechanism: idiom-shaped probe (`reason: "` / `reason: format!` only), **non-recursive walk** (a new `src/foo/bar.rs` escapes), `From<String>` re-add checked in one file only |

Honest framing the charter gates deserve: their dominant value is
**documentation-forcing** — they make ownership decisions get recorded and
stale rows die. They also demonstrably catch. Both product charters have a
`dispatch` catch-all and no entry-quality enforcement (explicitly by design:
"checks coverage and existence, not prose"). Two files copy the armed MCP
charter's "N rules keep the charter honest" phrasing with **no gate behind
them** (`trace_commons/src/contribution/mod.rs`, `capabilities/src/host/mod.rs`)
— a reader pattern-matching the phrase would wrongly assume enforcement.

### 2.3 Script gates and committed baselines

| Gate | Verdict | Evidence |
|---|---|---|
| `check_no_panics.py --reborn-baseline` + baseline (50 entries) | **KEEP** (exemplary) | ⚡ armed both directions; stale entries FAIL (self-ratcheting); baseline clean today; 7.3s local; cargo-metadata-derived scope survived the family moves |
| `check-composition-budget.sh` + `composition-budget.toml` | **KEEP-FIX** | ⚡ armed on real growth; ⚡ **fail-open for production code in `*_tests.rs`-named files** (basename exclusion, nothing verifies cfg-gating — the panic gate disagrees on what "test file" means); inline-test mass counts (25.2% of composition's number, 10,199 lines ≈ 68 tolerance-windows of mintable headroom); live tree is **49 lines below the effective ceiling** — the next routine composition PR trips it. History: every gate-fired event resolved by re-seed (+371, +63, ratify-1122, +4), never by eviction-in-response; evictions happened on program schedule. **Pins re-equalized to observed in this PR** (49 LOC / 10 sites of headroom restored to the designed 150/15 windows, per the TOML's own instructions; probes: +100 passes, +160 fails) |
| `reborn-coverage-ratchet.sh` + `coverage-floor.toml` (global 86.96% + 22 crate floors) | **KEEP** | NOT CI-only: `.githooks/pre-push` runs the local ratchet by default (for anyone with hooks installed — nobody, see §4.3); CI enforcement is push-to-main only by owner decision 2026-08-04. #7083 (11 crates silently dropped from both numerator and denominator under enforce=true) is the gate's own inert-precedent |
| `check-target-tree.py` | **KEEP** | 0.2s; shrink-only exceptions; self-tested |
| `check-guidance.py` | **KEEP** | 0.4s; 2,084 path refs verified; this audit leaned on it to make a deletion self-verifying |
| `docs_publication_boundary.py` | **KEEP** | Frozen `.mintignore`, remove-only |
| `regression-test-check.py` + workflow | **KEEP** | Anti-tamper: CI executes the checker from the PR **base** SHA so a PR cannot edit the gate that judges it |
| `critical_mutation_gate.py` (merge-queue) | **KEEP** | Result set must equal the mutant allowlist exactly; zero mutants for a named fn is an error |
| `check-wasm-artifact-freshness.py`, `check-include-str-paths.sh`, `check-hermetic-env.sh`, hermetic runner family, `ws12_workflow_contracts.py`, `ws12_suite_shards.py`, `classify-test-scope.sh`, `package-feature-flags.sh` | **KEEP** | Each self-tested in CI; `ws12_workflow_contracts` is the meta-gate against silently-disconnected lanes |
| `check-test-suite-boundaries.sh`, `check-version-bumps.sh` | **KEEP-FIX** | Gate in CI with **no self-test** (the only gate scripts in that state) |
| `delta_lint.sh` | **DEMOTE** (status quo) | Opt-in, documented fail-open on base detection |
| `quality_gate.sh` | **KEEP** | The local CI-parity gate; notably uses `--no-fail-fast` — the exact flag CI lacks |
| `quality_gate_strict.sh` | **DELETE (recommend)** | Zero references anywhere; superseded by `quality_gate.sh` |
| `check-e2e-matrix-files.sh` | **DELETE — done in this PR** | Zero references; default target `.github/workflows/e2e.yml` was deleted with the v1 monolith (`b6da0272a8`) |
| `check-boundaries.sh` | **DELETE — done in this PR** | Measured broken on a clean tree (recorded 2026-08-05); never run by CI; two guidance files claimed it "enforces" gating it never enforced |
| `test-ci-artifact-naming.sh`, `test_cut_ironclaw_release.py`, `test-import-reborn-run-artifact.py`, `test-reset-extension-state.sh`, `test-pre-commit-safety.sh`, `tests/test_check_reborn_responses_e2e_manifest.py` | **KEEP-FIX (wire or delete)** | Self-tests that run in **no workflow** — the exact shape that produced the 204-dark-test find. `test-pre-commit-safety.sh` is the highest-value wire-up (its subject runs on every hook-installed commit) |

---

## 3. The shortlist that matters (ranked: developer friction × weakness of protection)

1. **CI reports one broken gate per round-trip. Fix the run shape, weaken
   nothing.** (§4.1–4.2; measured: two broken gates → default run reports 1
   in 18s then stops; `--no-fail-fast` reports both in 211s.) One-line-per-lane
   change plus a fast-checks aggregation. This is the highest-leverage item in
   the audit and it touches no gate's semantics.
2. **The two LOC ceilings measure production+inline-test mass and are pinned
   at zero slack.** Composition: ceiling == observed, 150 tolerance, live
   headroom 49 lines; contracts: 5 of 6 crates within single digits of a jaw
   (+1 line in host_api reds the suite — sabotage-verified). Meanwhile 25–41%
   of every counted crate is inline `#[cfg(test)]` mass, so the honest-looking
   fix to any trip is a file move that changes nothing (#7157 did it twice;
   ~27k relocatable lines remain across composition + the six contracts
   crates). Two concrete repairs, both strengthening:
   - **Count only non-test-context lines.** `check_no_panics.py` already
     contains a self-tested Rust lexer + test-context tracker; this audit used
     it to produce every inline-test number above in seconds. Teach
     `check-composition-budget.sh` and `production_rust_files`'s line-count
     consumer to subtract test-context lines (or shell out to a shared Python
     helper), then re-seed all ceilings to the true production counts. This
     kills the relocation-minting class outright, closes the `*_tests.rs`
     basename fail-open (⚡ §6.2), and makes a trip mean what the gate claims:
     behavior accreted.
   - **Give the upward check the tolerance the downward check already has.**
     The asymmetry is in the code, not just the pins: `TOLERANCE = 400` is
     consulted in exactly one direction — the banked-slack check
     (`ceiling.saturating_sub(lines) > TOLERANCE`) — while the growth check
     is a bare `lines > *ceiling`. Combined with the in-file instruction
     "set to current, not padded," every ceiling is a hard cap at the exact
     observed count: one line added *anywhere on main* to a contracts crate
     reds **every open branch** at its next fold until someone re-captures.
     Measured on #7157, this is not hypothetical — the branch re-captured
     `loop_contracts` **four times, roughly once per fold onto main**
     (14,479 → 13,850 → 13,949 → 13,115 → 13,181; the last, at
     `5dde7c3370`, was tripped by main's #7361/#7363 landing 66 lines in
     `instruction_bundle.rs`, nothing the branch wrote). The gate has been
     generating its own busywork.
     **Repaired in this PR at owner direction** (the fold-red recurrence made
     the call): the growth check now allows `GROWTH_TOLERANCE = 150` of
     working slack above each pin — sized to the composition-budget
     precedent, 1.4× the largest routine delta observed (105), and far under
     the reviewed raises the gate has caught (+1,069 / +1,214) — and all six
     ceilings are re-pinned to the counts the test itself reports, which
     also removes the +400 seed padding that had put common/prompt_envelope
     one *deleted* line from the banked jaw. Sabotage-verified: +1 line and
     −1 line (both red before) now pass; +151 lines still fails with the
     effective-ceiling arithmetic in the message. The composition budget got
     the matching maintenance in the same push: its pins had drifted to 49
     LOC / 10 `Arc<dyn>` of live headroom, and were re-equalized to observed
     per the TOML's own instructions (probe: +100 LOC now passes, +160 still
     fails). The dial is one constant if the owner wants a different width.
     Every #7157 recapture would have been absorbed with zero red builds
     under this shape. Remaining from this item: the rationale-comment
     append-only discipline is now stated in the gate's doc (#7235 overwrote
     #7230's +1,214 rationale — that trail is already lossy).
3. **Arch gates are merge-queue-only for most PRs.** The architecture bucket
   runs on a PR only when the PR touches the arch crate or `ironclaw_host_api`
   — a PR touching only `loop_contracts` (i.e., #7157) meets the size ceiling
   for the first time in the queue. The suite costs 4.2 min (measured, warm).
   Recommend: the affected-area planner adds the `architecture-misc` bucket
   whenever anything under `crates/` changes.
4. **Fix the two remaining sabotage-verified fail-opens not fixed here.**
   (a) Slack/telegram BoundaryRules skip silently (inventory basename
   mismatch — §2.1); (b) `composition_crate_installs_installed_tier_only_
   through_registrar` truncates at the first inline test module. Both are
   contained, mechanical fixes in the same style as this PR's transport-gate
   fix.
5. **Retrofit the older literal-token scanners with their younger siblings'
   hardening.** `authorized_seal` (alias + line-break evasion, guarding
   `Authorized` forgery of all things) and the deployment-branching ratchet
   (`Self::Variant` + alias + CLI scope). The mint ratchet contains the
   finished implementation to copy.
6. **Decide `cross_crate_include_scan`'s `REPORT_ONLY` flag.** The PR named as
   its flipper merged without flipping; the residual 16 are ratcheted but the
   ~88 repo-root reach-ins are unbounded. Either flip with the current
   inventory as the allowlist, or retitle the gate a census so it stops
   promising enforcement.
7. **Charter-gate small arms**: owner-count floors for llm/auth (product
   siblings pin ≥11); recursive walk + `From<String>` all-files check for MCP;
   an "(unenforced)" marker or a real gate for the two charter-mimicry module
   docs.
8. **Wire the orphan self-tests or delete them** (§2.3 last row) — six
   checker-tests no lane runs, in a repo whose history includes a 204-test
   module in exactly that state.

Not on the list deliberately: deleting LOC ceilings or charter gates. The
evidence says the charters catch real defects at low cost, and the ceilings —
once they count the right thing — encode a priced, reviewed decision the
dependency gates cannot see (a crate that imports nothing and implements
everything). The budget TOML's own history shows the ceilings *documenting*
growth rather than preventing it (every fire resolved by re-seed), so the
owner should hold them to the §3.2 repair or consciously accept them as
growth-visibility instruments.

---

## 4. CI ergonomics (separate from gate value)

### 4.1 Why one PR ate six round-trips

Three mechanisms, all verified against the workflow files:

1. **Within a bucket, cargo stops at the first failing binary.** All 37 arch
   gate files are test binaries of one crate in one bucket
   (`architecture-misc`); each bucket runs as a single
   `cargo test -p … --all-targets` with no `--no-fail-fast`
   (`reborn-tests.yml:411-414`). `--no-fail-fast` appears nowhere in CI — its
   only repo occurrence is the *local* `quality_gate.sh`. The same stop-early
   shape holds for the root-partition loop, the group-suite loop, the
   integration lanes, and the exact-target PR path (`set -euo pipefail`
   loops). Measured on this tree with two gates broken (§6.6): default run
   reports **1** failing binary in 17.9s and stops; `--no-fail-fast` reports
   **both** in 210.7s.
2. **`cancel-in-progress: true`** (`reborn-tests.yml:46-48`): each fix push
   cancels the still-running sibling jobs that would have surfaced the other
   failures. Serial discovery across *parallel* jobs.
3. **Sequential steps in `code_style` fast-checks:** fmt → … → panic gate →
   … → composition budget run as one job's steps; the first failing step hides
   every later gate.
4. (Throttling, not truncation: PR runs cap crate buckets at `max-parallel: 3`
   and root/integration lanes at **1**.)

**Recommendation R1 (weakens nothing): add `--no-fail-fast` to the five CI
cargo-test shapes and make fast-checks steps non-short-circuiting (collect and
report at the end).** Cost: a failing run finishes its lane (~+3–4 min for the
arch bucket) instead of aborting. Benefit: N broken gates = 1 round-trip.
Caveat for the implementer: `ws12_workflow_contracts.py` pins load-bearing
literal strings inside these workflows — run its self-test with the edit.

### 4.2 Local runnability (measured on a warm tree)

| Gate layer | Command | Time |
|---|---|---|
| Composition budget | `bash scripts/ci/check-composition-budget.sh` | 1.5s |
| Panic baseline (full) | `python3 scripts/check_no_panics.py --reborn-baseline` | 7.3s |
| Target tree / guidance / include_str / hermetic-env / docs boundary | one command each | 0.1–0.5s each |
| Architecture suite (all 37 gates) | `cargo test -p ironclaw_architecture_tests` | 251.9s (nextest would parallelize; not installed on the audited machine) |
| WebUI handlers charter | `SKIP_FRONTEND_BUILD=1 cargo test -p ironclaw_webui --test handlers_module_charter` | 82.0s (compile-dominated; test itself 0.00s) |
| Contracts size ceiling alone | prebuilt binary, single test | 0.2s |

Every gate that fired on #7157 is locally runnable in one command. Nothing
tells a developer this: the commands live in six different files, and the
coverage-floor file is the only one that documents its local runner.

### 4.3 The pre-push gauntlet: it exists, and that's the problem

`.githooks/pre-push` runs `quality_gate.sh` — fmt + clippy + **the full
workspace test suite** — plus the local coverage ratchet (a full instrumented
rebuild) plus a WebUI provider replay that **hard-errors if the Emulate CLI
isn't built**. It is CI-parity maximalism: hours cold, brittle by default, and
**not installed** — this clone (the owner's) has only `.sample` hooks
(verified), `dev-setup.sh` is opt-in, and the repo's two hook-install stories
(`dev-setup.sh` symlinks vs `.githooks/pre-commit`'s `core.hooksPath`
instruction) install *different* pre-commit checks; neither installs both.

**Proposed: `scripts/preflight-gates.sh`** — included in this PR as unwired
tooling. It runs exactly the deterministic-gate classes above: the script
layer (~10s), the architecture suite with `--no-fail-fast` (~4.2 min warm,
nextest-aware), and the module-charter tests of crates the diff touches
(0–90s). Measured end-to-end on this branch: **402.8s (~6.7 min), exit 0,
"every deterministic gate green"** — that run also recompiled the gate
binaries this audit edited; a no-recompile run bounds at ~4.5 min, and
nextest would cut the suite further. Every gate keeps running after a
failure and the script reports the full list at the end. Run against
#7157's failure set it covers all four gates (budget: script layer; contracts
ceiling: arch suite; both charters: changed-crate charter runs) — i.e., all
six red CI runs were reachable locally in one five-minute command before the
first push. Adoption suggestions for the owner: mention it in
`AGENTS.md`'s build/run block, and consider making it the *default* pre-push
hook with today's heavy hook behind an env flag (inverting the current
defaults, which are so heavy they produced zero installs).

### 4.4 Lane-coverage oddities worth knowing (not defects)

- `code_style.yml`'s merge-lane smoke runs
  `cargo test -p ironclaw_architecture_tests reborn` — a **test-name**
  filter. Whole binaries whose test fns lack the substring run 0 tests there
  (conversations 5/5, process-storage 7/7, tracing 3/3, telegram 11/12,
  scanner fixtures 11/11, specificity 5/8 incl. its dependency gate). The
  full plan still runs them; only the redundancy is lost. Either drop the
  filter (the suite is 4 min) or stop treating file names as lane selectors
  (one gate file's false claim about this was fixed in this PR).
- Coverage floors + changed-line gate enforce on push-to-main only (owner
  decision 2026-08-04, documented in-line in the workflow) — post-merge
  enforcement announced via Slack alerts, deliberate and traceable.

---

## 5. Inert / fail-open findings (highest priority, with sabotage evidence)

Fixed in this PR (each its own commit, evidence in the message):

1. **Transport-gate nested-use-group fail-open** — sabotage-verified: a
   brand-new product import spelled `use ironclaw_assistant::{m::{X}};`
   passed the gate; plain-path spelling failed. Parser now does a balanced
   walk; fixtures added; the original sabotage re-run goes red.
2. **`ironclaw_storage` phantom row + duplicates, silently skipped** in
   `no_substrate_crate_depends_on_composition_root` — the skip is now a
   panic naming the stale entry (sabotage-verified with a probe row).
3. **Stale sanctioned path** in `reborn_extension_specificity.rs` exempting a
   file deleted by #6430 (the one exclusion list in that gate with no
   staleness check).
4. **Dead `ironclaw_gateway/static` exclusions** in both telegram cross-tree
   scans (the v1 crate no longer exists).
5. **Missing partial-tree floor** in `reborn_memory_retired_vocabulary.rs` —
   its twin had the 500-file floor from birth; this gate would have passed
   green over a partially-moved tree (the #6963 class). Floor added +
   fixture proving a 10-file tree scans clean and is rejected.
6. **False lane-coverage claim** in the scanner-fixture file's header
   (file-name vs test-name filter).
7. **Gate-header fiction** in the DTO-collapse ratchet (promised assertions
   that don't exist; two names that silently left governance now recorded).
8. **`check-boundaries.sh`** — measured broken on a clean tree, never run by
   CI, cited by two guidance files as "enforcing" gating it never enforced.
   Deleted along with every live reference (check-guidance green).
9. **`check-e2e-matrix-files.sh`** — checker for a workflow deleted with the
   v1 monolith; zero references. Deleted.

Found, verified, and left for the owner (mechanical fixes, but each changes a
gate's blast radius): the slack/telegram BoundaryRule skip; the
registrar-gate first-test-module truncation; the webui HTTP-ingress
documented gap; `REPORT_ONLY` on the include scan; the older seal ratchet's
evasion shapes; the six unwired self-tests.

---

## 6. Sabotage log (methods and outputs)

All probes ran against the live tree with the shipped gate implementations;
arch-gate probes ran the prebuilt test binaries (the gates are runtime file
scanners). Worktree verified clean after each probe.

1. **Panic gate**, both directions: injected `.unwrap()` into a
   shipping-closure production file → exit 1 naming `identity.rs:934` with
   the exact fingerprint; bogus baseline row → exit 1 "Stale baseline entries
   (remove them to ratchet downward)". Clean baseline confirmed: "OK: Reborn
   production panic baseline matches (1266 files, 50 reviewed invariant(s))".
2. **Composition budget**: +200-line production-named file → exit 1
   (40,724 > effective 40,573). Same 200 lines renamed `__audit_probe_tests.rs`
   → exit 0, count unchanged (basename fail-open). Live headroom measured: 49
   LOC (abs), 10 `Arc<dyn>` sites.
3. **Contracts size ceiling**, both jaws: `echo '// comment' >>
   host_api/src/lib.rs` → "18785 production lines over a ceiling of 18784",
   FAILED. Deleting one line from `common/src/lib.rs` → "3392 … (401 of
   slack, window is 400)", FAILED.
4. **Transport product boundary**: nested-group import → `test result: ok`
   (fail-open, pre-fix); plain-path import → FAILED with the gate's message.
   Post-fix: nested-group import → FAILED (armed).
5. **Persistence driver boundary**: `use deadpool_postgres::Pool as …;` in a
   new event_store file → FAILED naming `store.rs:1` and citing §6.3.2.
6. **Fail-fast demonstration**: with the host_api +1 line and the webui
   plain-path import planted simultaneously — `cargo test -p
   ironclaw_architecture_tests`: **1** failing binary reported, 17.93s,
   remaining binaries never ran; `--no-fail-fast`: **2** failing binaries
   reported, 210.74s, all 37 ran.
7. **Substrate-list arming** (post-fix): probe row `ironclaw_zzz_probe` →
   FAILED "is listed in SUBSTRATE_CRATES but is not a workspace package".
8. **Inline-test mass measurement** (via `check_no_panics.py`'s lexer,
   mirroring the budget counter's exclusions): composition 10,199/40,524
   lines (25.2%); contracts crates 26.0–40.5% each (§2.1/§3.2).

Historical precedent this audit stands on (all verified in git):
issue #6963 + `fa3c95d9c0` (six path-keyed gates hardened after being
reproduced "green while measuring nothing"); `292c83b5a8` (204-test module no
lane ran, five assertions drifted red); `16bab10248` (fail-open reads inside
the enforcement crate); `ca4acb30d0` (two gates born inert in one lane,
measured 0→6 and 0→5); `3c3bf37a04` ("a twelfth green-but-inert gate, found
on the way").

---

## 7. What this PR changes vs. recommends

**Changed (eleven fix commits, each with its evidence in the message, plus
this report):** the nine §5 fixes, the charter-count prose fix in
`crates/product/AGENTS.md`, the manifest-reparse allowlist note repoint, and
the proposed-but-unwired `scripts/preflight-gates.sh` (validated end-to-end:
exit 0 on this branch).

A twelfth commit landed because a gate caught this audit's own PR: the
affected-area planner's fail-closed arm refused
`unmapped test or CI path: scripts/check-boundaries.sh` — precisely the
forced-decision behavior its `PR_STATIC_CONTROL_PATHS` comment block
documents, working as designed against the auditor. Both audit-touched
script paths are now classified per that list's membership rule.

**Landed after owner direction (2026-08-07, this conversation):** the §3.2
zero-slack repair itself — `GROWTH_TOLERANCE = 150` on the contracts size
ceiling with all six pins re-captured to current, and the composition
budget's pins re-equalized to observed (record constant moved in the same
commit, as its file requires). These are deliberate loosenings of two
ratchets' *pin states*, made at the owner's call with the sizing rationale
and sabotage evidence in the commits; the gates' catch thresholds for real
growth remain far below every event they exist to catch. Still open from
§3.2: the cfg-test-aware counting rule, which would re-seed these numbers
again and kill the relocation-minting class.

**Recommended, not changed (owner decisions):** `--no-fail-fast` +
fast-checks aggregation in CI (R1); cfg-test-aware LOC counting + mid-window
pins + append-only rationales (R2); arch bucket on any `crates/` change (R3);
the two remaining fail-open fixes (R4); seal/branching-ratchet retrofits
(R5); the `REPORT_ONLY` decision (R6); charter small-arms (R7); orphan
self-test wiring (R8); `quality_gate_strict.sh` deletion; hook-install-story
reconciliation and preflight adoption (§4.3).
