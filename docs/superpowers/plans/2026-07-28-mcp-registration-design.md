# User-Registered Hosted MCP Servers — Implementation Contract

## Status and scope

This document describes the implemented v1 architecture for registering a
tenant-owned, hosted MCP endpoint through the existing extension lifecycle.
It supersedes the earlier pending-core, membership-lease, and atomic
admission/finalization design. Those mechanisms are intentionally **not** part
of this implementation.

The feature accepts a custom server definition, makes that definition durable
and tenant-visible, and uses the ordinary install/authenticate/activate/invoke/
remove lifecycle to make its discovered tools usable. It supports no-auth,
manual bearer tokens, and OAuth. The WebUI supplies the custom-MCP modal and
then returns to the established extension lifecycle UX.

## Current source of truth

The relevant implementation is:

- `ironclaw_host_api`: `RegisterHostedMcpRequest`,
  `HostedMcpAuthSelection`, and `ExtensionRegisterHostedMcp`.
- `ironclaw_extension_host`: `HostedMcpPreparationService`, hosted-MCP
  admission/discovery, and the existing lifecycle manager.
- `ironclaw_extensions`: immutable package-definition admission,
  `PreparationRequirement`, and `PackageDefinitionRetention`.
- `ironclaw_auth` / `ironclaw_product` / secret-backed runtime ports: bearer
  account handling, OAuth recipe/DCR/profile admission, credential staging,
  continuation, and injection.
- `ironclaw_webui`: the product-surface adapter and extension UI.

### Design-to-code reconciliation

| Earlier proposal | Implemented behavior |
|---|---|
| Atomic hidden-core admission, members, leases, and orphan recovery | Not implemented. A package definition is admitted immutably with the store CAS; ordinary installation follows as a separate lifecycle operation. |
| A new durable `PendingPreparation` aggregate state | Not implemented as a new aggregate state. The existing `PreparationRequirement::{Ready, Required}` controls first-install preparation. |
| Atomic persistence of definition plus first catalog | Not implemented. Definition admission is atomic; discovery/finalization occurs during the later install preparation. |
| A special hosted-MCP `setup_needed` state | Not implemented. The existing public lifecycle readiness/setup response is used. |
| Refresh, drift comparison, filtering, partial publication, catalog history | Excluded from v1. The prepared catalog is used until the normal lifecycle changes it. |

## Core data and ownership

`RegisterHostedMcpRequest` contains a desired package id/name, an HTTPS endpoint,
and a closed auth selection: `NoAuth`, `Bearer`, or `OAuth`. The backend contract
retains an optional operator-managed OAuth client-profile id as a future
composition seam, but v1 does not expose it in the WebUI because production does
not yet compose a profile registry. The ordinary WebUI OAuth choice therefore
uses standards discovery and DCR. The request never carries an account secret
or OAuth client secret.

The host canonicalizes and validates the endpoint, derives the hosted extension
id, builds a user-registered manifest with:

```text
initial_preparation = Required
definition_retention = RetainInCatalog
```

and calls `ExtensionInstallationStorePort::admit_package_definition`. That is a
single immutable-definition CAS: a byte/semantic exact retry succeeds; a
different definition for the same package id is rejected. `RetainInCatalog`
keeps the tenant definition available after its final ordinary installation is
removed, enabling a later install/retry without re-registering it.

This operation deliberately does **not** create a hidden lifecycle core, add
member rows, or make an installation atomic with catalog discovery. The public
registration action admits the definition first and then immediately invokes
the canonical install/activation path; these are intentionally separate steps.
If that install is not attempted, cannot complete, or is later removed, the
retained definition is registered-but-uninstalled and an ordinary install retry
is the recovery path. The existing installation service owns per-user
installation, activation, and removal.

Tenant visibility is definition/catalog visibility, not tool authority. A user
must still use the existing install path and meet its own credential readiness
before tools become active/callable for that user. Agent-driven
`extension_install` takes the same lifecycle path as UI installation; it does
not receive a parallel custom-MCP implementation.

## Lifecycle flow

```text
WebUI modal / product command
  -> BoundProductSurface adapter
  -> LifecycleProductService::ExtensionRegisterHostedMcp
  -> HostedMcpPreparationService::register
  -> immutable definition CAS + available catalog entry
  -> canonical extension install (a distinct, non-atomic next step)
  -> Required preparation (credentials, discovery, safety, finalization)
  -> existing activation / capability publication
  -> existing invocation and removal
```

`HostedMcpPreparationService` is an orchestration seam, not a second generic
lifecycle. It loads the registered definition, verifies that the caller may
operate the installation, delegates credential readiness/staging to existing
auth/runtime ports, runs hosted-MCP discovery and catalog safety, finalizes the
manifest through the store, refreshes the available catalog, and synchronizes
the lifecycle package. The generic lifecycle sees only whether preparation is
required and whether it has completed.

