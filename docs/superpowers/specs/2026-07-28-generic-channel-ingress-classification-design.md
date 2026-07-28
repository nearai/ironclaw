# Generic Channel Ingress Classification Design

**Date:** 2026-07-28
**Status:** Approved
**Decision:** Classify channel-neutral interactions and commands once in the generic host ingress sink.

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
- Replace tests that manually inject Slack-only production behavior with tests
  that exercise the actual generic host path for Slack and Telegram.
- Fail closed for malformed reserved interaction or command syntax.

## Non-goals

- Changing approval, auth, command authorization, or turn-resume semantics.
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
  - slash commands reach the typed command path;
  - normal messages still submit turns.
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
- `cargo test -p ironclaw_architecture reborn_crate_dependency_boundaries_hold`
- targeted clippy for changed crates with warnings denied

## Compatibility, Rollback, and Risk

The product payload and workflow semantics are unchanged. The shared
classification enum gains a variant, so all exhaustive matches are updated in
the same change. Legacy public parser exports remain available to avoid an
unnecessary source-compatibility break.

The main behavior change is intentional: reserved gate syntax and slash
commands received through any generic channel no longer become model-visible
user messages. Unsupported commands are still rejected by the existing product
command inventory/admission path.

Rollback is a normal revert of this change. No persistence schema, secret,
credential, or external vendor configuration changes are involved.

The highest regression risk is over-classifying natural language. The shared
interaction parser already distinguishes confident reserved forms from phrases
such as “approve this design,” and slash-command classification requires a
leading `/`. Caller-path tests preserve both boundaries.
