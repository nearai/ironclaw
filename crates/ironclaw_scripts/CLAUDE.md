# ironclaw_scripts guardrails

- Own the Reborn script runtime lane: manifest-derived script execution requests, backend abstraction, Docker CLI backend, output parsing, and script resource accounting.
- Keep script backend requests normalized and manifest-derived. Do not expose raw Docker flags, host paths, host environment variables, caller-supplied command fragments, or ad-hoc network access.
- Runtime HTTP, secret injection, network policy, and approval/authorization must be mediated by host-runtime services, not implemented in script backends.
- If a prepared resource reservation is provided, reconcile/release that reservation exactly once instead of reserving again.
- Bound stdout/stderr and wall-clock behavior through configuration; runtime-visible errors must be stable and sanitized.
- Keep script-specific execution semantics here. Extension parsing belongs in `ironclaw_extensions`, dispatch selection in `ironclaw_dispatcher`, process lifecycle in `ironclaw_processes`, and product workflow outside this crate.
