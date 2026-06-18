# Trace credit settlement and non-transferable contributor credits

Date: 2026-05-02
Branch: `trace-client-server-split-reborn`
Status: Approved design, pending implementation plan

## Goal

Build a production-shaped server-side credit system for Trace Commons where users earn non-transferable contributor credits from reviewed trace utility.

The first version should not mint a transferable token. It should create an auditable internal credit balance backed by server review, downstream utility evidence, and frontier-lab or trusted-worker attestations.

The NEAR path should start as a non-transferable on-chain receipt or balance anchor over finalized credit batches. The server ledger remains the source of truth for v1; NEAR records provide public auditability and account binding, not transferability.

## Approved direction

Use a **shadow-to-settlement pipeline**:

1. A contributor submits a redacted trace envelope.
2. The TraceDAO server validates policy, consent, privacy risk, duplication, tenant scope, and object refs.
3. Accepted traces become credit-eligible but do not immediately settle.
4. Reviewers, benchmark workers, ranker workers, process-evaluation workers, or frontier-lab attestations produce utility evidence.
5. A settlement worker converts eligible utility evidence into append-only credit ledger events.
6. The contributor sees estimated, pending, settled, reversed, and held credit balances.
7. Finalized settlements can enqueue NEAR transactions that mint or update non-transferable credit receipts for contributor account hashes.

This keeps uploads from becoming a token faucet and makes credit defensible: credits are earned because reviewed traces were accepted or used in validated training/evaluation workflows.

## Product model

### Non-transferable contributor credits

Credits are internal account units, not freely transferable assets.

They should be described as:

> Non-transferable account credits representing reviewed contribution utility.

They should not be marketed as appreciating, investable, or backed by future AI data demand.

### NEAR non-transferable receipts

NEAR interactions are in scope for the first production-shaped build, but only as non-transferable settlement receipts.

The on-chain representation should be one of:

- a non-transferable credit balance keyed by a contributor account hash; or
- non-transferable settlement receipts keyed by settlement batch and contributor account hash.

The server ledger remains authoritative for contributor balances, holds, reversals, redemption eligibility, and private evidence. The NEAR contract mirrors finalized settlement state for auditability and should never receive raw trace content, raw contributor identity, lab-private notes, or per-source details.

The contract must not expose a general transfer path. Any standard token interface used for wallet/indexer compatibility must either omit transfer methods or make them always fail with a clear non-transferable error.

### Pending versus settled credit

The system must keep credit states distinct:

- **Estimated**: local or server scoring estimate before central review or downstream use.
- **Pending**: accepted evidence exists, but final settlement checks are not complete.
- **Settled**: final credit can be redeemed inside the product under current policy.
- **Held**: credit is paused by abuse, duplicate, policy, legal, or review concerns.
- **Reversed**: prior pending or settled credit was invalidated according to policy.

Contributor-visible totals must not mix these states.

### Review-backed backing

The economic backing is not the raw trace. It is the auditable chain:

`redacted contribution -> review/utility evidence -> settlement event -> account balance`

For frontier-lab integrations, the preferred evidence is a batch-level signed attestation that references a deterministic source-list hash and agreement id without exposing raw trace data or contributor identity.

## Architecture

### TraceDAO server owns credit authority

This work belongs primarily in `zmanian/tracedao-server`.

The server owns:

- utility attestation ingestion and verification
- settlement policy
- credit ledger writes
- credit-account projections
- credit holds and reversals
- audit events
- admin/operator reporting
- NEAR settlement outbox, transaction submission, and chain reconciliation

IronClaw owns:

- local opt-in, redaction, queueing, and upload behavior
- local credit sync/display using server APIs
- local credit notices and retry outbox

IronClaw must not compute authoritative credit locally.

### NEAR contract boundary

The NEAR contract is a settlement mirror, not the primary accounting system.

Contract responsibilities:

