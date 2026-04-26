# ironclaw_events guardrails

- Own runtime/process event and control-plane audit sink contracts only.
- Runtime events and audit records are metadata-only; never include raw input, output, host paths, secrets, approval reasons, fingerprints, lease IDs, or backend detail strings.
- Sink failures are reported to callers, but dispatcher/resolver outcome policy remains in owning crates.
- Do not depend on workflow, runtime, process, resource, extension, or host-runtime crates.
- JSONL sinks must not overwrite, truncate, or append to unreadable/malformed existing logs.
