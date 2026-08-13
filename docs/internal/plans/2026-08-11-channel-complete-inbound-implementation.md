# Complete Channel Inbound Contract — Implementation Plan

> **Owner:** channel model PR #7477 (`webapp-inbound-channel`)
> **Authority:** `docs/internal/design/2026-08-11-channel-adapter-contract.md` as amended by the owner-approved mission dated 2026-08-11.

**Goal:** Make channel ingress a one-shot translation boundary: each adapter returns a complete normalized message, including fetched attachment bytes and any relevant conversation context, while the generic host retains validation, sanitization, budgets, policy, persistence, and turn orchestration.

**Boundary:** `ChannelIngress::receive` is the only ingress method. It receives the manifest-restricted egress capability and performs all vendor-specific reads. Generic crates never branch on a channel name and never call back into an adapter to finish admission.

## Task 1: Pin the new contract and caller behavior (RED)

**Files:**
- Modify `crates/contracts/ironclaw_extension_contracts/src/channel_adapter.rs`
- Modify existing Slack and Telegram channel tests under `crates/extensions/packages/{slack,telegram}/`
- Modify `crates/extensions/ironclaw_extension_host/tests/ingress_router_contract.rs`
- Modify `crates/product/ironclaw_assistant/src/inbound_turn/tests/attachments.rs`
- Extend the existing production-wired channel integration suite in `tests/integration/extension_ingress.rs` or `tests/integration/extension_delivery.rs`

1. Change existing tests to call `receive(request, restricted_egress)` and assert returned attachment bytes, descriptor integrity, and history behavior.
2. Pin Slack history lookup to exactly `BotMention | ReplyToBot`; pin Telegram context to `None`.
3. Pin host rejection for duplicates, declared-size mismatch, MIME mismatch/unsupported MIME, per-file/count/total budgets, and policy-rewrite reconciliation without an adapter callback.
4. Pin the production ingress seam by asserting complete bytes reach the existing attachment landing/read path.
5. Run the narrow tests and record the expected compile/behavior failures before implementation.

## Task 2: Make the extension contract complete (GREEN)

**Files:**
- Modify `crates/contracts/ironclaw_extension_contracts/src/channel_adapter.rs`
- Modify `crates/contracts/ironclaw_extension_contracts/src/channel.rs`
- Modify contract conformance fixtures and tests

1. Add a complete normalized attachment value pairing `ProductAttachmentDescriptor` with fetched `InboundAttachment`; implement manual `Debug` that exposes only byte length.
2. Change `NormalizedInboundMessage.attachments` to the new value and add `conversation_context: Option<ChannelConversationContext>`.
3. Keep `ChannelAttachmentRef` as adapter-internal parse/fetch state and document that it never crosses host admission.
4. Reduce `ChannelIngress` to the exact one-method signature and remove both late-fetch methods.
5. Put constant-free shape checks in `NormalizedInboundMessage::validate`.
6. Delete the non-executable `[channel.attachments]` recipe vocabulary and its Telegram manifest section.

## Task 3: Finish Slack and Telegram inside `receive`

**Files:**
- Modify Slack payload/channel/attachment/history modules and existing tests
- Modify Telegram normalize/channel/attachment modules, manifest, and existing tests

1. Introduce package-private parsed-message values carrying normalized message plus pending `ChannelAttachmentRef`s.
2. Fetch every pending attachment through the passed `RestrictedEgress`, preserving all existing response, MIME, size, token-page, two-hop, and path-traversal validation.
3. Fetch Slack history only for the exact shared-trigger predicate and degrade history lookup failure to `None` as today.
4. Always set Telegram conversation context to `None`.
5. Return only complete messages/fragments from `receive`.

## Task 4: Delete generic late completion seams

**Files:**
- Modify `crates/extensions/ironclaw_extension_host/src/ingress/router.rs`
- Modify `crates/extensions/ironclaw_extension_host/src/extension_ingress.rs`
- Modify `crates/contracts/ironclaw_product_contracts/src/{surface,inbound}.rs`
- Modify `crates/product/ironclaw_assistant/src/{workflow,inbound_turn}.rs`
- Update test doubles and callers found by `rg`

1. Construct manifest-restricted egress before `receive` and pass it by reference; preserve retryable/permanent transfer error status mapping.
2. Remove adapter/egress from `InboundAdmission`, batch scheduling, and product calls once parsing completes.
3. Sanitize `message.conversation_context` in the host, preserving newline normalization, control stripping, and oldest-line byte clamping.
4. Route complete channel attachments through the single consuming
   `admit_channel_inbound` door; it separates transient bytes from byte-free
   product metadata internally.
5. Delete `admit_channel_inbound_with_attachment_transfer`, `InboundAttachmentAdmission::Channel`, pinned adapter/egress turn parameters, `ProductInboundEnvelope.channel_attachment_refs`, and every host callback/fork used only by late fetch.
6. Validate attachments in the inbound turn path against `DEFAULT_ATTACHMENT_BUDGETS`, normalized/supported MIME, declared metadata, and policy-rewritten descriptors; descriptor filenames win.

## Task 5: Documentation, charters, and adjacent review fixes

**Files:**
- Update extension-contract, extension-family, assistant, auth, domain, and package guidance named in the mission
- Update `crates/product/ironclaw_webui/CONTRACT.md`
- Update `docs/reborn/extension-runtime/overview.md`
- Amend `docs/internal/design/2026-08-11-channel-adapter-contract.md` with dated corrections
- Fold the requested `serve.rs` no-session-channel warning and review-thread fixes still valid on the live tree

1. Explain the translator boundary, optional halves, manifest-to-trait mapping, complete ingress payload, and enforcing gates.
2. Repair both gate-pinned charter maps without reflowing unrelated rows.
3. Amend the five specified design claims, record deletion of `[channel.attachments]`, and name the per-axis binding test.
4. Make WebUI route and handler documentation field-exact.
5. Apply only verified adjacent review findings authorized by the mission; do not broaden channel-specific behavior outside packages.

## Task 6: Ratchets and narrow verification

**Files:**
- Modify architecture gates and composition budget only from measured failures

1. Run touched-crate tests, the assistant/auth charter gates, WebUI descriptor/handler gates, and affected integration suites.
2. Re-run architecture tests; lower the extension-specificity equality baseline for removed live hits.
3. Re-capture only failing contract ceilings from exact gate output with per-crate rationale.
4. Expand the retired web-push scanner to root `Cargo.toml`, `tests/e2e`, and Python sources; preserve exact persisted-identity allowlists.
5. Move composition LOC/`Arc<dyn>` ceilings only if the measured budget requires it and record each rationale.

## Task 7: Main sync, full battery, signed commits, and PR ownership

1. Fetch and merge current `origin/main` before the battery; resolve conflicts by preserving both current-main behavior and this contract boundary.
2. Run the requested single-script battery to completion with one PASS/FAIL line per command. Fix every real failure and repeat the full battery if any command needed a change.
3. Format and create signed commit(s); push the shared branch.
4. Rewrite the PR title/body for the full diff, including compatibility, rollback, behavior changes, placement reasoning, ratchets, and the no-ingress stream-reply validation defect.
5. Update structurally resolved/stale review replies, resolve actionable threads, and leave only the four owner-named threads answered.
6. Poll CI, new reviews, base freshness, conflicts, and mergeability at run-length cadence. On change: diagnose, fix or sync, revalidate, push, and re-arm. Stop only when all required checks are green and the PR is mergeable; do not merge.
