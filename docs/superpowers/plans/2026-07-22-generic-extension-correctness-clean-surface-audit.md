# Generic Extension Correctness Clean-Surface Audit

**Audit date:** 2026-07-22
**Audited checkout:** `4f0d1bb258771ce6092218890017f57ceb5607e9` plus the complete dirty working tree
**Verdict:** **NOT CLEAN YET: ONE STATIC ARCHITECTURE BLOCKER PLUS FINAL VERIFICATION**. The unsafe gate exemption and replay/dyn smells are fixed. Changed lifecycle/pairing policy remains in composition at the exact functions named below; all runtime evidence also remains pending until the post-rebase SHA exists.

This report is static. It does not claim Cargo, clippy, test, browser, canary, or live-stack results.

## 1. Low-risk fixes made during this audit

1. Added missing `"removed":true` assertions after both Notion removal calls in `scenario_extension_install_reauth_gate`; retry phases can no longer pass after a no-op remove.
2. Extracted reply-target authority policy from the 1,559-line `run_delivery/lifecycle_events.rs` into a focused submodule, bringing the changed production file below the repository's 1,500-line ratchet without an exemption.
3. Reworded one stale module comment from extension “activation” to extension “readiness,” and renamed the local error-mapping helper accordingly. Internal persisted continuation/error variants were not changed.
4. Removed current-run external routing from both model-origin approval bypasses. The bidirectional channel journey now proves the exact operation blocks, is approved, resumes the same run, persists the authorized target, and reaches the selected provider wire.

## 2. Clean-surface findings

| Finding | Status | Required action |
| --- | --- | --- |
| New code in a production file over 1,500 lines without exemption | FIXED | `lifecycle_events.rs` is now 1,381 lines. |
| Optional durable replay service in the production router | FIXED, TEST RUN PENDING | Production `RunDeliveryEventRouter::new` requires the durable event and outbound stores; only explicit `new_ephemeral_for_test` can omit them under test/test-support compilation. |
| Fifth `dyn` store view over one outbound store allocation | FIXED, TEST RUN PENDING | Handoff operations are on the existing `OutboundStateStore`; global replay reads are on the existing `TurnEventProjectionSource`. The temporary one-implementation traits and extra composition store field are absent. |
| New/expanded extension policy in composition | BLOCKER, NARROWED | Authority identity and candidate publication/refresh retention moved to `ironclaw_extension_host`. Residual changed policy remains in composition: lifecycle sequencing/compensation in `RebornLocalExtensionManagementPort::{activate_inner,commit_activation}`, pairing completion/retry/manifest-prefix interpretation in `ChannelPairingService`, and WebApp/no-egress policy in `GenericTriggeredRunDeliveryHook::on_trigger_submitted`. Move it or explicitly disposition the bounded debt with a named follow-up. |
| New external-write capability is ungated for model-origin calls | FIXED, TEST RUN PENDING | `builtin.outbound_delivery_target_route_current` is absent from both ungated/exempt lists and retains the normal `GatedUnlessGranted` policy. Caller-owned target validation remains defense in depth after the approval decision. |
| Prompt regex/keyword routing | CLEAN | No production prompt parsing was found. String matching additions are assertions/test-fake protocol inspection. |
| Provider-specific generic lifecycle behavior | CLEAN | No Slack/Telegram/Notion branch was found in the new generic product-workflow routing path. |
| New dead code suppression or temporary feature shim | CLEAN | No added `allow(dead_code)`, TODO, FIXME, or HACK marker was found. |
| Public Activate operation/state | CLEAN WITH EXPLICIT COMPATIBILITY | No public Activate route/tool is intended. Internal host publication retains activation terminology. The only new manifest-wire residue is the serde alias `activation_success_message` for `connection_success_message`. |
| Production panic additions | NO CONFIRMED HIT | Diff scan found test expectations and poison-recovery lock handling, not a confirmed new production panic. Re-run the repository no-panics check after rebase. |
| Test-only interfaces asserted as product behavior | NEEDS FINAL RUN | New caller-level journeys use production facades, durable stores, or provider doubles at external seams. Tests that isolate enqueue/fan-out use the explicitly named, cfg-gated `new_ephemeral_for_test`; crash recovery contracts use the mandatory durable production constructor. |
| Duplicate/oversized integration tests | REVIEW | `tests/integration/delivery_user_journeys.rs` is 2,144 lines. It covers distinct P0 journeys, but final review should remove helper duplication and verify every scenario is registered in CI rather than granting a size exemption by inertia. |
| Stale/obsolete remote CI diagnosis | CLEANLY CLASSIFIED | The red checks currently shown by GitHub ran at `4f0d1bb`; the local tree contains later fixes. They are evidence of the prior failures, not evidence that the dirty tree is green. |

