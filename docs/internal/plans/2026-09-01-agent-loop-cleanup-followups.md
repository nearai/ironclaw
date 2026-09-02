# Agent loop cleanup follow-ups

The behavior-preserving agent-loop ownership cleanup intentionally leaves the
following behavior-bearing bugs and compatibility decisions unchanged. Each
item needs separate test-first work rather than being folded into a structural
refactor.

## Decompose the capability execution stage

Status: completed by the structural follow-up.

`executor/capabilities.rs` now retains the single `CapabilityStage::process`
path and its cross-mechanic orchestration. One private dispatch owner contains
the mutually recursive outcome/recovery state machine; acyclic leaf modules own
batch scheduling, resume reconstruction, result records, outcome persistence,
failure normalization, and recovery terminals. No additional executor stage or
dispatch path was added.

Invariant: fresh calls and approval, authentication, and external-tool resumes
continue to enter the same `CapabilityStage::process` path. Future cleanup must
extend the private mechanic owners rather than add another dispatcher, stage,
strategy axis, or execution mode.

## Stale reply-completion signals can nudge a later capability turn

Status: confirmed by source-path analysis; not fixed by the cleanup.

1. `AssistantReplyStage` stores three completion flags from a reply.
2. A queued follow-up continues the loop without clearing those reply-specific
   flags.
3. A later capability-only turn does not replace them.
4. A graceful stop can therefore issue a completion nudge based on the prior
   reply.

Follow-up: add a caller-level regression covering trailing reply, drained
follow-up, capability-only turn, and graceful stop. Then define and implement
the flags' expiration lifecycle.

## Retired batch strategy remains in stable family identity text

Status: confirmed contract drift; correcting it changes behavior-visible
family identity.

Commit `7c5d57687` removed `BatchPolicyStrategy` and made model-emitted
multi-call batches parallel subject to host ordering. The cleanup removes the
remaining local `BatchPolicy` enum in favor of contract-owned
`BatchPolicyKind`, but two unbound family fingerprints still name
`DefaultBatchPolicyStrategy(parallel_unless_exclusive)`.

Follow-up: decide compatibility for stable family digests, update fingerprint
text and digest constants together, and pin the change with milestone
regression coverage.

## Goal-refresh checkpoint slot has no live owner

Status: compatibility debt, not a proven runtime failure.

`GoalRefreshStrategyState` has a checkpoint field and default but no live
producer or reader. Deleting it removes a key from new checkpoints and a public
type path.

Follow-up: make an explicit checkpoint-wire compatibility decision, then add
legacy-decode and fresh-payload tests before removing or repurposing the slot.
