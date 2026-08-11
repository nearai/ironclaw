# Hosted Volume Railway Shell Override Design

## Status and scope

This is a release-branch compatibility patch. It does not replace the existing
Reborn profile model or remove either sandbox-specific profile. It adds one
operator-controlled alias that lets `hosted-single-tenant-volume` reuse the
already-shipped Railway sandbox shell path without changing that profile's
durable storage paths.

The new operator variable is:

```text
IRONCLAW_REBORN_ENABLE_RAILWAY_SANDBOX_SHELL=true
```

It uses the CLI's strict boolean grammar: `1` and `true` enable the override;
`0` and `false` disable it; an empty, non-UTF-8, or unrecognized value fails
startup when the configured profile is `hosted-single-tenant-volume`.

## Activation contract

| Configured profile | Override | Shell backend | Capability guidance | Storage paths |
| --- | --- | --- | --- | --- |
| `hosted-single-tenant-volume` | unset or false | none | no shell guidance | unchanged base-volume paths |
| `hosted-single-tenant-volume` | true | Railway user sandbox | generic user-sandbox plus Railway guidance | unchanged base-volume paths |
| `hosted-single-tenant-volume-sandboxed-railway` | any value | Railway user sandbox | generic user-sandbox plus Railway guidance | existing dedicated-profile behavior |
| `hosted-single-tenant-volume-sandboxed` | any value | local Docker user sandbox | generic user-sandbox guidance only | existing dedicated-profile behavior |
| every other profile | any value | unchanged | no Railway guidance | unchanged |

The override is read only for the base volume profile. It cannot enable shell
for production, local development, hosted PostgreSQL, migration dry-run, or any
other profile. Existing dedicated sandbox profile names remain accepted for
backward compatibility.

## Runtime and storage selection

Boot keeps two decisions distinct:

- The configured `RebornProfile` continues to select the Reborn home, local
  runtime storage root, workspace root, configuration, logging, and operator
  diagnostics.
- An effective composition profile selects runtime policy, the Railway
  `UserSandboxProcessPort`, and provider-specific shell guidance.

When the configured profile is `hosted-single-tenant-volume` and the override
is true, its effective composition profile is
`hosted-single-tenant-volume-sandboxed-railway`. The CLI builds the existing
Railway process binding and retains all existing fail-closed validation for the
Railway project, environment, token, CLI path, idle timeout, and worker image.
There is no Docker or unsandboxed fallback.

The original configured profile is still passed to storage-root resolution.
Therefore enabling the override cannot move, duplicate, or adopt application
state into the dedicated sandbox profile's legacy storage subdirectory.

## Shell capability guidance

Selecting the Railway effective profile retains `builtin.shell` because its
resolved process backend is `UserSandbox`. The existing generic user-sandbox
description remains in place. Composition then appends Railway-only guidance
to both the capability descriptor and its manifest description so the model
sees one consistent contract:

> Shell commands run in fresh workers inside a Railway Sandbox. Persist files
> only under `/workspace`; processes, environment changes, working-directory
> changes, and system packages do not survive between calls. Outbound internet
> uses Railway NAT. Railway credentials and host-control tooling are not
> available inside the worker. The sandbox `/workspace` is separate from the
> IronClaw workspace where users save and manage files; files do not
> automatically appear in both locations.

The text lives in a prompt file owned by `ironclaw_extension_support`, the
sanctioned home for concrete first-party vendor-specific userland data, and is
exported as inert guidance data. The CLI attaches that data to
`RebornHostBindings` only when it constructs the existing Railway process
binding. Composition consumes the opaque optional guidance input and must not
ship prompt content or branch behavior on its profile telemetry label. A
provider-neutral `ironclaw_host_runtime` helper appends supplied shell guidance
while preserving descriptor/manifest parity, so the kernel does not name
Railway. Local Docker and other `UserSandbox` consumers carry no supplemental
guidance and continue to receive only the generic guidance.

## Failure behavior

- Invalid override values fail before runtime assembly for the base volume
  profile.
- Missing or ambiguous Railway credentials continue to fail startup without
  exposing credential values.
- Missing project/environment identifiers, invalid timeouts, and invalid
  worker-image configuration continue to use the existing Railway errors.
- A Railway process binding paired with a non-`UserSandbox` runtime policy
  remains a composition error.

## Test strategy

Implementation starts with focused failing tests that establish:

1. Base volume plus override true retains base storage paths, resolves
   `UserSandbox`, and builds the Railway binding.
2. Base volume with the override absent or false remains processless.
3. A malformed override fails only when the base volume profile consumes it.
4. Other profiles do not gain Railway shell behavior from the variable.
5. Both the dedicated Railway profile and the alias path expose
   `builtin.shell` with generic and Railway-specific guidance.
6. Local Docker sandbox shell exposes generic guidance but contains no Railway
   wording.
7. The Railway guidance includes the explicit distinction between sandbox
   `/workspace` and the IronClaw user workspace.

Focused CLI and composition tests run first, followed by formatting,
architecture tests if the composition input seam changes, and the narrowest
existing sandbox-shell integration coverage that does not require live Railway
provisioning. Live Railway provisioning remains an operator QA step because it
requires provider credentials and preview availability.

## Compatibility and rollback

The default remains processless. Existing profile strings, serialized config,
and dedicated sandbox deployments are unchanged. Rollback is operationally
immediate by unsetting the variable and restarting; the base profile resumes
without shell and continues using the same durable paths. The code can be
removed after the release branch no longer needs the compatibility alias.
