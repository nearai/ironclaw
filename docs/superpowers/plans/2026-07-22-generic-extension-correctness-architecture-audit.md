# Generic Extension Correctness Architecture Audit

**Audit date:** 2026-07-22
**Audited checkout:** `4f0d1bb258771ce6092218890017f57ceb5607e9` plus the complete dirty working tree
**Target branch audited:** `origin/main` at `de17222940eab5f90a0939b63b902f073bb852b3`
**Reference:** `origin/main:docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md` revision r11
**Verdict:** **ONE STATIC ARCHITECTURE BLOCKER REMAINS BEFORE REBASE**. The unsafe gate exemption, optional production replay wiring, and two one-implementation store/replay traits are resolved. Changed lifecycle and pairing policy still lives in composition, as named below; it must move to an owner crate or receive an explicit, bounded maintainer disposition. The branch must then be rebased onto current `main` and verified at the resulting SHA.

This is a static architecture audit. Cargo was reserved by the coordinating agent, so this report does not claim compile, clippy, test, or live-runtime evidence.

## 1. Required design outcomes

| Design requirement | Result | Evidence and disposition |
| --- | --- | --- |
| One canonical outbound target ID | PASS | `OutboundDeliveryTargetId` is owned by `ironclaw_host_api`; outbound, product workflow, and triggers reuse or alias it instead of maintaining mirror validators. |
| No prompt parsing or text heuristics for routing | PASS | Trigger/source routing consumes sealed run state and typed target IDs. Added `contains`/`starts_with` uses are assertions or provider-test fakes, not production intent parsing. |
| Provider-neutral lifecycle and delivery policy | PASS | New final-reply, trigger-target, replay, and current-target paths carry extension/adapter IDs as data. No new Slack/Telegram/Notion lifecycle branch was found in the generic product-workflow path. |
| Product/channel policy outside composition | BLOCKER, NARROWED | Durable event delivery lives in `ironclaw_product_workflow`; hosted-MCP authority identity and candidate publication/refresh retention now live in `ironclaw_extension_host`. However, composition still owns changed lifecycle sequencing/compensation in `RebornLocalExtensionManagementPort::{activate_inner,commit_activation}` and pairing completion/retry/command interpretation in `ChannelPairingService::{complete_pairing,finish_pending_completion_with,candidate_code,intercept}`. Those are domain decisions, not dependency assembly. Move them to the extension/product owner or record an explicit bounded exception with a named follow-up. |
| Composition is assembly, not a feature surface | BLOCKER, NARROWED | `factory.rs` and durable replay changes are assembly. The generic host now owns atomic candidate publication and failed-refresh retention, and `HostedMcpDiscoveryAuthority` owns the exact authority comparison. The remaining changed behaviors above still execute from composition; `channel_triggered_delivery.rs` also owns the WebApp/no-egress short-circuit. The PR therefore moves in the target direction but cannot claim the §5.11 assembler-only end state. |
| Fewer DTOs / one vocabulary | PASS WITH REVIEW | The duplicate trigger/outbound target wrappers were removed. New handoff and routing records encode distinct durable/authorized states rather than field-for-field mirrors. |
| Less `dyn`; no one-production-implementation test seams | PASS WITH REBASE REVIEW | The temporary `RunFinalReplyHandoffStore` and `DurableTurnEventReplaySource` traits are gone. Handoff persistence is part of the existing `OutboundStateStore`; indexed lifecycle-log reads are part of the existing, multiply implemented `TurnEventProjectionSource`. `RouteCurrentRunFinalReply` remains a narrow runtime-to-product dependency-inversion seam and must be reconciled with current-main `ProductSurface` during rebase rather than automatically retained. |
| No optional service when production always supplies it | PASS | The production `RunDeliveryEventRouter::new(source, outbound_state)` requires both durable ports and there is no `Default`, no no-argument `new`, and no `with_durable_replay`. The only incomplete constructor is explicitly named `new_ephemeral_for_test` and compiled only for tests or the repository's test-support feature. Internal optionality represents that isolated test mode, not a production wiring choice. |
| No local/deployment-specific domain types | PASS | No new `LocalDev*`, hosted-only domain DTO, or deployment-mode branch was introduced by the audited additions. |
| Public extension lifecycle is exactly three derived states | PASS WITH COMPATIBILITY RESIDUE | Current docs and product projection define `uninstalled`, `setup_needed`, and `active`; remove is the sole public disable action. Internal host publication still uses `activate`/`deactivate`, permitted by the contract. `activation_success_message` remains only as an explicit serde alias for older manifests; the live field is `connection_success_message`. |
| Product actions converge on `ProductSurface` descriptors | REBASE BLOCKER | Current `main` (`bcc7cf962`) advanced the `ProductSurface` descriptor/view migration. The PR adds outbound delivery capability wiring against the pre-rebase `RebornServicesApi`/composition shape. Rebase resolution must retain current-main descriptors/views and adapt the new capability rather than restoring removed facade methods or old wiring. |
| Origin-to-gate policy is fail-closed | PASS, TEST RUN PENDING | `builtin.outbound_delivery_target_route_current` remains an `ExternalWrite`, but is absent from both `UNGATED_LOOP_RUN_CAPABILITIES` and the approval-exemption list, so model-origin routing uses the normal `GatedUnlessGranted` policy. The bidirectional channel journey now parks on `BlockedApproval`, approves the exact gate, resumes the same run, and reads back the sealed destination before asserting provider egress. A future host-sealed Product gesture may use `ConsentSufficient`; natural-language intent may not. |
| Architecture ratchets remain monotonic | UNVERIFIED | The old remote PR head passed the architecture and composition-mass jobs, but the complete local implementation and the post-rebase SHA have not run those checks. |