```text
registered definition
       |
       +-- no install --> visible tenant catalog item; no active tools
       |
       +-- install --> credentials ready? -- no --> existing setup response
                          |
                         yes
                          v
                 discover + safety + finalize
                          v
                 ordinary activation and invocation
```

## Authentication and secrets

Authentication policy belongs to `ironclaw_auth`, product-auth services,
secret-backed account storage, and mediated runtime egress—not the UI modal or
the extension manifest as raw secret material.

| Registration choice | Setup and runtime behavior |
|---|---|
| NoAuth | Discovery and invocation proceed through policy-mediated egress without credential injection. |
| Bearer | Existing manual-token account setup stores the token in the secret-backed account path; staged runtime credentials inject the bearer header. Missing/rejected credentials return the existing setup/readiness response. |
| OAuth | A challenge triggers protected-resource and authorization-server metadata admission, then existing OAuth recipe/DCR/PKCE/continuation machinery. The admitted protected-resource metadata URL is retained exactly for DCR; it is not reconstructed from the MCP endpoint. The resulting account is selected and injected through the same runtime boundary. |

The hosted fixture and integration scenarios assert redacted request facts;
they do not retain bearer values. A tenant catalog definition is shared for
visibility, while each user’s account readiness remains scoped to that user.

## Discovery, safety, and v1 exclusions

Preparation uses the real mediated MCP client and policy-mediated egress. It
discovers the remote catalog, applies the composed catalog safety policy, and
materializes accepted tool descriptors into the finalized manifest. Failure
does not invent a partial catalog or a new lifecycle enum; it uses existing
error/readiness behavior.

V1 intentionally excludes:

- schema drift refresh, tool-catalog refresh, and visibility reconciliation;
- background/manual refresh, polling, notifications, history, or quarantine;
- partial publication, truncation-as-success, and a new `setup_needed` enum;
- standalone skills and a generic multi-runtime preparation framework.

Future packaged WASM or skill-bearing extensions can reuse the lifecycle shape
only when a second concrete preparation implementation justifies extracting a
shared abstraction. Linear-style scenarios remain ordinary lifecycle clients:

```text
Linear (OAuth) -> register definition -> existing OAuth setup -> install -> invoke
No-auth server  -> register definition -> install -> invoke
Future WASM     -> its own concrete preparation, if/when it exists -> activate
Future skills   -> no hosted-MCP shortcut; retain their established lifecycle
```

## Patterns used deliberately

- The WebUI/product boundary is an
  [Adapter](https://refactoring.guru/design-patterns/adapter): it converts HTTP
  input and output while retaining authorization and domain behavior below.
- `HostedMcpPreparationService` is a narrow concrete
  [Facade](https://refactoring.guru/design-patterns/facade) over existing
  installation, auth, discovery, safety, and catalog services. It does not
  hide a new subsystem or duplicate their policy.
- `PreparationRequirement` provides the small variation point in the existing
  lifecycle; the concrete preparation service resembles a constrained
  [Strategy](https://refactoring.guru/design-patterns/strategy), but no
  speculative registry is introduced for one implementation.
- The lifecycle supplies the stable sequencing while hosted preparation fills
  one step, analogous to
  [Template Method](https://refactoring.guru/design-patterns/template-method).
  This describes control flow only; it is not a mandate to create an inheritance
  hierarchy.

These names are explanatory, not a reason to cargo-cult pattern classes,
registries, or generic interfaces ahead of a second real use case.

## Verification and operational evidence

The hermetic hosted integration suite uses a dedicated Streamable-HTTP fixture
and exercises no-auth registration, idempotent definition retry/conflict,
real lifecycle preparation/activation/invocation, bearer challenge behavior,
and OAuth metadata admission followed by the existing callback continuation,
account persistence, activation, credential injection, and tool invocation. A
focused generic-auth regression proves that DCR reuses a non-well-known
challenge-advertised metadata URL. The suite also replays a sanitized Microsoft
Release Communications trace. The live Microsoft smoke is explicitly opt-in
because a vendor endpoint and public network are not a deterministic PR
contract.

Required evidence for a change in this area:

```text
cargo test -p ironclaw_reborn_integration_tests \
  --test reborn_integration_hosted_mcp_registration
pytest tests/e2e/scenarios/test_reborn_webui_v2_custom_mcp.py
```

Run the live smoke only with its explicit environment opt-in. PRs must explain
which deterministic journey was run, whether the live smoke was intentionally
skipped, and how noauth/bearer/OAuth plus tenant visibility were covered.

## Compatibility and rollback

Existing package records default to `PreparationRequirement::Ready`; hosted
definitions explicitly require preparation. There is no wire replacement for
existing installation state and no raw credential migration. Removing the UI
entry point or disabling registration stops new definitions while existing
ordinary lifecycle data remains intact. Removal follows the existing lifecycle;
retained registered definitions can be installed again. A rollback therefore
preserves data compatibility, but operators should decide separately whether to
hide retained custom definitions from the catalog.
