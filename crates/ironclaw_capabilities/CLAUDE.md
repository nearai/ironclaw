# ironclaw_capabilities guardrails

- Own caller-facing `CapabilityHost` invoke/resume/spawn workflow.
- Use the neutral `CapabilityDispatcher` port; do not add a normal dependency on concrete `ironclaw_dispatcher` or runtime crates.
- `CapabilityHost` is the single caller-facing authority path for invoke/resume/spawn: host-runtime adapters, built-ins, custom packages, and external runtimes must enter through this workflow rather than adding parallel authorization/approval dispatch paths.
- Host authorization must use the trust-aware contract (`TrustAwareCapabilityDispatchAuthorizer`) with a policy-derived `TrustDecision`; do not wire production `CapabilityHost` with grant-only authorization that bypasses trust ceilings.
- Do not absorb process lifecycle/result APIs; those belong in `ironclaw_processes::ProcessHost`.
- Approval resume must validate and claim the matching fingerprinted lease before dispatch.
- Authorization denial or unsupported/failed obligations must fail before runtime dispatch, process start, or approval lease claim.
- Keep obligation handling behind a seam; built-in obligation implementations belong in later host-runtime/obligation slices.