- accept mints or balance updates only from an authorized TraceDAO settlement account
- bind credit to a contributor account hash or explicit NEAR account link
- record settlement batch id, policy version, source-list hash, attestation hash, credit amount, and issuer signature hash
- reject duplicate batch/account/event idempotency keys
- support deterministic reversal or burn events for settled credits that are later invalidated
- expose read-only balance and batch-receipt views
- emit safe events for indexing
- provide an operator pause/freeze path for incident response

Contract non-goals:

- store raw trace payloads
- store raw contributor identities by default
- perform scoring, review, or settlement policy
- allow contributor-to-contributor transfers
- decide redemption eligibility

Server integration responsibilities:

- enqueue a NEAR transaction only after a settlement batch is finalized in the off-chain ledger
- use a durable outbox so DB commit and chain submission are recoverable
- use an idempotency key derived from settlement batch id, credit account hash, event type, policy version, and amount
- poll transaction finality and update chain-anchor status on the settlement batch/account event
- retry transient failures without double-minting
- support dry-run settlement without submitting chain transactions
- keep signing keys out of contributor-visible APIs and ordinary logs
- allow operators to disable NEAR submission while leaving credit settlement active

### Server components

#### Credit policy

`CreditPolicy` is a versioned rule bundle for settlement.

It should define:

- allowed credit event types
- base deltas and caps
- duplicate and novelty adjustment rules
- privacy-risk eligibility
- required review state by event type
- required consent scopes and allowed uses
- per-contributor and per-tenant rate limits
- holding rules for suspicious accounts or clusters
- reversal policy for revocation, expiry, purge, or lab retraction

Every credit ledger event must name the policy version used to calculate it.

#### Utility attestation

`UtilityAttestation` records trusted evidence that traces or batches have downstream value.

Attestation sources:

- reviewer decision
- benchmark conversion
- ranker training candidate or pair export
- process-evaluation worker result
- utility-credit worker result
- frontier-lab batch acceptance or usage receipt

Minimum fields:

- `tenant_id`
- `attestation_id`
- `source_type`
- `source_actor_ref`
- `agreement_id` when applicable
- `export_manifest_id` or `batch_id`
- `source_list_hash`
- `accepted_count`
- `rejected_count`
- `utility_tier`
- `use_category`
- `policy_version`
- `signature_key_id` when signed
- `signature` when signed
- `created_at`
- `raw_attestation_object_ref` or safe canonical JSON hash

Raw lab payloads should not be visible to contributors. Public metadata should use hashes and safe aggregates.

#### Settlement batch

`SettlementBatch` is a periodic, replayable job.

It should:

- select eligible utility attestations and accepted submissions
- apply the active `CreditPolicy`
- produce a dry-run report
- write append-only credit events idempotently
- update or rebuild materialized account balances
- emit audit events and source-list hashes

Settlement starts as an admin-triggered dry-run/non-dry-run command. It can become scheduled after canary evidence is reliable.

#### Credit ledger

The existing `trace_credit_ledger` concept remains the source of truth, but it should be extended or complemented to support final settlement batches and utility attestations.

Ledger events are append-only. Historical rows are never mutated.

Initial event types:

- `accepted_reviewed_trace`
- `benchmark_conversion`
- `ranking_utility`
- `training_utility`
- `regression_catch`
- `duplicate_rejection`
- `privacy_rejection`
- `abuse_penalty`
- `revocation_reversal`
- `settlement_finalized`

Each event should include:

- tenant id
- credit account ref
- contributor principal or pseudonym ref
- submission id or settlement batch id
- event type
- decimal credit delta
- settlement state
- policy version
- evidence ref
- evidence hash
- idempotency key
- reason hash or safe reason code
- actor/job id
- audit event id
- created time

#### Credit account projection

`ContributorCreditAccount` is a materialized view of ledger totals.

It should be rebuildable from the ledger and should separate:

- estimated credit
- pending credit
- settled credit
- reversed credit
- held credit
- last settlement batch
- last sync time

The contributor API should read this projection or a DB view derived from the same event stream.

