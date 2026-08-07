# Generic Channel Ingress Classification Design

**Date:** 2026-07-28
**Status:** Approved
**Decision:** Classify channel-neutral interactions once in the generic host
ingress sink, and expose product commands only when the resolved channel
manifest explicitly declares them.

## Problem

The product workflow already owns source-neutral approval resolution, auth
resolution, and command dispatch. WebUI reaches those services through typed
product payloads. Bundled channel ingress does not currently construct the same
typed payloads:

- Slack and Telegram production bindings set their optional inbound classifier
  to `None`.
- Composition then discards the binding value and registers `classifier: None`
  unconditionally.
- The generic channel sink therefore admits gate replies such as
  `auth deny gate:<ref>` as ordinary user messages.
- Telegram's production normalizer also reduces bot commands to message text,
  so the generic product command path is not reached.

The QA symptom is a blocked auth run that remains blocked after the advertised
Slack denial command. The next message instead hits the busy response because
the original run was never resumed or cancelled.

## Goals

- Make approval, auth, and command classification a channel-neutral ingress
  behavior for every current and future `ChannelAdapter`.
- Preserve vendor-specific parsing, verification, trigger selection, attachment
  extraction, conversation references, and bot-addressing rules in adapters.
- Route classified payloads through the existing `ProductSurface` workflow
  without channel-specific resume or command execution paths.
- Make command availability a fail-closed channel-manifest declaration instead
  of exposing the complete product command registry to every direct channel.
- Ship Slack and Telegram with only the exact `/status` command enabled.
- Replace tests that manually inject Slack-only production behavior with tests
  that exercise the actual generic host path for Slack and Telegram.
- Fail closed for malformed reserved interaction or command syntax.

## Non-goals

- Changing approval, auth, action-level command authorization, or turn-resume
  semantics. Manifest availability is an exposure boundary, not a substitute
  for administrator, owner, membership, or capability authorization.
- Adding new product commands.
- Moving vendor webhook verification, delivery, rendering, activation, or
  preference-target behavior into the generic host.
- Removing compatibility parser/render exports that may have external callers.
- Redesigning Telegram group trigger policy.

## Options Considered

### 1. Generic host classification

Every normalized channel message is classified in
`GenericChannelInboundSink` before `ProductSurface` admission. The classifier
uses the shared interaction and slash-command grammars and produces typed
`ChannelInboundClassification` values.

This is the selected option. It makes the behavior an invariant of channel
ingress and removes optional wiring that can be omitted.

### 2. Shared classifier wired per channel

The CLI could install the same shared classifier into both Slack and Telegram
bindings and composition could stop discarding it. This is a smaller patch, but
future channel adapters can still omit the binding and silently regress.

### 3. Classify inside each channel adapter

Slack and Telegram could each emit gate and command payloads. This preserves
maximum protocol context but duplicates channel-neutral product grammar,
reintroduces drift, and conflicts with the adapters' normalization boundary.

## Architecture

The ingress flow becomes:

```text
verified vendor request
  -> ChannelAdapter protocol normalization
  -> NormalizedInboundMessage
  -> generic channel classification
       1. interaction resolution
       2. slash command
       3. ordinary user message
  -> ChannelInboundSurfaceRequest
  -> ProductSurface workflow
  -> generic approval/auth/command service
```

`ChannelInboundClassification` gains a `Command(InboundCommandPayload)` variant.
Its existing conversion into `ProductInboundPayload` remains the only mapping
needed by the product surface.

The generic classifier runs interaction parsing before command parsing:

1. Strip only the shared wrapping-inline-code presentation accepted by the
   advertised gate grammar.
2. Parse `approve`, `deny`, and `auth deny` interactions with the existing
   channel-neutral parser.
3. If the text is not an interaction, parse slash-command syntax with
   `parse_product_slash_command`.
4. If neither parser recognizes the text, return no classification so it
   remains a `UserMessage`.

A confident reserved form that fails validation becomes `NoOp`. It must not
fall through as a user turn because malformed authority-bearing syntax should
not reach the model or create a competing run.

## Telegram Command Normalization

Telegram alone supplies structured `bot_command` entities and supports
`/command@botname`. The Telegram protocol normalizer remains responsible for
validating entity offsets, checking the addressed bot, applying its configured
group trigger policy, and reducing a recognized entity to canonical text:

```text
/command arguments
```

The generic host then parses that canonical text into
`InboundCommandPayload`. This keeps vendor addressing rules in the Telegram
adapter while keeping product command grammar and dispatch generic.

Private-chat slash text is also eligible for generic command classification;
the product command inventory and admission service remain authoritative about
whether a command is supported and allowed.

## Manifest-Declared Command Availability

The optional `commands` field on the typed channel descriptor is the sole
declaration of product commands exposed by that channel:

```toml
[channel]
id = "messages"
display_name = "Slack messages"
inbound = true
outbound = true
conversation_model = "continuous"
commands = ["status"]
```

Command entries are exact command tokens without a leading `/`. Missing
`commands` and `commands = []` both mean that the channel exposes no product
commands. There is no compatibility fallback to the global registry. Aliases
are independently declarative: `commands = ["status"]` admits `/status` but
does not implicitly admit `/progress`.

The neutral `ChannelDescriptor` validates command-token shape, length, count,
and uniqueness. The generic channel host validates each declared token against
the centralized product command registry when it assembles the resolved
manifest. A syntactically valid but unknown declaration fails channel assembly
instead of becoming an inert typo.

