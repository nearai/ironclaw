# Native Extensions

## Provider Split Policy

Each provider lives in a self-contained subdir under `src/`. Subdirs MUST NOT import from siblings.

Split a provider into its own sibling crate (`ironclaw_native_extensions_<provider>`) when ANY of:
- Provider needs heavy external deps not shared by others (>5 MB additional binary size or >5s compile-time impact)
- Provider count exceeds ~10
- Provider has independent release cadence (for example, a vendored SDK with its own versioning)

Split is mechanical: copy subdir -> new crate -> update workspace + `reborn_dependency_boundaries.rs` -> adjust register fn name.