#### Credit holds

`CreditHold` pauses settlement or redemption.

Hold reasons:

- duplicate cluster under review
- privacy risk under review
- suspected spam or automation abuse
- revoked/expired source under propagation
- lab attestation under dispute
- policy migration or settlement incident
- legal/compliance hold

Holds must be tenant-scoped, audited, reasoned, and visible to admins. Contributor-facing responses should expose only safe hold categories.

## Data flow

### Submission eligibility flow

1. Contributor uploads a redacted `ironclaw.trace_contribution.v1` envelope.
2. Server validates auth-derived tenant and contributor principal.
3. Server re-runs redaction and computes safe hashes.
4. Server classifies privacy risk and duplicate/novelty signals.
5. Server stores accepted low-risk traces or quarantines traces requiring review.
6. Server records estimated credit but does not settle final credit from upload alone.

### Utility evidence flow

1. A reviewer, worker, export job, or lab creates utility evidence.
2. Server validates source eligibility:
   - accepted or approved state
   - not revoked, expired, purged, rejected, quarantined, or aggregate-only
   - consent scope permits the requested use
   - tenant policy permits the requested use
   - signed claim or grant permits the requested use
   - active object refs exist when required
3. Server writes `UtilityAttestation` and audit rows.
4. Evidence remains pending until a settlement batch consumes it.

### Settlement flow

1. Operator runs settlement in dry-run mode for a tenant/time window.
2. Server returns a deterministic report:
   - candidate events
   - skipped sources and safe reasons
   - credit totals by event type
   - hold totals
   - duplicate/reversal totals
   - policy version
   - source-list hash
3. Operator runs non-dry-run settlement.
4. Server writes ledger events idempotently.
5. Server rebuilds credit-account projections.
6. Contributors see updated settled/pending/reversed totals.

### Frontier-lab attestation flow

1. TraceDAO creates an export manifest with source ids and a source-list hash.
2. The lab reviews or uses the batch under an agreement.
3. The lab returns a signed attestation containing:
   - manifest hash
   - agreement id
   - accepted item count
   - rejected item count
   - utility tier
   - use category
   - signature key id
   - signature
4. TraceDAO verifies the signature and writes `UtilityAttestation`.
5. Settlement converts the attestation into contributor credit events.

No raw trace content, raw contributor id, or per-trace lab notes should appear in public metadata or on-chain records.

## API shape

### Contributor APIs

`GET /v1/contributors/me/credit`

Returns:

- estimated credit
- pending credit
- settled credit
- reversed credit
- held credit
- safe explanation summaries
- latest settlement batch refs
- recent safe credit events

`GET /v1/contributors/me/credit-events`

Returns contributor-scoped event history with safe reasons and evidence refs. It must never expose other contributors, raw trace bodies, raw lab notes, or unrestricted export manifests.

### Admin/operator APIs

`POST /v1/admin/credit/settlement-batches/dry-run`

Computes a settlement report without writing ledger events.

`POST /v1/admin/credit/settlement-batches`

Writes a settlement batch and idempotent ledger events.

`GET /v1/admin/credit/settlement-batches`

Lists settlement batches with status, policy version, counts, and safe hashes.

`GET /v1/admin/credit/accounts`

Lists safe account aggregates for operational review.

`POST /v1/admin/credit/holds`

Creates a reasoned credit hold.

`POST /v1/admin/credit/holds/{hold_id}/release`

Releases a hold with an audited reason.

### Worker/lab APIs

`POST /v1/workers/utility-attestations`

Accepts signed or trusted utility attestations from workers or lab bridges.

`GET /v1/admin/utility-attestations`

Lists safe attestation metadata for operators.

The existing utility-credit worker can be preserved as a compatibility path, but the production path should prefer `UtilityAttestation -> SettlementBatch -> CreditLedger` over direct credit mutation.

## Scoring and ranking model

### Score components

Initial components:

