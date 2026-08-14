# Reply and delivery: two axes for channel output

**Status:** implemented with dated amendments below · **Date:** 2026-08-11 ·
**Follows:** the unified channel model
(`2026-08-10-unified-channel-model.md`), which unified the *pipeline*. This
reshapes the *contract* that pipeline drives.

**Audience:** anyone touching the channel capability contracts, extension
lifecycle, the delivery coordinator, or the projection stream.

**Implementation amendment (2026-08-11):** the reply/delivery split landed,
but implementation work disproved the post-ack attachment proposal in §7.
The dated amendments at §§4.4, 5, 7, 7.4, 9, and 10 are authoritative over
the preserved proposal text.

---

## 0. Why now

The unified channel model made every channel ride one inbound core, one
delivery coordinator, and one notification-setup surface. It did not touch the
trait those paths call into. `ChannelAdapter` carries eleven methods:

```
activate · cleanup · inbound · fetch_attachment · fetch_conversation_context
deliver · deliver_notification · notification_setup_status
enable_notifications · disable_notifications · list_targets
```

Measured on this branch:

| Method | slack | telegram | web-app |
|---|---|---|---|
| `activate` / `cleanup` | default (no-op) | **overridden** | default (no-op) |
| `inbound` | yes | yes | unsupported (host-side actor authority) |
| `deliver` | yes | yes | yes (renders push) |
| `deliver_notification` | default → `deliver` | default → `deliver` | default → `deliver` |
| notification setup ×3 | unsupported | unsupported | **implemented** |

Only `deliver` is implemented by all three. The trait is a union of
per-channel needs rather than a contract.

But surface area is the symptom. **The root problem is that two independent
concepts are collapsed into one vocabulary**, and §1 is the fix everything
else follows from.

---

## 1. The model: reply and delivery are different axes

| | **Reply** | **Delivery** |
|---|---|---|
| what it is | answering the run's input | reaching someone out-of-band |
| routing | **source-routed** — back where the input came from | **target-resolved** — host-configured or model-chosen |
| exists without a run? | never | yes |
| web-app | stream to the subscribed client | browser push |
| slack / telegram | a message in the thread | a message |

**They are orthogonal, not alternatives.** One run can do both: the answer
streams into your open tab (reply) *and*, if you are not looking, a browser
push tells you it arrived (delivery). Today that is awkward to express because
"reply" and "notification" are competing branches of one decision.

### 1.1 Why this cut, and not the current one

The current code dispatches on **intent** (`FinalReply`, `GatePrompt`,
`BackgroundRunNotice`, …). That is the wrong axis, and it has already produced
a real defect on this branch:

> A gate prompt is a **reply** when a human is sitting in the Slack thread,
> and a **delivery** when a 3am routine is blocked and nobody is there. Same
> intent, same content, different axis. Keying the streaming decision on the
> intent silently dropped the second case — blocked-routine pushes vanished.
> Caught by the blocked-fire journey test, fixed by re-keying on the route.

Naming the axis explicitly makes that class of bug **unexpressible**: the
router decides the axis once, and content never has to imply routing.

### 1.2 Why "delivery" and not "reach" / "notification"

*Delivery* already means target-resolved in this codebase — delivery targets,
delivery attempts, the delivery coordinator, `builtin.outbound_deliver`.
Adopting it costs no relearning. "Notification" is too narrow (it excludes a
model-chosen send to a channel), and inventing "reach" would give us a third
word for something we already have two of.

This also merges what are today two separate ideas — *notifications* and
*outbound delivery* — into one. They already share machinery
(`BackgroundRunNotice` and `ModelDelivery` are both policy-class, both resolve
a validated target, both persist an attempt). They were two labels on one
machine.

### 1.3 The one thing that must stay split inside delivery

**Who chose the target.**

- **User-configured**: the host resolved the user's notification channels.
  Trusted.
- **Model-requested** (`builtin.outbound_deliver`): the model named a target.
  Untrusted — must be validated against what the user actually authorized.

Unify the transport and the attempt record; keep target *authorization* as a
distinct stage with two entry points. Collapsing these would let a
model-chosen target inherit the trust of a user-configured one.

---

## 2. Manifest

Three sections, one per axis. **Absence means unsupported.**

```toml
[channel.ingress]                  # how input arrives (exists today)
verification = { kind = "hmac_sha256", … }

[channel.reply]                    # how a run's answer gets back
transport = "stream"               # | "message"

[channel.delivery]                 # how we reach the user outside a run
transport = "push"                 # | "message"
requires_enrollment = true         # per-user setup before we can deliver
```

