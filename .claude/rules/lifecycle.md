---
paths:
  - "crates/ironclaw_extensions/**"
  - "crates/ironclaw_first_party_extensions/**"
  - "crates/ironclaw_product_adapters/**"
  - "crates/ironclaw_product_adapter_registry/**"
  - "crates/ironclaw_reborn_composition/**"
  - "crates/ironclaw_mcp/**"
  - "crates/ironclaw_wasm/**"
---
# Discovery, installation, and activation

These lifecycle stages are distinct:

1. **Discovery** enumerates descriptors and manifests. It is side-effect-free.
2. **Installation** records an available extension and validates its contract.
3. **Configuration** binds user-owned settings or credential references without
   starting execution.
4. **Activation** registers runtime surfaces and starts explicitly owned
   background work.
5. **Execution** occurs only through authorized, mediated capability dispatch.
6. **Deactivation/removal** stops owned work, unregisters surfaces, and cleans up
   only data named by the lifecycle contract.

Discovery must not connect sockets, start pollers, register hooks, request
credentials, or mutate installation state. Activation must be idempotent and
must expose failure rather than leaving a half-active record.
Authentication rejection is a terminal activation failure: transition to an
explicit failed state and stop reconnect attempts until the credential revision
changes.

Composition owns startup and shutdown orchestration. Descriptor crates remain
declarative; runtime lanes execute; product adapters translate product ingress
and delivery. Do not combine those responsibilities in an extension registry.

Test repeated activation, failed activation rollback, restart reconstruction,
deactivation, and removal through the production caller. Authentication-failure
tests must also prove reconnect does not resume without updated credentials.

## Lifecycle ownership rules

- Manifests describe capabilities and requirements; parsing is not activation.
- Installation records are durable state. An in-memory registry is a derived
  execution view, not the source of truth.
- Configuration stores credential references through secrets/auth contracts;
  it never copies raw credentials into manifests or runtime state.
- Activation validates installation, trust, configuration, and runtime support
  before registering surfaces.
- Background tasks have one lifecycle owner, cancellation, and bounded restart.
- Removal cannot race active execution silently. Define whether it denies,
  drains, cancels, or waits, and test that choice.
- Authentication rejection enters a terminal failure state and stops reconnect
  or retry loops until the credential revision changes. Never resume merely
  because a timer fired, and never hot-loop invalid credentials.
- Installed, configured, and active are distinct query/status states. Listing an
  installation must not imply a registered or healthy runtime surface.
- Restart rehydration reconstructs state through validated constructors and
  re-checks actor/tenant scope, expiry, revocation, installation state, and
  runtime support. Do not deserialize a snapshot directly into trusted/active
  state.

Review flags are constructors that start work, discovery functions accepting
network/secrets/process handles, activation persisting `Active` before wiring
succeeds, and shutdown paths that drop a handle without awaiting owned work.

```bash
rg -n "discover|install|activate|deactivate|remove" \
  crates/ironclaw_extensions crates/ironclaw_first_party_extensions \
  crates/ironclaw_reborn_composition crates/ironclaw_product_adapters
```
