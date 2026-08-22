# Telegram Linked Device as Channel Identity

**Status:** Superseded on 2026-08-20 by issue #7715

**Date:** 2026-08-13

**Builds on:** [PROPOSAL.md](PROPOSAL.md) and
[ADR-device-link-auth-hook.md](ADR-device-link-auth-hook.md)

**Superseded by:** the current product decision that workspace-bot pairing and
personal device linking are separate, optional user actions. The historical
design below must not be used as current implementation guidance.

## 1. Problem and motivation

The Telegram extension currently exposes two useful surfaces under one
extension identity, but asks the user to connect them independently:

- the deployment bot is the conversational channel entrypoint and delivery
  sender;
- a linked Telegram account provides personal messaging tools that act as the
  user.

That separation is technically explicit but product-hostile. Completing a
Telegram device link already proves the user's Telegram account ID. Asking the
same user to copy or deep-link a short proof code to the bot repeats identity
proof, leaves the extension card in `Finish setup` after a successful device
link, and makes Telegram behave differently from Slack without a security
reason.

The desired product model is:

1. Link a personal Telegram account once.
2. Use the proven Telegram account ID as the user's channel identity.
3. Admit that account's subsequent bot DMs and group messages without a pairing
   code.
4. Keep Bot API replies and delivery separate from MTProto tools that act as
   the user.

## 2. Current system

The package already returns a bounded `vendor_user_ref` from the authenticated
MTProto login. For Telegram this is the numeric Telegram user ID. The host then
mints or reuses a linked-device credential account and stores the session blob.
The value is rendered to the user but is not currently used by channel
identity.

Telegram's `[channel.connection]` instead declares
`strategy = "web_generated_code"`. Composition registers it in
`ChannelPairingRegistry`; verified inbound DMs can consume the short code; the
pairing service writes an installation-scoped identity binding and a direct
message target. The generic channel connection service deliberately gives that
pairing registry precedence over ordinary identity-binding status.

Slack follows a different path: OAuth returns a proven provider identity, a
generic post-auth hook binds it to the IronClaw user, and inbound resolution
uses that binding. Its adapter can proactively open a DM through Slack's API.

Telegram cannot copy the final Slack step. Telegram bots cannot initiate a
conversation with a user; the user must contact the bot first. The first
durably admitted direct message already causes the generic post-admission
observer to retain the proven DM target, so Telegram should use that existing
path instead of inventing an unusable proactive target.

## 3. Goals

- A completed Telegram device link automatically establishes the caller's
  installation-scoped Telegram channel identity.
- The first bot DM after linking is admitted as the linked IronClaw user with
  no pairing code.
- Replies remain source-routed through the Telegram Bot API.
- Outbound direct delivery becomes available after the user's first admitted
  bot DM records the target.
- Personal Telegram tools remain credential-gated and execute through the
  linked MTProto session.
- Disconnect/unlink removes the personal credential, channel identity binding,
  and retained DM target in a retryable order.
- Completion, reconnect, conflict, and failure behavior is covered at the
  integration seam and with a live full-stack Telegram scenario.

## 4. Non-goals

- Do not replace the Telegram Bot API channel with MTProto ingress.
- Do not consume the linked account's live update stream.
- Do not let the bot initiate a conversation before the user contacts it.
- Do not add Telegram-specific branches to generic host lifecycle code.
- Do not remove proof-code pairing support from other extensions.
- Do not auto-switch one IronClaw user between two Telegram identities. The
  existing connection must be disconnected first.

## 5. Decision

### 5.1 A real linked-device channel strategy

Add `DeviceLink` / `device_link` to the manifest and product channel connection
strategy enums. Telegram changes its channel declaration from
`web_generated_code` to `device_link` and removes its pairing-code deep-link
template and inbound code prefixes.

The strategy means:

- the channel's provider identity is established by completion of the
  extension's device-link auth method;
- no `ChannelPairingService` is registered for the extension;
- connection status comes from the installation-scoped identity binding, with
  credential-account status supplying configured/revoked state;
- the existing device-link card is the setup interface.

This is explicit vocabulary, not an alias for OAuth or proof-code pairing.

### 5.2 One deep linked-device channel-binding module

Add a vendor-blind module in `ironclaw_extension_host` that accepts:

- extension ID;
- provider ID from the declared device-link recipe;
- caller scope from the durable auth flow;
- the validated vendor user reference returned by the device-link adapter.

The module resolves the active channel installation, verifies that the channel
declares the `device_link` strategy for the same provider, constructs the
installation-scoped provider-user key, and returns a small rollback
transaction.

It has three outcomes:

1. **New identity:** bind it and return a rollback that deletes exactly that
   binding if later credential completion fails.
2. **Same identity:** treat the operation as idempotent and return a no-op
   rollback.
3. **Different identity or another IronClaw owner:** reject the link. A caller
   must disconnect the existing Telegram connection before linking a different
   Telegram account.

The implementation must not duplicate `ChannelPairingService` or add a
Telegram condition to the host. The provider and strategy are manifest data.