Concretely, the entire web-app/Slack difference becomes two words of data:

```toml
# web-app                          # slack / telegram
[channel.reply]                    [channel.reply]
transport = "stream"               transport = "message"
[channel.delivery]                 [channel.delivery]
transport = "push"                 transport = "message"
requires_enrollment = true
```

This retires the `inbound` / `outbound` / `notifications` booleans, which say
*that* a channel does something without saying *how*.

Provider message limits are deliberately not manifest policy. Slack and
Telegram measure different units, so each adapter renders and chunks against
its protocol's authoritative limit before egress.

**Rolling compatibility keeps v3.** The public/current writer emits only the
three section-based axes above. The `ChannelDescriptor` serde boundary alone
retains private read fields for the immediately preceding v3 booleans and the
old presentation-level `max_message_chars`: conversational `outbound = true`
normalizes to message reply + message delivery, while the one deployed
`notifications = true` shape normalizes to enrollment-backed push delivery.
This is an inventory-bounded bridge, not a v4 or a general migration layer;
persisted resolved rows therefore remain active during an upgrade.
Security-sensitive nested recipes keep strict unknown-field rejection. This
bridge is intentionally forward-read only: the current writer does not
re-emit retired booleans, so rolling an already-rewritten manifest row back to
a binary that predates the split requires restoring the prior manifest row (or
reinstalling its prior package), not another permanent dual taxonomy.

---

## 3. Core types

```rust
/// Where an outbound thing is going — the axis, decided once, by the router.
enum OutboundRoute {
    /// Back to the conversation/session the run came from. The request
    /// already carries its run and source binding.
    Reply,
    /// To a resolved target, with no assumption that a run exists. The
    /// delivery resolution already carries its target and authorization.
    Delivery,
}

enum ReplyTransport    { Stream, Message }
enum DeliveryTransport { Push,   Message }
```

**Two transport enums, deliberately.** `Stream` is meaningless for delivery
and `Push` is meaningless for reply; separate types make those nonsense
combinations unrepresentable. Slack's `Message` appearing in both is not
duplication — it is the observation that for Slack the two axes happen to
share a mechanism, which is exactly why the distinction stayed invisible until
web-app existed.

---

## 4. The dispatcher — one place, two entry points

```rust
impl OutboundCoordinator {
    async fn reply(&self, run: &Run, content: ReplyContent) -> Outcome;
    async fn deliver(&self, spec: DeliverySpec, content: Content) -> Vec<Outcome>;
}
```

Both persist a delivery attempt. Both return evidence. **Neither has a silent
skip.**

### 4.1 Evidence is symmetric

| transport | evidence returned |
|---|---|
| `Message` / `Push` (vendor) | vendor message id |
| `Stream` | projection cursor at which the reply is visible |

Both provide durable transport evidence: a vendor acceptance identifier for a
message/push, or a projection cursor proving the stream-visible reply was
committed. Neither is proof that a human client actually rendered the result;
that would require a separate client-acknowledgement contract. This closes a
real hole: today a browser reply produces **no delivery record at all**, so the
durable availability of the user's answer has no uniform audit trail and
web-app is invisible in delivery audits.

### 4.2 The asymmetry that is real, and where it lives

`Message`/`Push` deliver **once, at completion**. `Stream` delivers
**continuously, during the turn**.

That is inherent to the clients, not something to design away. The dispatcher
runs at one moment for both (completion); for `Stream` that call is the
*seal* — "the turn is done and the final state is durable at cursor N" — not
the first time the user saw anything. Incremental flow is that transport's
characteristic, the way chunking at 4096 chars is Telegram's.

### 4.3 The projection stream stays shared infrastructure

It is keyed **per thread**, not per channel, and already has multiple readers
(the WebUI SSE route *and* OpenAI-compat). `Stream` reads from it to obtain
the cursor; it does not own it. What a channel legitimately owns is its
**reader** — the transport that subscribes and forwards frames to its client.
That is the piece with no home today, which is why the browser's half lives in
the WebUI frontend.

### 4.4 Decision: verify, do not own