- acceptance/review state
- privacy risk
- deterministic redaction pass/fail
- replayability or evaluation usefulness
- duplicate score
- novelty score
- benchmark conversion utility
- ranker candidate/pair utility
- process-evaluation quality
- lab acceptance tier

Scores should be explainable as components, not a single opaque model output.

### Model output is advisory

Ranking/model utility outputs can inform credit, but must not directly mint or finalize credits.

The settlement policy decides how much credit a piece of evidence is worth and records a policy version for replay.

## Abuse controls

Required v1 controls:

- per-contributor and per-tenant daily pending-credit caps
- duplicate cluster dampening
- no final credit for revoked, expired, purged, rejected, quarantined, or aggregate-only sources
- credit holds for suspicious accounts or clusters
- idempotency keys for all utility and settlement events
- signed attestation verification for lab evidence
- admin dry-run review before first non-dry-run settlement
- sampled manual review of high-credit batches

Future controls:

- contributor reputation
- anomaly detection on trace timing and cluster composition
- dispute workflows
- lab attestation retraction workflows
- automated settlement thresholds after canary history matures

## Revocation, retention, and reversals

Revocation and retention transitions must fan out to credit.

Rules:

- Revoked, expired, purged, rejected, quarantined, and aggregate-only sources cannot enter new settlement batches.
- Existing pending credit from those sources should be excluded from user totals.
- Existing settled credit should reverse only according to explicit policy.
- Reversal rows must be deterministic negative ledger events linked to the original event id.
- Contributor-facing explanations must be safe and must not expose trace bodies or lab-private notes.

## Tokenization and NEAR path

Transferable tokenization is out of scope for v1 implementation.

NEAR non-transferable receipts are in scope as an optional settlement mirror once the off-chain ledger path is correct. The contract must be driven by finalized settlement batches as backing evidence.

Preferred sequence:

1. Internal non-transferable credits.
2. Redeemable internal account credits.
3. NEAR non-transferable settlement receipts.
4. Only after legal/product review, consider whether any transferability is appropriate.

On-chain metadata, if introduced, should include only:

- settlement batch id
- source-list hash or commitment
- attestation hash
- policy version
- total settled amount
- issuer signature
- credit account hash or linked NEAR account id

It must not include trace bodies, raw contributor identities, or per-source details that could reveal private corpus contents.

## Observability and operations

Operators need safe aggregate visibility:

- settlement volume
- pending versus settled totals
- skipped source counts by safe reason
- hold counts by category
- reversal totals
- top utility event types
- duplicate cluster impact
- lab attestation acceptance rates
- reviewer/worker source coverage
- canary tenant reconciliation status

Settlement reports should be exportable as operator evidence and should include deterministic hashes for reproducibility.

## Rollout plan

### Stage 1: Shadow credits

Credits are computed and shown as estimated/pending only.

No redemption. No tokenization.

### Stage 2: Settled account credits

Admin-triggered settlement batches finalize non-transferable account credits for canary tenants.

Credits can be redeemed only inside the product.

### Stage 3: Lab attestations

Trusted lab/batch attestations become a production credit evidence source.

### Stage 4: NEAR non-transferable receipts

Finalized settlement batches can mint or update non-transferable NEAR receipts.

This stage can be developed with the first server implementation if transferability remains impossible and the off-chain ledger remains authoritative.

## Out of scope

- Transferable token markets.
- Cash-equivalent payouts.
- Broad public proof of raw training data.
- Letting client-side IronClaw compute authoritative credit.
- Immediate credit settlement on upload alone.
- Open corpus downloads.
- Raw lab-review payloads in contributor APIs.
- Contributor-to-contributor credit transfers.

## Acceptance criteria

### Product behavior

- Contributors can see estimated, pending, settled, reversed, and held credit separately.
- Uploading a trace does not immediately settle credit.
- Utility evidence can be recorded without exposing trace bodies or contributor identities.
- Settlement batches can run in dry-run mode before writing ledger events.
- Settled credits are non-transferable.
- Optional NEAR receipts are minted or updated only after off-chain settlement finalization.

