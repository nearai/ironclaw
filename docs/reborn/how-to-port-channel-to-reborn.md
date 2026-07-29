# How to port a channel to Reborn

Use this guide when adding a native host surface or an external protocol
integration. Reborn has two entry shapes, but both converge on
`ironclaw_host_api::ProductSurface` for product-facing reads and effects.

## Choose the boundary

| Surface | Owner | Entry shape |
| --- | --- | --- |
| WebChat, CLI, TUI, local control API | host crate | host auth/session resolution -> native handler -> `ProductSurface` |
| Telegram, Slack, or another external protocol | channel extension + host ingress | verify protocol auth -> `ChannelAdapter` -> host ingress -> `ProductSurface` |

Do not model a browser or local host session as an external protocol. Do not
invent `ExternalActorRef`, protocol-auth evidence, delivery sinks, or declared
egress for a native host surface.

## Native host surface

Use `ironclaw_webui` as the WebChat reference. The normal shape is:

```text
HTTP/UI request
  -> host authentication and caller scope
  -> route descriptor and thin handler
  -> BoundProductSurface
  -> ProductSurface query/invoke
  -> projection or authoritative command result
```

Handlers must validate and bound input, derive identity from the authenticated
caller, and avoid direct access to stores, dispatchers, runtime lanes, or
composition internals. Reads use typed `ProductView` descriptors. Mutations
use typed command or capability descriptors and read back durable state when
the contract requires evidence.

Native host checklist:

- [ ] route policy, body limits, rate limits, origin checks, and auth are explicit;
- [ ] handler uses `ProductSurfaceCaller` and `BoundProductSurface`;
- [ ] product DTOs/descriptors live in `ironclaw_product`;
- [ ] frontend code lives under `crates/ironclaw_webui/frontend`;
- [ ] caller-level route tests cover success, denial, scope, and failure paths;
- [ ] whole-path integration coverage is added when behavior crosses turns,
      persistence, runtime lanes, or external-service seams.

## External protocol integration

External integrations implement the current `ChannelAdapter` contract. The
protocol-specific crate owns parsing and rendering; host ingress owns
verification, installation scope, deduplication, persistence, and admission.

```text
external payload
  -> host verifies protocol auth and installation scope
  -> host drops verification secrets and resolves manifest-declared
     non-secret configuration for the verified installation
  -> ChannelAdapter::inbound
  -> normalized inbound message
  -> host admission and ProductSurface
  -> projection/outbound selection
  -> ChannelAdapter::outbound
  -> host-mediated egress and delivery evidence
```

Use the existing implementations as references:

- `crates/ironclaw_slack_extension/src/channel.rs`
- `crates/ironclaw_telegram_extension/src/channel.rs`
- `crates/ironclaw_extension_host/src/ingress/`
- `crates/ironclaw_host_api/src/product_adapter/channel_adapter.rs`

The adapter must not own canonical threads, runs, transcripts, authorization,
approval state, secrets, filesystem access, or direct network clients. It must
not mint trusted trigger requests or trusted inbound markers.

External integration checklist:

- [ ] protocol authentication is verified by host ingress before adapter use;
- [ ] installation, tenant, actor, and conversation scope are host-derived;
- [ ] `ChannelAdapter` returns normalized inbound/outbound shapes;
- [ ] discovery and parsing are side-effect-free;
- [ ] network calls use host-mediated egress and opaque credential handles;
- [ ] delivery returns provider evidence or an explicitly unverified result;
- [ ] caller-level conformance tests cover malformed input, scope isolation,
      duplicate delivery, denial, retry, and permanent failure.

## Verification

Start with the owning crate's guidance and the nearest existing caller test:

```bash
bash scripts/codebase-graph.sh status
rg -n "ProductSurface|ProductView|ProductSurfaceCommandDescriptor|ProductCapabilityDescriptor" crates/ironclaw_product crates/ironclaw_host_api crates/ironclaw_webui
rg -n "trait ChannelAdapter|impl ChannelAdapter" crates
cargo test -p ironclaw_product
cargo test -p ironclaw_webui --all-features
cargo test -p ironclaw_architecture
```

Run `bash scripts/reborn-e2e-rust.sh` when the change affects a whole Reborn
contract or cross-layer runtime behavior. Do not add a new abstraction merely
to make the channel fit an obsolete facade; reuse the existing surface and
typed descriptors first.