> **Amended 2026-08-11 — settled as verify.** `StreamDelivered` records the
> projection cursor the turn pipeline already wrote. The delivery coordinator
> verifies and reports that durable **projection-commit** evidence; it does not
> take ownership of transcript/projection persistence or disturb replay. The
> historical variant name does **not** assert that a subscribed browser
> received a frame: there is no client acknowledgement contract, and callers
> must not present the cursor as proof of client receipt.

Should `Stream` **verify** the append (read the cursor the turn already wrote)
or **own** it (move the assistant-message append out of the turn pipeline)?
Owning is conceptually purer — one writer, symmetric with push — but means
surgery on turn/timeline persistence, which touches replay. **Recommendation:
verify first.** It delivers the property without destabilizing replay, and the
interface does not change if the write moves later.

---

## 5. Adapter traits

> **Amended 2026-08-11 — agreement is now enforced, not merely possible.**
> Pairing a trait with a manifest section did not by itself prevent the
> `ChannelSurfaces` options from drifting. `check_binding` now checks each axis
> at activation: vendor/webhook ingress requires `ChannelIngress`, message
> reply requires `ChannelReply`, and delivery requires `ChannelDelivery`;
> authenticated-session ingress and stream reply require their adapter halves
> to be absent because the host owns them. The regression is pinned by
> `ironclaw_extension_host::entrypoint::tests::each_channel_section_must_have_exactly_its_implementing_half`
> (with the host-owned absence cases pinned by
> `a_stream_reply_and_session_ingress_must_bind_no_half`).

```rust
trait ChannelIngress  { async fn receive(&self, verified, restricted_egress) -> InboundOutcome; }
trait ChannelReply    { async fn send_reply(&self, envelope, egress) -> Report; }
trait ChannelDelivery { async fn deliver(&self, envelope, egress) -> Report; }
```

**Eleven methods on one trait → three across three traits.** Web-app implements
only `ChannelDelivery`: `authenticated_session` means the host normalizes
ingress, and `transport = "stream"` means the host publishes the reply. Both
missing halves are meaningful rather than a mystery.

---

## 6. Decision — delete `activate` / `cleanup`

**Only Telegram implements them**, and what it does is `setWebhook` /
`deleteWebhook`: telling the vendor where to POST. That is *ingress
registration*, and every input is already known to the host — it owns the
webhook route and therefore the URL. It becomes one more recipe:

```toml
[channel.ingress.registration]
method = "post"
path = "/bot{credential}/setWebhook"
body = { url = "{webhook_url}", secret_token = "{ingress_secret}" }

[channel.ingress.deregistration]
method = "post"
path = "/bot{credential}/deleteWebhook"
```

The host substitutes the placeholders and runs it through existing restricted
egress with existing credential injection. **Two method bodies become zero**,
and a manifest field cannot drift from its implementation.

Channels needing no registration (Slack — its events URL is configured in the
vendor app; web-app — no webhook) omit the section, which is what "default
no-op" means today, minus the trait surface.

**Rejected:** moving them to a `SurfaceLifecycle` trait. That relocates debt
rather than removing it, and buys generality for zero callers. Add the hook if
a channel ever needs genuinely imperative activation — with a real second
implementor.

---

## 7. Superseded proposal — `receive` is async; attachment fetch stays pre-ack

> **Amended 2026-08-11 — complete messages won.** The owner declined the
> refs/post-ack design after measuring the live path. Both old adapter
> callbacks already ran before the durable acceptance commit, so resolving
> attachments inside `receive` does **not** introduce provider I/O into a
> pre-ack window that was previously post-commit. The actual order is:
>
> 1. verify/bound the request and construct manifest-restricted egress;
> 2. `ChannelIngress::receive` parses and resolves attachment bytes and any
>    conversation context through that egress;
> 3. the host sanitizes context, validates exact descriptor/byte agreement and
>    budgets, and runs inbound policy;
> 4. the host durably accepts the message (idempotency, binding, turn submit,
>    and attachment landing); then
> 5. the router returns 2xx.
>
> The prior `fetch_attachment`/`fetch_conversation_context` callbacks occupied
> step 2 as well, between parse and `accept_prepared_user_message`. Ack-after-
> commit and retry semantics are unchanged.
>
> **Second amendment, found during review:** the commit boundary is unchanged,
> but the ordering relative to the product ledger's replay preflight is not.
> The retired path parsed the event id, checked for a settled replay, and only
> then fetched attachment bytes. A complete-message `receive` must fetch before
> the product sees that id, so an exact vendor redelivery can repeat
> credentialed reads and can return 503 if that repeat fetch fails even though
> the original event settled. Restoring that optimization needs a separate
> host-owned durable verified-request replay guard. It must not be implemented
> as another adapter parse/fetch callback, which would recreate the partial
> contract this decision removes. This is a follow-up risk, not a claim that
> the current ordering is unchanged in every respect.
>
> `[channel.attachments]` was deleted because neither shipped transfer is a
> generic request template. Telegram performs `getFile` and then downloads a
> response-derived, path-validated suffix; Slack follows a payload-derived
> absolute URL and must reject HTML error bodies returned with HTTP 200. Most
> of both implementations validates untrusted vendor responses. Encoding those
> protocols — especially Telegram's path-traversal defense — as a TOML
> validation DSL would be less reviewable and less safe than keeping them in
> package Rust.
>
> Conversation context is also not analogous to an attachment reference: the
> context *is* the content. There is no cheap ref to commit first, and fetching
> it after admission would make the current turn answer without the shared
> messages that make questions such as “can you check?” meaningful. It is
> therefore completed by `receive` too.

