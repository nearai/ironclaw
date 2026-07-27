# ironclaw_process_sandbox guardrails

- Own the typed `SandboxProcessPlan` contract only: plan types and validation (`ValidatedSandboxProcessPlan`) for arbitrary commands, generated code, repo-local code, and user-installed CLIs.
- Accept only typed `SandboxProcessPlan` input. Do not accept raw Docker flags, raw host paths, host environment inheritance, or raw secret material from plan JSON.
- There is no production backend wired for this capability today — see `ironclaw_host_runtime`'s `process_executor` for the dispatch seam this crate's plans feed into. Do not add Docker mount-root or executor configuration here; that lives with whatever crate eventually wires a real backend.
- Treat install and credentialed run phases separately in the plan types: install may declare scoped tool/cache state with no secrets; credentialed run declares brokered secrets and read-only tool/cache state.
- Secret values must stay inside broker/lease seams and redaction helpers. Plan JSON, validation errors, and debug data must not contain secret material.
- Do not stretch `ironclaw_scripts`; this crate is plan-validation-only and does not itself execute anything.
