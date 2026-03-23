# Summary

This PR adds the first ANP integration foundation to ironclaw.

The scope is intentionally narrow:

- add a persistent instance `did:key`
- add protected identity inspection APIs
- add CLI DID inspection commands
- add a minimal ANP agent description preview

This PR does **not** add public discovery, ANP messaging, `did:wba`, or behavior changes to existing chat/tool/channel flows.

# What is ANP

ANP, Agent Network Protocol, is an open protocol effort for agent interoperability. Its role is not to replace ironclaw internals, but to provide a standard path for:

- identity
- capability description
- discovery
- agent-to-agent communication

Relevant upstream references:

- [ANP repository](https://github.com/agent-network-protocol/AgentNetworkProtocol)
- [ANP README](https://github.com/agent-network-protocol/AgentNetworkProtocol/blob/main/README.md)
- [ANP technical white paper](https://github.com/agent-network-protocol/AgentNetworkProtocol/blob/main/01-agentnetworkprotocol-technical-white-paper.md)
- [ANP Agent Description specification](https://github.com/agent-network-protocol/AgentNetworkProtocol/blob/main/07-anp-agent-description-protocol-specification.md)

ANP is being developed as an open community protocol, with public specs, contribution docs, and companion implementation work such as [AgentConnect](https://github.com/agent-network-protocol/AgentConnect).

# Scope of this PR

This PR establishes an instance-level cryptographic identity layer for ironclaw.

More concretely, it adds:

- a persistent local instance DID
- DID document export
- CLI support for inspecting the DID
- protected gateway endpoints for identity metadata
- a local ANP agent description preview derived from current ironclaw capabilities

The goal is to create a low-risk and independently useful first step that future ANP PRs can build on.

# Why `did:key` first

ANP's longer-term public identity direction is compatible with `did:wba`, but `ironclaw` is currently a strongly local-first system:

- many instances do not have a stable domain
- many instances do not have a permanent HTTPS endpoint
- tunnel URLs are suitable for temporary ingress, not durable identity

For that reason, this PR uses:

- `did:key` as the local root identity

This gives each instance a stable, restart-safe identity immediately, without introducing public network assumptions into the first PR.

This PR does **not** claim that `did:key` is the final public ANP identity model for ironclaw. The expected follow-up path is:

- keep local root identity as `did:key`
- add a public alias identity later, with `did:wba` as the first officially supported public method

# Included

## 1. Persistent instance DID

Each ironclaw instance now gets a stable local `did:key` identity, persisted under the existing ironclaw base directory.

This identity survives restarts and does not depend on the database.

## 2. CLI DID inspection

New CLI commands:

```bash
ironclaw did show
ironclaw did document
```

`ironclaw status` also now shows the current instance DID and identity file path.

## 3. Protected identity APIs

This PR adds protected gateway endpoints for inspecting the local identity:

- `GET /api/identity`
- `GET /api/identity/did-document`
- `GET /api/identity/agent-description`

These remain behind the existing gateway auth token and are not public ANP routes.

## 4. ANP agent description preview

ironclaw can now generate a minimal ANP-compatible agent description preview derived from:

- the current instance DID
- the configured local agent name
- current local structured/natural-language interfaces

This is intentionally a preview, not a full discovery/publication implementation.

# What this PR does not do

To keep the first integration step small and reviewable, this PR does **not** include:

- `did:wba`
- public DID publication
- `/.well-known/agent-descriptions`
- ANP discovery
- ANP messaging / JSON-RPC transport
- DID-based trust store
- E2EE
- changes to existing chat/tool/channel semantics

# Notes for maintainers

This PR introduces **instance identity**, not workspace persona identity.

It is separate from prompt/persona files such as `IDENTITY.md`, `SOUL.md`, or `AGENTS.md`. Those files affect prompts and memory; this PR adds cryptographic identity for future agent interoperability.

This PR is also designed to fit ironclaw's current architecture:

- identity is stored under the existing local base directory
- identity metadata is exposed through the existing protected gateway model
- current session/channel behavior is left unchanged

# Validation

Docker-based TDD validation used for this PR:

```bash
docker-tests/run_rust_tests.sh
docker-tests/run_rust_tests.sh cargo test did --lib --no-default-features --features libsql
docker-tests/run_rust_tests.sh cargo test anp --lib --no-default-features --features libsql
docker-tests/run_rust_tests.sh cargo test --test ws_gateway_integration --no-default-features --features libsql
docker-tests/run_rust_tests.sh cargo test --test openai_compat_integration --no-default-features --features libsql
docker-tests/run_rust_tests.sh cargo test --test anp_identity_integration --no-default-features --features libsql
docker-tests/run_rust_tests.sh cargo fmt --all -- --check
```

Manual verification also confirmed:

- `cargo run -- --no-db did show`
- `cargo run -- --no-db did document`
- protected `/api/identity*` endpoints return expected data when called with gateway auth

# How to review

Recommended review order:

1. `src/did/`
2. `src/anp/`
3. CLI changes
4. gateway wiring
5. `src/main.rs`
6. tests and `docker-tests/`

Main review points:

- no existing channel behavior changed
- identity endpoints are protected by existing gateway auth
- instance DID remains stable across reloads
- this PR adds local identity foundation only, not public network behavior

# Follow-up PRs

Planned follow-up work after this foundation lands:

1. `did:wba` support and public identity publication
2. DID-based peer trust store / approval flow
3. ANP discovery endpoints
4. ANP messaging transport
5. richer ANP agent description export based on tool metadata
