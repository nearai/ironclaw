# Reborn Transport Adapter Contract

`ironclaw_transport` defines the policy-free edge between transport-specific
channels and the Reborn kernel.

Transport adapters normalize browser, channel, webhook, CLI, TUI, and IDE
ingress into `TransportIngress`. Kernel-facing services consume that ingress
through `TransportIngressSink`. Runtime or product services deliver normalized
egress back through `TransportRegistry` to a named `TransportAdapter`.

## Owns

- Transport identity and routing: `TransportAdapterId`, `TransportMessageId`,
  and `TransportRoute`.
- Protocol ingress normalization: message text, attachment descriptors,
  sender display name, timezone, typed host `ResourceScope`, opaque transport
  thread id, and transport metadata.
- Protocol egress delivery: replies, status updates, approval/auth prompt
  display payloads, projection updates, and heartbeats.
- Adapter lifecycle and health surfaces.
- Stable, redacted transport error categories.

## Does Not Own

- Authorization, approval resolution, capability dispatch, or trust policy.
- Prompt assembly, model reasoning, turn orchestration, or transcript
  durability.
- Event-log source of truth, projection reduction, or cursor durability beyond
  transport-specific delivery cursors.
- Secret storage, network allowlists, resource budgeting, or sandbox policy.
- Business policy about which channel/user/thread may perform an action.

## Security Invariants

- Typed route fields are authoritative. Transport metadata is supplemental and
  must not override `ResourceScope`, `TransportThreadId`, adapter id, or route
  fields.
- Attachment payload bytes are outside this contract. `TransportAttachment`
  carries descriptors and references only.
- Unknown adapter delivery fails closed with `adapter_not_found`.
- Duplicate adapter registration fails with `adapter_already_exists`.
- Transport errors expose stable kinds and redacted reasons only.

## Current Implementation Slice

This branch adds the contract crate, contract tests, a v1 channel bridge in
`src/channels/transport_adapter.rs`, and Reborn transport composition helpers in
`src/reborn/transport.rs`. Existing `Channel` implementations can now be
wrapped as `TransportAdapter`s, registered into a `TransportRegistry`, started
against a kernel ingress sink, and used for typed-route egress delivery.

`LegacyAgentTransportSink` is the current transition bridge: it converts
`TransportIngress` back into the existing `IncomingMessage` shape and injects it
into the v1 agent path, leaving authorization, approval, conversation,
transcript, model, and tool behavior in their current owning services.
`LegacyAgentTransportSource` starts v1 channels through `RebornTransportRuntime`
and exposes the injected message stream the legacy agent loop can consume
without also starting those channels directly.

The production agent startup path can be exercised with
`REBORN_TRANSPORT=true`: `main` starts the Reborn transport source and
`Agent::run` consumes that source instead of calling `ChannelManager::start_all`
directly. While that source is active, `ChannelManager::respond` and
`ChannelManager::send_status` route outbound replies and status updates through
the same `TransportRegistry` instead of directly calling the legacy channel.
Status egress carries a compatibility metadata payload so the v1 channel bridge
can preserve existing `StatusUpdate` shapes during the cutover.