### 5.3 Completion ordering and compensation

Device-link completion uses this order:

1. The package authenticates Telegram, stores the provisional session blob,
   and returns `Completed { account_label, vendor_user_ref }`.
2. The host validates `vendor_user_ref` and begins the channel-identity binding
   transaction.
3. The host calls the existing linked-device credential completion operation,
   which mints or reuses the account, bumps `link_revision`, and stores the
   durable blob.
4. On success, the host commits by dropping the rollback and reports
   completion.
5. On credential/custody failure, the host awaits the binding rollback before
   returning a sanitized device-link failure.

This ordering never exposes a connected channel without durable linked
credentials. It also avoids the substantially harder inverse compensation of
trying to reconstruct a prior session blob after credential completion.

Rollback is installation- and caller-scoped. It must never delete a binding
written by a concurrent later link attempt. The concrete identity store must
therefore provide an exact conditional delete or equivalent CAS operation;
prefix-only best-effort deletion is insufficient for this transaction.

### 5.4 Relink and conflict policy

Reconnecting the same Telegram identity is idempotent and follows the existing
credential-account reuse plus link-revision bump.

If the caller already has a different identity bound under the same Telegram
installation, completion fails with stable user-facing guidance to disconnect
the existing account first. If the new Telegram identity belongs to another
IronClaw user, completion fails without altering either binding or credential.

This policy avoids a multi-identity swap transaction and makes ownership
changes deliberate. Multi-account Telegram support remains a separate product
decision.

### 5.5 Inbound, reply, and delivery behavior

Once linked, the generic channel actor resolver uses provider `telegram` and
the installation-scoped external actor ID to resolve the IronClaw user. Direct
messages and eligible group messages enter the existing product surface.

The bot's reply path is unchanged. The Bot API answers the source conversation.

The link itself does not create a DM target. After the user sends the first
direct message, the existing post-admission observer records the proven
conversation as that user's Telegram DM target. Outbound delivery can then use
the existing Bot API delivery path. Before that first DM, the product reports
that no Telegram delivery target exists.

An unlinked Telegram actor receives the manifest's connect-required notice,
which directs them to link Telegram in the WebUI; their message is not admitted
as an operator or another user.

### 5.6 Disconnect and unlink

Telegram no longer uses pairing-owned disconnect cleanup. The generic channel
disconnect order applies:

1. revoke and remove the caller's linked-device credential/session;
2. remove the retained Telegram DM target;
3. delete the caller's installation-scoped Telegram identity binding as the
   visible connection-state commit point.

For an extension whose channel connection strategy is `device_link`, the
extension card's unlink action routes to channel disconnect as the sole cleanup
coordinator; it must not first call the generic credential unlink and then
recursively enter channel disconnect. Device-link credentials that do not own a
channel continue using the ordinary credential unlink path. Removing only the
credential would leave an orphan identity binding that still admits bot
messages; removing only the binding would leave a personal Telegram session
usable by tools. Both user-visible entry points must be idempotent and retry the
incomplete tail.

## 6. Product and UI behavior

- Telegram's extension card renders the existing device-link setup rather than
  the generated-code pairing panel.
- A successful link changes the per-user channel state to connected.
- Success copy explains that the user can now message the workspace bot and
  that personal messaging actions use the linked account.
- The card may show the bot username from administrator configuration, but it
  must not generate or display a pairing code.
- Unlink/disconnect wording states that both bot identity and personal account
  access are removed.
- Existing configured Telegram deployments continue to use their bot token,
  webhook secret, webhook URL, and bot username unchanged.

## 7. Security and failure handling

- The external actor ID comes only from a completed, validated device-link
  adapter step; no UI field or model input can supply it.
- The binding key remains installation-scoped, preventing one deployment's bot
  identity from authorizing another deployment.
- A provider mismatch, missing installation, non-device-link channel strategy,
  or malformed vendor reference fails closed.
- Binding collisions do not mutate credentials, existing identities, or DM
  targets.
- Credential completion failures roll back only the newly written binding and
  never report `Completed`.
- Disconnect keeps the identity binding until credential revocation and target
  cleanup succeed, so the visible connected state remains retryable rather than
  falsely disconnected.
- Logs and wire errors carry stable categories only; phone numbers, session
  blobs, codes, passwords, and provider response bodies remain redacted.

## 8. Compatibility and rollout

The manifest change is Telegram-specific. Other proof-code and OAuth channels
are unchanged.

The new strategy variants must land in contracts and all exhaustive consumers
before Telegram emits the new wire value. Older binaries do not understand the
new manifest strategy, so the package manifest and runtime consumers ship in
one release.

