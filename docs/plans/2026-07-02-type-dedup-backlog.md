# Type Dedup Backlog — Judged Semantic Duplicates (2026-07-02)

Method: `scripts/check-type-duplicates.py` produced 178 cross-crate candidate
pairs (field/variant-signature similarity over 1,762 pub types with ≥3 items);
three review agents then read every pair's definitions, doc comments, and
usage and classified each. Rule context: `.claude/rules/type-placement.md`.

**Verdict totals: 18 TRUE-DUP · 87 JUSTIFIED-MIRROR (14 borderline) · 73 COINCIDENTAL.**

Semantic duplication is real but rare (~18–32 of ~2,900 pub types ≈ ~1%), and
most of it has *different names* — invisible to name matching. The dominant
failure mode: a downstream crate re-declares an upstream enum/struct verbatim
"for decoupling," plus an identity `From`/match that never diverges.

## TRUE-DUP backlog (unify — each is one small PR or a batched cluster)

Grouped by cluster; "owner" per the placement ladder in type-placement.md.

### Cluster 1 — mcp/scripts capability-host copy-paste → owner `host_api` / concept owner
| Pair | Owner | Note |
|---|---|---|
| `mcp::McpHostHttpRequest` = `scripts::ScriptHostHttpRequest` | host_api | 10 identical host_api-typed fields, verbatim copy |
| `mcp::McpCapabilityResult` = `scripts::ScriptCapabilityResult` | host_api | same copy-paste event |
| `extensions::HostedMcpDiscoveredToolAnnotations` = `mcp::McpDiscoveredToolAnnotations` | mcp | copied to dodge a dep that now exists |
| `extensions::HostedMcpDiscoveredTool` = `mcp::McpDiscoveredTool` | extensions | mcp already depends on extensions; identity converter in composition |

### Cluster 2 — provider-tool-call metadata triplicated (replay-critical) → owner `turns`
| Pair | Note |
|---|---|
| `threads::ProviderToolCallReferenceEnvelope` = `turns::ProviderToolCallReference` | 11 identical fields; hand-written identity copy in loop_support keeps them in sync |
| `…Envelope` = `turns::ProviderToolCallReplay` | same family; Reference = Replay + capability_id — unify the family in turns |

### Cluster 3 — wasm limiter copied wholesale → owner `wasm_sandbox_core`
| Pair | Note |
|---|---|
| `wasm_limiter::WasmResourceLimiter` = `wasm_sandbox_core::WasmResourceLimiter` | verbatim incl. private fields |
| `wasm::WitToolLimits` = `wasm_sandbox_core::SandboxLimits` | identical limits triple |

### Cluster 4 — event_projections re-declares upstream enums it already imports
| Pair | Owner |
|---|---|
| `event_projections::AuditProjectionStage` = `host_api::AuditStage` | host_api (identity From, same serde) |
| `event_projections::PendingGateKind` = `turns::TurnBlockedGateKind` | turns (identity From, same serde) |
| `reborn::SubagentTerminalEventKind` = `turns::TurnEventKind` | turns (10-for-10 identity map fn; note serde-case divergence) |

### Cluster 5 — auth ↔ composition manual-token mirrors → owner `ironclaw_auth`
| Pair | Note |
|---|---|
| `auth::ManualTokenSetupRequest` = `composition::RebornManualTokenSetupRequest` | composition builds the auth type field-by-field |
| `auth::SecretSubmitResult` = `composition::RebornManualTokenSubmitResponse` | auth type already documented "safe for product surfaces" |

### Cluster 6 — singles
| Pair | Owner / action |
|---|---|
| `host_api::DispatchInputIssueCode` = `turns::CapabilityInputIssueCode` | host_api (turns already depends on it; add serde) |
| `approvals::CapabilityPermissionState` = `webui_v2::SettingsToolPermissionState` | approvals (webui re-declares wire-stable strings with NO type link — active drift hazard) |
| `loop_support::SkillActivationSelectionError` ~ `host_runtime::HostSkillContextBuildError` | extract shared skill-selection error core in loop_support |
| `ports::SkillActivationRequest` = `product_workflow::RebornSkillActivation` | first_party_extension_ports (bidirectional identity Froms = lockstep) |
| `approvals::LeaseApproval` ⊃ `host_api::GrantConstraints` | embed GrantConstraints + issued_by instead of re-declaring fields |

## Borderline identity-lockstep mirrors (14) — document or collapse

Justified-by-role but joined by an identity `From`/map that has never diverged
("a mapping must earn its keep"). Each needs either a doc comment stating the
independent-evolution rationale, or collapsing into an import:

`CheckpointKind`/`LoopCheckpointKind` · `IndexedMessageKind`/`LoopContextCompactionKind` ·
`CapabilityActivityStatus(View)` · `ThreadLiveWorkSummaryPhase`/`ProductWorkSummaryPhase` ·
`SkillActivationMode`/`RebornSkillActivationMode` · `SkillSourceKind`/`RebornSkillSourceKind` ·
`WebUiCancelReason`/`SanitizedCancelReason` · `RebornResumeGateResponse`/`ResumeTurnResponse` ·
`ProductionWiringComponent`/`RebornReadinessDiagnosticComponent` ·
`RebornCancelRunResponse`/`turns::CancelRunResponse` (strips one serde-skip field) ·
`CapabilityDisplayPreviewView`/`ViewInput` (field-identical, same file) ·
`PendingGateKind`/`LoopGateKind` (Resource vs ResourceWait rename — lockstep hazard) ·
`PendingAuthResume` (flatten → embed `CapabilityCallCandidate`) ·
`DispatchInputIssue`/`CapabilityInputIssue` (identical incl. code enums, identity mapper).

## Explicitly NOT duplicates (do not "fix")

- The reborn layering (turns/threads → event_streams/event_projections →
  product_adapters/product_workflow → webui_v2) produces architecture-enforced
  mirrors; webui_v2's Cargo.toml deliberately bans depending on
  product_adapters. Judged justified.
- The actor/scope 4-tuple (`tenant_id, user_id, agent_id?, project_id?`)
  recurs in 8+ crates — shared vocabulary in genuinely different concepts
  (approval key ≠ auth owner scope ≠ actor scope). Unifying would couple
  unrelated domains.
- Severity/role/status ladders (Low..Critical, log levels, Error/Warning/Info)
  and per-newtype validation errors — generic shapes, not shared concepts.
- The `capabilities::*Request` ↔ `host_runtime::RuntimeCapability*Request`
  family — documented layered facade (+idempotency_key, non_exhaustive).

## Execution notes

- Each cluster is one behavior-preserving PR: pick owner → import → delete
  mirror → fix call sites → full test suite (`--all-features` + default).
- Cluster 2 touches replay-critical serde: add a round-trip fixture test
  BEFORE unifying (testing.md: test through the caller).
- #68 (permission state) first: it is the only one with an active drift
  hazard (untyped wire-string re-declaration in webui_v2).