## 3. CodeRabbit reconciliation

All 19 live, non-outdated CodeRabbit threads were checked against the local tree. “Implemented locally” does not mean the GitHub thread is resolved; the fixes must be pushed and the bot/reviewer must re-evaluate the final SHA.

| Thread | Disposition in current tree |
| --- | --- |
| Admin configuration resolver direct coverage | Implemented: secret, non-secret, filtering, missing/mismatch, and ambiguity branches have direct tests. |
| Invalid admin handle drops cause | Implemented: both parse failures log the bound source before stable error mapping. |
| `activation_success_message` naming | Implemented: live field is `connection_success_message`; old name remains only as an intentional serde alias. |
| Lifecycle error mapping drops cause | Implemented: non-transient source is logged; local helper now uses readiness terminology. |
| Auth generic card raw TSX/current key | Implemented: VM TSX setup is loaded and the neutral key is asserted. |
| Wrong frontend path in runtime docs | Implemented. |
| OAuth readiness incorrectly requires tools | Implemented: usable declared surface is required; tool assertion is conditional for tool-bearing manifests. |
| Incomplete activation-removal scan | Implemented: executable paths and expanded patterns are used. |
| Legacy tenant row can be torn down | Implemented: `without_member` fails with `LegacyTenantOwnerNotCanonicalized` and has direct coverage. |
| `ScheduledLoopRun` serde/kind drift | Implemented in both drift-protection tests. |
| First-party adapter drops sealed scheduled origin | Implemented with a non-`None` forwarding test. |
| Pairing-code newtype shape | Implemented with fallible `new`, shared validation, serde `TryFrom<String>`, and explicit access. |
| Admin config persist/reconcile divergence | Implemented through `replace_with_reconcile` rollback semantics. |
| Refresh deactivates working generation before republish | Implemented: next generation is built/published atomically; the prior generation remains on failure. |
| OAuth reconcile lacks route-level coverage | Implemented: completed/unfenced once, fenced no replay, terminal no dispatch, and cross-scope hidden/no dispatch. |
| Incomplete readiness documentation | Implemented across extensions/auth/checklist docs. |
| MCP failed-refresh contract/coverage | Implemented: initial failure exposes no surface; failed refresh retains the prior snapshot; authority inputs are rechecked. |
| Documentation validation commands absent | Implemented in the deterministic matrix. |
| Re-auth remove calls not asserted | Fixed during this audit: both calls assert `"removed":true`. |

## 4. Checklist reconciliation

The merge-readiness checklist contains 532 evidence gates and intentionally has no checked boxes yet. This is correct: no box may be checked from static inspection or from an earlier SHA. The implementation appears to cover most named P0 behaviors, but merge readiness still requires:

- final-main rebase and conflict resolution;
- deterministic compile/test/clippy/pre-commit evidence on the exact pushed SHA;
- browser journeys for operator/account A/account B and manifest-driven setup UI;
- final-head live canaries for auth/channels/workflow as selected by the testing playbook;
- exact-head security, persistence, rollback, and deployment-state review;
- GitHub mergeability, required approvals, and resolved review threads.

The deleted-test parity report is materialized separately at `docs/superpowers/plans/2026-07-22-generic-extension-correctness-deleted-test-parity-audit.md`. It found 1,024 exact removed test paths/names, including 953 names absent globally, rather than the earlier informal count of 274. Missing P0 journeys identified there must remain merge-blocking until their final CI registration and pass evidence are recorded.

## 5. Final rerun commands

After rebase and consolidation, run the exact commands in Section 8 of the readiness checklist. At minimum, clean-surface sign-off requires:

```bash
git diff --check
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p ironclaw_architecture
scripts/pre-commit-safety.sh
```

Then run every changed owning-crate and registered P0 integration/browser tier selected by `docs/internal/testing-playbook.md`. Record commands, exit codes, date, and exact commit SHA; do not copy evidence from the pre-rebase head.
