# Retained Legacy Identifiers

Issue: #6552

The default workspace, crates, Rust types, test suites, scripts, and active
documentation use the IronClaw name. A small set of older identifiers remains
intentionally stable because it is stored on disk, configured outside this
repository, or consumed as a wire contract.

Do not copy these identifiers into new APIs or internal names. Removing or
changing one requires an explicit compatibility migration and rollback plan.

## Configuration and deployment aliases

- `IRONCLAW_REBORN_*` environment variables remain aliases for their
  `IRONCLAW_*` equivalents.
- `REBORN_TOOL_DISCLOSURE` and `REBORN_COLLAPSE_REPEATED_FAILURES` remain
  fallback aliases for the corresponding `IRONCLAW_*` variables.
- Existing `~/.ironclaw/reborn` and `/data/ironclaw-reborn` homes are adopted
  in place when no canonical IronClaw home exists.
- Existing `com.ironclaw.reborn` and `ironclaw-reborn.service` service
  definitions remain manageable by the CLI.
- The `reborn-live-canary-pr` GitHub environment and old live-QA repository
  variable names remain fallback inputs until those external settings migrate.
- The external `nearai/benchmarks` workflow still accepts
  `ironclaw-reborn` as a compatibility framework identifier.

## HTTP and extension wire contracts

- `/api/reborn/product-auth/*` remains an alias of `/api/product-auth/*` so
  existing OAuth redirect registrations continue to work.
- `reborn.extension_manifest.v1`, `.v2`, and `.v3` remain the extension
  manifest schema identifiers.
- `ironclaw.reborn.onboarding/v1` remains the persisted onboarding marker
  schema.
- `reborn/v1/*` remains the authenticated-encryption domain separator family.

## Durable storage and identity

Existing installations may contain `reborn-local-dev.db`,
`.reborn-local-dev-secrets-master-key`, the `reborn-cli` tenant/agent
identities, `/tenant-shared/reborn-projects`,
`/tenant-shared/reborn-identity`, and older bundled-skill marker files. These
are durable lookup keys rather than current product names.

Run-profile and loop-driver identifiers such as `reborn-planned-default`,
`reborn:planned-default`, and `reborn:text-only-model-reply` may be present in
persisted turn or run records and therefore remain readable.

## Compatibility rule

New code writes canonical IronClaw configuration and uses canonical routes,
artifact names, Docker paths, and observability targets. Compatibility aliases
must converge on the same handlers and policy boundaries; they must not create
a second behavior path.
