# ironclaw_filesystem guardrails

- Own `RootFilesystem`, `ScopedFilesystem`, virtual-path persistence, and backend containment checks.
- Depend on `ironclaw_host_api`; do not depend on product, authorization, dispatcher, runtime, process, event, or extension workflow crates.
- Keep `HostPath` backend-internal and non-serializable.
- Reject traversal, mount escapes, unknown mounts, and permission mismatches fail-closed.
- New persistence behavior must preserve tenant/user virtual-path scoping.
