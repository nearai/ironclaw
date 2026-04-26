# ironclaw_filesystem guardrails

- Own `RootFilesystem`, `ScopedFilesystem`, `CompositeRootFilesystem`, `FilesystemCatalog`, virtual-path persistence, backend placement metadata, and backend containment checks.
- Depend on `ironclaw_host_api`; do not depend on product, authorization, dispatcher, runtime, process, event, secrets, memory/search, or extension workflow crates.
- Keep `HostPath` backend-internal and non-serializable.
- Reject traversal, mount escapes, unknown mounts, duplicate exact backend roots, and permission mismatches fail-closed.
- Use longest virtual-prefix routing for composite backend selection and catalog placement.
- Catalog metadata describes placement and content/index policy; it does not grant runtime authority.
- Keep memory-specific path grammar and memory document repository adapters in `ironclaw_memory`, not this crate.
- Do not force structured/control-plane records through file byte APIs. Secrets, approvals, leases, process records, events, and search indexes should stay in typed service repositories unless exposed as deliberate file-shaped projections.
- New persistence behavior must preserve tenant/user virtual-path scoping.