The text below records the original post-ack proposal and its reasoning; it is
retained as decision history and superseded by the amendment above.

`inbound` is synchronous today, which is an artificial constraint. Make it
async. But **do not fold attachment fetching into it** — and the obvious
objection ("just ack first") does not apply, for a reason worth stating.

### 7.1 Why we cannot simply ack first

The ingress router acks *after* commit, deliberately:

> `// Durable dedupe + admission commit (idempotency ledger keyed by`
> `// installation + external event fingerprint) plus identity/`
> `// conversation binding and turn submission — synchronous, so the`
> `// router's 2xx is ack-after-commit.`
> — `extension_ingress.rs`

"Commit" here is the durable write, not git: the idempotency-ledger entry and
the submitted turn. This is the at-least-once contract. Ack first, then fail
to write, and the vendor considers the message delivered and never retries —
**silent message loss**. Acking early converts at-least-once into at-most-once.

### 7.2 The real problem: the commit depends on the fetch

```rust
let attachments = self.resolve_inbound_attachments(...).await?;    // ← network
self.accept_prepared_user_message(prepared, envelope, attachments) // ← durable write
```

The fetched bytes are an *argument* to the write, so the vendor round-trip is
unavoidably inside the pre-ack window. **This is a choice about what the commit
contains, not about ordering.** Ordering is already correct.

| | committed | fetch happens |
|---|---|---|
| today | message **with landed bytes** | before commit → before ack |
| target | message **with refs only** | after ack |

Both commit-then-ack. The second commits something cheaper — a ref is already
in the parsed payload, needing no network — so the message is durable
immediately, we ack, and *then* pull the bytes.

### 7.3 Decision

`receive` becomes async but does not fetch. Fetching becomes a declarative
recipe the host runs **post-ack**:

```toml
[channel.attachments]
fetch = { method = "get", path = "/files/{external_file_id}" }
```

No adapter method — consistent with §6: per-channel *data*, generic execution.
Fallback for a channel whose fetch cannot be expressed declaratively: the
outcome carries a deferred handle the host invokes post-ack. Inline fetching
inside `receive` is rejected outright — it moves the round-trip *earlier*,
ahead of the commit.

`fetch_conversation_context` follows the same analysis.

### 7.4 Moot: no unresolved-attachment window

> **Amended 2026-08-11.** Because `receive` returns complete attachments and
> the durable accept still lands their bytes atomically with the message, the
> proposed unresolved window does not exist. No wait/backfill behavior is
> required.

Bytes landing after submission means the turn must tolerate briefly
unresolved attachments. Refs are committed, so nothing is lost if a fetch
fails — but the model may see a message whose bytes are not yet available, and
the loop needs a defined behavior for that window (wait, proceed-and-backfill,
or fail). **This is the open question here, not whether to move the fetch.**

---

## 8. Decision — enrollment moves to the host, with **no** adapter method

Today the adapter owns enrollment storage and exposes three methods. The
consequence is not just surface area: **the host cannot answer "is this user
set up?"**, which is why there is no guardrail before a delivery — the send
simply fails at the adapter when no subscriptions exist.

A push subscription (endpoint + keys, per user, revocable, listed in settings)
*is a per-user delivery registration*. Target model:

- The host stores **per-user delivery registrations**: an opaque,
  size-bounded blob keyed by `(tenant, user, extension_id)`.