The resolved allowlist configures the channel's production
`ProductCommandAdmissionService`. Admission checks the exact inbound token
before any product command handler is invoked. A disabled or unknown command
settles as a durable, non-retryable rejection and the user-visible response
names only commands enabled for that channel. When the allowlist is empty, the
response says that commands are unavailable for the channel.

Direct-conversation admission remains a separate rule. A declared command is
still denied in a shared conversation, and future action-level authorization
must still run after availability admission. Manifest authors cannot grant
administrator or operator authority by listing a command.

Interaction-resolution grammar (`approve`, `deny`, and `auth deny`) is not a
product-command inventory and remains available wherever its existing gate
policy permits it. Manifest-declared pairing syntax such as Telegram `/start`
also remains owned by `[channel.connection].inbound_code_prefixes`; it is
intercepted before product command dispatch and is unaffected by
`channel.commands`.

The bundled manifests declare:

```toml
# Slack and Telegram
[channel]
commands = ["status"]
```

No Slack- or Telegram-specific filter, parser, handler, or command
implementation is introduced.

## Wiring Changes

The optional `InboundPayloadClassifier` hook and its copies in
`ChannelExtras` and `ChannelExtensionBinding` are transitional debt and will be
removed. The generic sink will no longer depend on CLI or composition code to
enable channel-neutral behavior.

The remaining channel extras are intentionally not generalized:

- `PreferenceTargetCodec` is vendor grammar and stays adapter-specific.
- `ChannelAdapter` owns vendor normalization, delivery, activation, and cleanup.
- The generic subject-route resolver and storage-root defaults are already
  correctly supplied by the host.

## Duplicate-Path Audit

| Area | Current state | Decision |
| --- | --- | --- |
| Slack gate classifier | Exported and optionally wired, but absent in production | Replace with generic host classification; retain compatibility exports if needed |
| Telegram legacy payload parser | Classifies interactions and commands, but production uses the normalizer | Production tests move to normalizer plus generic sink; retain public parser compatibility |
| Telegram render helpers | Older exports coexist with `ChannelAdapter::deliver` | Out of scope because they do not block ingress classification |
| Preference target codecs | Separate Slack and Telegram implementations | Correct vendor-specific duplication |
| Activation, cleanup, verification, delivery | Separate adapter implementations | Correct vendor-specific behavior |

## Testing

Tests are added before production changes and must initially fail against the
current wiring.

- Host API tests:
  - command classification converts to `ProductInboundPayload::Command`;
  - ordinary text remains unclassified;
  - malformed reserved syntax fails closed.
- Generic channel sink caller-path tests:
  - Slack-normalized auth denial reaches `AuthResolution`;
  - approval replies reach their typed resolution path without injected
    classifier extras;
  - a manifest-declared `/status` command reaches the typed command path;
  - `/model`, `/extension_configure`, and skill mutation commands are rejected
    before their product handlers are invoked;
  - a missing or empty `channel.commands` declaration exposes no commands;
  - rejection feedback lists only the commands enabled for that channel;
  - normal messages still submit turns.
- Manifest contract tests:
  - `commands = ["status"]` parses and round-trips;
  - missing and empty command lists are fail-closed;
  - duplicate, malformed, oversized, and excessive declarations fail
    validation;
  - syntactically valid unknown commands fail generic channel assembly;
  - bundled Slack and Telegram manifests declare exactly `["status"]`.
- Telegram protocol tests:
  - `/command@botname` canonicalizes correctly;
  - UTF-16 entity offsets and command arguments remain correct;
  - commands addressed to another bot do not bypass group trigger policy.
- Production binding/composition tests:
  - bundled Slack and Telegram adapters no longer require classifier extras;
  - generic assembly cannot accidentally disable classification.

Targeted verification follows the owning-crate guidance:

- `cargo test -p ironclaw_host_api`
- `cargo test -p ironclaw_extension_host`
- `cargo test -p ironclaw_telegram_v2_adapter`
- `cargo test -p ironclaw_telegram_extension`
- targeted CLI and composition tests
- `cargo test -p ironclaw_architecture_tests reborn_crate_dependency_boundaries_hold`
- targeted clippy for changed crates with warnings denied

## Compatibility, Rollback, and Risk

The shared classification enum gains a variant, so all exhaustive matches are
updated in the same change. Legacy public parser exports remain available to
avoid an unnecessary source-compatibility break.

`ChannelDescriptor.commands` is additive at the current schema version and
defaults to an empty list while reading older manifests and persisted resolved
manifests. This is intentionally fail-closed: an existing channel package must
explicitly opt into commands before an upgraded host will execute one. The
bundled Slack and Telegram manifests opt into only `status`.

The main behavior change is intentional: reserved gate syntax and slash
commands received through any generic channel no longer become model-visible
user messages. Slash commands not declared by the resolved channel manifest are
durably rejected before command execution.

Rollback of code and bundled assets is a normal revert for installations whose
manifests predate the new field. A new binary may persist a resolved manifest
containing `channel.commands`; an older binary whose strict descriptor rejects
unknown fields may require restoring the pre-deploy installation snapshot
before rollback. No secret material or external vendor configuration changes.

The highest regression risk is over-classifying natural language. The shared
interaction parser already distinguishes confident reserved forms from phrases
such as “approve this design,” and slash-command classification requires a
leading `/`. The command-availability risks are accidentally treating an empty
list as allow-all, checking the allowlist after a side effect, or conflating
pairing/gate grammar with product commands. Caller-path denial and
non-invocation tests preserve those boundaries.
