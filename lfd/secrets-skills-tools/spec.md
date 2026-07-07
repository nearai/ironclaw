# Spec: secrets-skills-tools minimal slice

Profile: `secrets_skills_tools`.

This slice hardens the existing secret path under skill/tool traffic: setup stores scoped secret material, authorization decides whether use is allowed, host-runtime stages and injects material into approved egress, and every model-visible surface is redacted.

## Stage 0

Before using the eval, make the profile execute every dev case with `status: "ran"` and keep these suites green:

- `cargo test -p ironclaw_secrets`
- `cargo test -p ironclaw_authorization`
- `cargo test -p ironclaw_host_runtime`
- `cargo test --test skill_credential_injection`
- `cargo test --features integration --test secret_injection`
- existing lease-expiry parking coverage

## Profile Schema

Each case uses `setup.profile_extra`:

- `scenario`: `happy_path`, `setup_naming`, `authorization_denial`, `manual_header_denial`, `wrong_host_denial`, `approval_resume`, `lease_expiry_resume`, `redaction_revocation`, `mcp_handshake_strip`, or `redirect_no_reinject`.
- `origin`: `skill`, `tool`, `slack_dm`, `webui`, or `mcp`.
- `actor`: scoped tenant/user/agent ids.
- `extension_name`: user-facing setup identity.
- `credential_name`: backend secret identity for storage, injection, and resume.
- `capability_id`: stable invocation id used by state queries.
- `declaration`: tool name, allowed hosts/paths, approved credential target, and required/optional behavior.
- `authorization`: expected decision and resume behavior.
- `lease`: normal, expires-before-egress, revoked-before-invocation, or rotated-before-invocation.
- `egress`: fake-provider request and expected credential visibility.
- `attack`: manual auth header, wrong host, output secret echo, redirect, or MCP handshake variations.

## Required State Queries

Every case includes state queries. The profile must answer them from persisted state or pinned recorders, not profile-local variables:

- `secret_meta`: `{exists, encrypted_at_rest, raw_value_visible, material_visible_to_model, credential_name, extension_name, setup_route_extension_name}`
- `credential_audit`: `{authorized, denied, approval_required, approval_resolved, denial_category, audit_events, secret_handle_visible}`
- `egress_capture`: `{network_attempts, credential_seen, injected_target, allowed_host, raw_secret_egress, raw_secret_in_runtime_args, stale_cache_used, redirected_credential_seen}`
- `lease_state`: `{created, consumed_once, expired, parked, resumed, revoked, reissued}`
- `redaction_scan`: `{secret_scan_hits, raw_secret_in_replies, raw_secret_in_events, raw_secret_in_tool_outputs, redaction_marker_seen}`
- `setup_route`: `{used_extension_name, used_credential_name_for_storage, routed_by_credential_name}`
- `run_state`: `{parked, resumed, terminal_status, denial_category}`

## Minimal Eval Slice

Dev has 10 cases: four success/setup paths, three denial paths, approval resume, lease-expiry resume, and revocation/redaction. Holdout has 3 off-repo cases with different providers plus MCP handshake stripping and redirect no-reinject behavior.

Contracts are two-sided: required state/gate/egress evidence plus forbidden leaks, raw secret egress, stale cache use, wrong-host egress, setup route conflation, and manual auth bypasses.