- **The one security-relevant check happens before storage, and it is
  generic**: the endpoint must target a host declared in `[[channel.egress]]`.
  Otherwise enrollment is an SSRF primitive that makes the host POST to an
  attacker's URL. The host owns that allowlist, so the host performs the
  check. Size bounds are host-side too.
- **Everything else validates where it is used** — the adapter parses the blob
  at delivery, which is when it needs the endpoint and keys anyway. A
  malformed record fails that delivery and is pruned, on the same path that
  already prunes 404/410.

Each stored row has an opaque host-minted registration id; provider addressing
(including a push endpoint) is never its identity. Existing Web Push documents
remain readable with their original `subscription_id`, and the writer carries
both canonical `records` and a private, structurally flattened
`subscriptions` rollback projection. The generic store does not interpret the
opaque document while producing that projection. A rolled-back writer drops
the unknown canonical projection; the next rollout reads the legacy projection
again without re-keying records.

So there is **no `validate_enrollment`**, and no setup methods at all.

**Client bootstrap data** (the VAPID public key the browser needs to
subscribe) is the public half of a credential the host already owns, injected
under a declared kind — so the host publishes it generically rather than the
channel exposing a bespoke status document.

**Consequence:** registrations become *delivery targets*, unifying them with
the outbound target catalog and giving the coordinator a real gate — a
delivery to a channel with zero registrations is a resolvable "no target"
outcome before any adapter call, not a failure discovered inside the vendor
path.

---

## 9. What changes

| today | becomes |
|---|---|
| `inbound: bool` | presence of `[channel.ingress]` |
| `outbound: bool` | presence of `[channel.reply]` / `[channel.delivery]` |
| `notifications: bool` | presence of `[channel.delivery]` |
| `notifications_require_setup` | `[channel.delivery].requires_enrollment` |
| `reply_mode = streaming\|batched` | `[channel.reply].transport = stream\|message` |
| `deliver()` | `send_reply()` **or** `deliver()`, by axis |
| `deliver_notification()` | `deliver()` |
| routing inferred from `DeliveryIntent` | two-arm `OutboundRoute`; content and authorization remain in their owning request types |
| `NoDelivery` for streaming | `Delivered { via: Projection, cursor }` |
| `activate` / `cleanup` | `[channel.ingress.registration]` recipe |
| `fetch_attachment` / `fetch_conversation_context` callbacks | one complete async `ChannelIngress::receive`, pre-commit through restricted egress |
| 3 notification-setup methods | host-owned registrations |

---

## 10. Open questions

1. **§4.1 audit shape.** Full attempt row for a `Stream` delivery, or a
   lighter marker on the browser's high-traffic path?
2. **Third transport.** A channel that streams but cannot subscribe itself
   (a third-party websocket the host pushes chunks into) would need
   `PushStreaming`. Not built now; the enums are shaped so adding it is not a
   rewrite.

**Settled** (2026-08-11, updated 2026-08-12): prompt/reaction/retraction remain
content rather than routing taxonomy, and the router stores only the two-arm
reply/delivery axis (§1.1); two transport enums rather than one (§3); stream
evidence verifies the existing projection cursor (§4.4); complete inbound
attachments/context stay pre-commit (§7); and delivery registrations live in
`ironclaw_auth`, not `ironclaw_outbound`. Both candidate owners were already
reachable from all consumers, so the tie-break was the outgoing edge: the
adapter-facing registration view belongs in `ironclaw_extension_contracts`,
which auth already depends on and outbound does not. Semantically, a per-user,
revocable, settings-listed delivery grant is credential-shaped.

---

## 11. Sequencing

Each is independently shippable. None belongs in the unified-channel-model PR
— that unifies the pipeline; this reshapes the contract, and mixing them makes
both unreviewable.

| # | Change | Size | Risk |
|---|---|---|---|
| 1 | `OutboundRoute` + terminal outcome for `Stream` (§3, §4) | small | low — closes the audit hole, kills the no-op |
| 2 | Delete `activate`/`cleanup` → ingress-registration recipe (§6) | small | low — one channel affected |
| 3 | Manifest sections replace the booleans (§2) | medium | low — mechanical, gate-checked |
| 4 | Async `receive` returns complete attachments/context (§7 amendment) | medium | medium — vendor I/O remains in the existing pre-ack window |
| 5 | Host-owned registrations (§8) | large | medium — persisted data moves |
| 6 | Split into three traits (§5) | mechanical | low — after 1-5 |