## 2. New port and type inventory

| Seam | Production implementations | Architectural assessment |
| --- | ---: | --- |
| `CurrentDeliveryTargetResolver` | 1 registry implementation, dynamically populated with extension providers | Acceptable owner boundary if retained: callers need one provider-neutral resolver and runtime provider population is genuine variation. Keep the contract narrow. |
| `RouteCurrentRunFinalReply` | 1 real service plus fail-closed unavailable adapter | Plausible dependency-inversion boundary between the first-party runtime lane and product-owned routing. Re-evaluate after the `ProductSurface` rebase; do not keep it if the current-main descriptor handler can own the operation without a new trait object. |
| `TurnEventProjectionSource` global replay read | Existing owner contract with filesystem, in-memory, and test implementations | Consolidated correctly: the bounded indexed global-log read is a host-owned projection operation on the existing event source, not a new one-implementation port. |
| `OutboundStateStore` final-reply handoff operations | Existing owner contract with filesystem implementation and external-crate test double | Consolidated correctly: the rebuildable handoff row and cursor are outbound projection state and no longer require a fifth `dyn` view of the same store allocation. |

The canonical target identifier and the three run-final-reply record types are not mirror DTOs: they represent target identity, sealed route authority, and durable handoff/cursor state respectively.

## 3. Composition-boundary findings

### 3.1 Correctly placed

- Durable lifecycle event consumption, deduplication, sealed-route revalidation, and provider-neutral coordination live in `ironclaw_product_workflow::run_delivery`.
- Neutral target IDs and route request/error vocabulary live below product/composition in `ironclaw_host_api` and `ironclaw_outbound`.
- Provider mechanics remain behind channel adapters; the generic product path receives IDs and declared metadata.
- Composition's `LateBoundTriggerSourceTurnStateStore` delegates the genuine `TurnStateStore` instead of inventing a second domain store.

### 3.2 Must be resolved before claiming architecture conformance

1. Partially resolved: `ironclaw_extension_host::HostedMcpDiscoveryAuthority` now owns the typed package/raw-manifest/max-tools/credential-generation fence, and `ExtensionHost::publish_candidate` owns candidate validation, first-publish failure recording, failed-refresh retention, and the atomic snapshot generation swap. Exact residual blocker: composition still decides the credential/discovery/recheck/commit sequence and cross-store compensation in `RebornLocalExtensionManagementPort::{activate_inner,commit_activation}`, pairing completion/retry and manifest-prefix interpretation in `ChannelPairingService`, and the WebApp external-egress short-circuit in `GenericTriggeredRunDeliveryHook::on_trigger_submitted`.
2. Resolved during this audit: production router construction requires the durable event source and outbound state store. The incomplete constructor is test-only and explicit.
3. Resolved during this audit: handoff persistence and global replay reads were folded into the existing `OutboundStateStore` and `TurnEventProjectionSource`; the two one-production-implementation traits and extra composition store field are absent.
4. Current `main` changed `ProductSurface` declarations and composition wiring in 22 files overlapping this work. Conflict resolution must preserve `main`'s descriptor/view direction and must not reintroduce old `RebornServices` facade methods.
5. Resolved during this audit: the `ExternalWrite` current-run routing capability was removed from the reviewed ungated seed and the legacy exemption list. Its normal grant remains, target authority still fails closed, and model-origin use now requires approval before the same run can persist and consume the selected destination.

## 4. Current-main rebase requirements

The PR base is `3c51d17a86d606620a553f8a16e00fa994a0cb1b`; `origin/main` is `de17222940eab5f90a0939b63b902f073bb852b3`. There are overlapping edits in, at minimum:

- `crates/ironclaw_host_runtime/src/first_party_tools/{mod.rs,schemas.rs}`
- `crates/ironclaw_product_workflow/src/{lib.rs,reborn_services.rs}`
- `crates/ironclaw_reborn_composition/src/{factory.rs,builtin_capability_policy.rs,webui/facade.rs}`
- `crates/ironclaw_reborn_composition/src/{outbound/mod.rs,runtime/local_dev/tests.rs,runtime/tests/core.rs}`
- integration harness builder/group files

Required conflict rule: take current-main's `ProductSurface`, descriptor, view, and capability-owner changes first; reapply only the new generic routing/delivery behavior on top. Do not resolve conflicts by selecting the old whole file.

## 5. Static safety results

- `git diff --check`: passed before and after the low-risk audit edits.
- Large-file ratchet: fixed for the new delivery code by extracting reply-target authority into `run_delivery/lifecycle_events/reply_target_authority.rs`; `lifecycle_events.rs` is now 1,381 lines instead of 1,559.
- Added production `.unwrap()`/`.expect()` scan: no confirmed new production panic was found; hits were in tests or established poison-recovery lock handling. Must be rerun on the post-rebase diff.
- Dead-code/TODO/HACK scan: no new `#[allow(dead_code)]`, TODO, FIXME, or temporary feature shim was found in the added paths.
- Provider-branch scan: no new provider-name switch in generic lifecycle/routing policy was found.
- Secret/PII scan: no secret values were observed in the audited diff; final staged-files and artifact scans remain required.

## 6. Required close-out

1. Rebase on `de1722294` or later and resolve `ProductSurface` overlaps in the target direction.
2. Move the exact residual composition policy named above to its owner crates, or obtain an explicit bounded maintainer disposition with a named follow-up. The one-implementation replay/handoff port findings are resolved.
3. Run architecture, composition-mass, pre-commit, workspace clippy, and the applicable integration matrix on the exact final SHA, including the newly gated bidirectional channel-routing journey.
4. Rerun this audit against that exact SHA; this dirty-tree report is evidence of review, not a merge verdict.