### Security and privacy

- Contributor APIs are tenant-scoped and principal-scoped.
- Utility attestation ingestion validates actor role, tenant scope, consent, allowed use, and idempotency.
- Settlement excludes revoked, expired, purged, rejected, quarantined, aggregate-only, and out-of-scope sources.
- Credit events and audit rows store safe hashes/reasons, not raw traces or raw lab notes.
- NEAR transaction payloads store only safe hashes, batch ids, policy versions, amounts, and account hashes.

### Accounting

- Credit ledger rows are append-only.
- Materialized account balances can be rebuilt from the ledger.
- Reversal rows link back to original events.
- Decimal arithmetic avoids float drift.
- Settlement reports are deterministic for the same policy version and source set.
- NEAR outbox processing is idempotent and cannot double-mint repeated settlement events.

### Operations

- Settlement batches expose safe reports and status.
- Admins can place and release credit holds with reasons.
- Canary tenants can run shadow settlements before enabling settled credits.
- Rollback can disable redemption without deleting ledger/audit evidence.
- Operators can disable NEAR submission without disabling off-chain credit settlement.

## Testing expectations

Implementation should include caller-level tests, not only helper tests.

Required test families:

- same submission ids across two tenants cannot cross-read or cross-credit
- utility attestation handler enforces tenant/role/use/scope checks
- dry-run settlement writes no ledger rows
- non-dry-run settlement writes idempotent ledger rows
- repeated settlement does not double-credit
- revoked source is excluded from new settlement
- revoked source can create deterministic reversal rows according to policy
- contributor credit API separates pending, settled, reversed, and held totals
- credit-account projection rebuild matches stored projection
- lab attestation signature verification accepts valid signatures and rejects tampered payloads
- IronClaw credit sync displays server states without calculating authority locally
- NEAR contract accepts authorized settlement mints and rejects unauthorized mints
- NEAR contract rejects or omits transfer operations
- NEAR outbox retries do not double-submit or double-credit a settlement event
- settlement finalization can run with NEAR submission disabled

## Files and repos likely involved

Primary implementation repo:

- `zmanian/tracedao-server`

Likely server areas:

- storage schema and DB facade for utility attestations, settlement batches, account projections, and holds
- credit ledger write paths
- contributor credit/status APIs
- worker/admin routes
- audit metadata and reconciliation
- CLI helpers for settlement dry-run and batch inspection
- NEAR transaction outbox, submitter, finality reconciler, and operator kill switch

Likely contract areas:

- NEAR contract crate or package for non-transferable settlement receipts
- contract integration tests for authorized mint, duplicate prevention, reversal, pause, and transfer rejection

IronClaw follow-up:

- local credit sync/display shape if server response changes
- CLI/web wording for estimated versus pending versus settled credits
- docs clarifying that server settlement, not local scoring, is authoritative

## Open decisions before implementation

1. What can settled credits redeem for in v1: platform usage, governance/reputation, or deferred payout claims?
2. Which lab attestation format should be the first supported format?
3. Should settlement batches be tenant-scoped only, or tenant plus agreement scoped?
4. Which event types are allowed to produce settled credit during the first canary?
5. What are the initial per-contributor and per-tenant caps?
6. Should the first NEAR contract use aggregate balances, per-batch receipts, or both?
7. Should users link a NEAR account in v1, or should receipts initially bind to server-side account hashes only?

Default assumptions for the implementation plan:

- v1 redemption is platform usage credit only.
- first lab attestation format is signed JSON over canonical payload.
- settlement batches are tenant plus optional agreement scoped.
- first canary settles only `accepted_reviewed_trace`, `benchmark_conversion`, `ranking_utility`, and `training_utility`.
- per-contributor caps are conservative and policy-configured.
- the first NEAR contract records per-batch receipts plus an aggregate view.
- v1 supports server-side account hashes first, with explicit NEAR account linking as a follow-up path.