No schema rewrite or background data migration is required — and no
compatibility bridge exists either. **The cutover is deliberately breaking for
previously paired users** (owner decision, 2026-08-14, superseding this
document's earlier zero-touch design): a proof-code identity binding written
before this release stops authorizing anything the moment the manifest
declares `device_link`. Identity lookups for a device-link channel consult
only the `device-link-v1` namespace; the retired row is inert data. A
previously paired user therefore

- sees the Telegram channel as not connected — the extension card is back to
  setup and offers device linking;
- receives the manifest's connect-required notice when they DM the bot,
  instead of an admitted turn;
- is offered no Telegram delivery target while unlinked (a target recorded
  before the cutover revalidates the moment they re-link the same account; a
  user who never had one records it with their first admitted DM);
- links their account once, and everything works from there — the same
  first-run ceremony a fresh installation gets.

This is the UX every extension presents when required credentials are
missing, which is the point: one connection model, no second lookup namespace
kept live for compatibility. The versioned key prefix exists purely so
retired rows can never satisfy a device-link lookup; it is not a fallback
chain. Inert rows are neither migrated nor bulk-deleted (no destructive boot
migration); a user's explicit disconnect scrubs both generations for that
user, because the unversioned delete prefix lexically contains the versioned
one.

A retired row also cannot veto a link: cross-user collision checks run only
in the `device-link-v1` namespace. A stale row's owner has no connected
channel through which to clear it, while a colliding live link is always a
freshly authenticated proof of the account it names.

Rollback can restore Telegram's `web_generated_code` manifest declaration and
pairing registry registration; pre-cutover rows are never rewritten, so they
resume authorizing under the restored strategy unchanged. Versioned
device-link bindings are ignored by the legacy strategy, but linked credentials
should be revoked or the `device_link` strategy retained until affected users
disconnect so operators do not leave inaccessible personal sessions behind.

## 9. Test strategy

### Contract and projection tests

- The v3 manifest parser accepts `device_link` and preserves it through the
  available-extension and product connection projections.
- Deep-link templates and inbound code prefixes remain valid only for
  `web_generated_code`.
- Frontend schema and configure-modal tests route `device_link` to the existing
  device-link setup and never render the generated-code panel.

### Extension-host tests

- New binding writes the installation-scoped Telegram identity.
- Same-identity reconnect is idempotent.
- Different identity for the same caller is rejected.
- Identity owned by another IronClaw user is rejected.
- Missing installation, provider mismatch, and wrong strategy fail closed.
- A credential completion failure conditionally rolls back the newly written
  binding.
- A concurrent newer binding is not removed by an older rollback.
- A retired proof-code binding is inert on a device-link channel: it neither
  reports the channel connected, nor admits the actor, nor confers a command
  role, nor validates a delivery target, nor vetoes another user's freshly
  proven link — and explicit disconnect scrubs it alongside the versioned row.

### Integration tests

Drive the production-wired path through the harness:

1. install and activate Telegram with administrator configuration;
2. complete a scripted device link carrying a literal external actor ID;
3. assert the channel reports connected and no pairing code exists;
4. admit a verified bot DM from that actor without proof-code interception;
5. assert the admitted turn runs as the linked IronClaw user;
6. assert the first DM records the outbound target;
7. disconnect and assert credential, target, and identity are all removed;
8. prove a later message from the old actor is rejected.

Add failure legs for binding collision, custody failure rollback, same-account
relink, and different-account rejection. Tests assert durable outcomes at the
identity, credential, conversation, and delivery seams rather than only a final
run status.

### Live full-stack validation

- Link the local Telegram account through the WebUI.
- Confirm the extension card reports connected without a bot pairing code.
- Send a natural-language message to the workspace Telegram bot and confirm the
  resulting turn appears under the same IronClaw user.
- Confirm a bot reply returns to Telegram.
- Confirm outbound delivery becomes available after that first DM.
- Re-run the linked-account read matrix.
- Exercise send, edit, reaction add/remove, and delete only against
  `@ironclawqa_bot`.
- Unlink and confirm both the bot identity and linked-account tools are removed.

## 10. Alternatives considered

### Auto-consume a synthetic proof code

Rejected. It would preserve the wrong product model, require fake inbound
conversation data during device-link completion, and couple a credential flow
to proof-code persistence and continuation semantics.

### Add a Telegram-only completion hook in composition

Rejected. It would violate the vendor-blind host contract, duplicate identity
and cleanup policy, and make a second linked-device channel require another
special case.

### Bind after credential completion and compensate the credential

Rejected. Reverting a reused account's link revision and previous session blob
is more complex and less reliable than rolling back one newly written identity
binding.

### Provision the bot DM target immediately from the Telegram user ID

Rejected. Telegram does not allow a bot to initiate a conversation. The first
verified inbound DM is the authoritative and already-supported target proof.

## 11. Acceptance criteria

- Linking Telegram is the only per-user connection ceremony.
- No Telegram pairing code, pairing route, or pairing registry entry is exposed.
- The linked Telegram actor's first bot message is admitted as the linking
  IronClaw user.
- Bot replies and post-first-DM outbound delivery work through the Bot API.
- All personal messaging tools continue to act through the linked MTProto
  account with existing permissions.
- Relink, collision, failure rollback, unlink, and disconnect semantics are
  covered by automated tests.
- Focused crate tests, integration tests, architecture tests, frontend tests,
  and the live provider scenario pass.
