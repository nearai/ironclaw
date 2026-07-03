# ironclaw_process_sandbox guardrails

- Own the Reborn process sandbox compatibility lane for arbitrary commands, generated code, repo-local code, and user-installed CLIs.
- Accept only typed `SandboxProcessPlan` input. Do not accept raw Docker flags, raw host paths, host environment inheritance, or raw secret material from plan JSON.
- Keep physical Docker mount roots in trusted executor configuration, never in `ProcessExecutionRequest.input`.
- Treat install and credentialed run phases separately: install may write scoped tool/cache state with no secrets; credentialed run uses brokered secrets and read-only tool/cache state.
- Secret values must stay inside broker/lease seams and redaction helpers. Docker args, process output, errors, and debug data must not contain secret material.
- Do not stretch `ironclaw_scripts`; this crate is for dynamic sandbox process execution behind `ProcessExecutor`.
