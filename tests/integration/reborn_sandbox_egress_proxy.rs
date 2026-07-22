//! Docker-real integration test: the sandboxed shell's egress allowlist
//! proxy enforces the allowlist end-to-end through the PRODUCTION
//! composition path (Phase C Task 3 of
//! `docs/plans/2026-07-21-persistent-sandbox-container-design.md`).
//!
//! Drives `ironclaw_reborn_composition::tenant_sandbox_process_binding` —
//! the exact function `build_local_runtime` calls to assemble the
//! `TenantSandbox` process-port binding for the sandboxed profile — with no
//! `IRONCLAW_SANDBOX_HTTP_PROXY[_PORT]` env set, so it spawns its own
//! default `EgressAllowlistProxy` (Phase C Tasks 1-2, already landed) the
//! same way an unconfigured production deployment would. A shell command
//! run through the resulting `TenantSandboxProcessPort` then proves the
//! proxy actually mediates egress: an allowlisted host succeeds, a
//! non-allowlisted host is blocked with a `403` from the proxy.
//!
//! Requires a reachable Docker daemon AND a locally-built sandbox worker
//! image. Neither is available on a typical dev machine (this worktree has
//! no Docker) — the test is authored to run for real in CI/hosted lanes
//! that have both, and skips cleanly (a visible `SKIP: ...` line, never a
//! silent pass) everywhere else, per `tests/integration/CLAUDE.md`.
//!
//! Task 7 (`docs/plans/...` Phase C) extends this SAME file with the
//! secret-lease-daemon Docker-real test, reusing this file's
//! `#[path] mod docker_gate;`.

#[path = "support/docker_gate.rs"]
mod docker_gate;

use std::collections::HashMap;

use ironclaw_host_api::{AgentId, InvocationId, ProjectId, ResourceScope, TenantId, UserId};
use ironclaw_host_runtime::{CommandExecutionRequest, RuntimeProcessPort};
use ironclaw_reborn_composition::{RebornRuntimeProcessBinding, tenant_sandbox_process_binding};

fn test_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("sandbox-egress-proxy-tenant").expect("valid tenant id"),
        user_id: UserId::new("sandbox-egress-proxy-user").expect("valid user id"),
        agent_id: Some(AgentId::new("agent").expect("valid agent id")),
        project_id: Some(ProjectId::new("project").expect("valid project id")),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

#[tokio::test]
async fn sandbox_egress_proxy_enforces_allowlist_through_composition() {
    if !docker_gate::docker_available() {
        eprintln!(
            "SKIP: no docker daemon reachable — sandbox_egress_proxy_enforces_allowlist_through_composition requires a real Docker daemon (CI/hosted Docker lane only)"
        );
        return;
    }
    let image = docker_gate::configured_sandbox_image();
    if !docker_gate::docker_image_available(&image) {
        eprintln!(
            "SKIP: sandbox worker image {image:?} is not built locally — sandbox_egress_proxy_enforces_allowlist_through_composition requires a locally-built ironclaw-worker image (CI/hosted Docker lane only)"
        );
        return;
    }

    let workspace_root = tempfile::tempdir().expect("tempdir for sandbox workspace root");

    // Production composition path: `default_broker_port: None` and no
    // IRONCLAW_SANDBOX_HTTP_PROXY[_PORT] env set means
    // `tenant_sandbox_process_binding` spawns its own default
    // `EgressAllowlistProxy` (the same call `build_local_runtime` makes for
    // the sandboxed profile) and points the container's `http_proxy`/
    // `https_proxy` env at it via the Docker host-gateway address.
    let binding = tenant_sandbox_process_binding(workspace_root.path().to_path_buf(), None)
        .await
        .expect("real docker connect + default egress proxy spawn should succeed");
    assert!(
        binding.egress_proxy.is_some(),
        "no proxy env was set, so tenant_sandbox_process_binding should have spawned and \
         returned ownership of its own default egress proxy"
    );

    let process_port = match binding.binding {
        RebornRuntimeProcessBinding::TenantSandbox { process_port } => process_port,
        RebornRuntimeProcessBinding::None => {
            panic!("tenant_sandbox_process_binding must return a TenantSandbox binding")
        }
    };

    let scope = test_scope();

    // Allowed: pypi.org is in DEFAULT_SANDBOX_ALLOWED_DOMAINS
    // (network_allowlist.rs) — curl -f fails (nonzero exit) on any HTTP
    // error or connection failure, so a 0 exit here proves the proxy let
    // the CONNECT tunnel through and the request completed for real.
    let allowed = process_port
        .run_command(CommandExecutionRequest {
            scope: scope.clone(),
            mounts: None,
            command: "curl -sS -f -o /dev/null https://pypi.org".to_string(),
            workdir: None,
            timeout_secs: Some(30),
            extra_env: HashMap::new(),
            output_limit_bytes: None,
            background: false,
        })
        .await
        .expect("allowed-host curl should complete");
    assert_eq!(
        allowed.exit_code, 0,
        "curl to the allowlisted host pypi.org should succeed through the egress proxy: {}",
        allowed.output
    );

    // Denied: example.com is NOT in DEFAULT_SANDBOX_ALLOWED_DOMAINS. The
    // proxy replies `403 Forbidden` to the CONNECT request itself (before
    // any TLS handshake with the origin), which curl surfaces as exit 56
    // ("CONNECT tunnel failed, response 403") — capture stderr into the
    // recorded output so the 403 signal is directly assertable, not just an
    // opaque nonzero exit that could also mean a network hiccup.
    let denied = process_port
        .run_command(CommandExecutionRequest {
            scope: scope.clone(),
            mounts: None,
            command: "curl -sS -o /dev/null https://example.com 2>&1".to_string(),
            workdir: None,
            timeout_secs: Some(30),
            extra_env: HashMap::new(),
            output_limit_bytes: None,
            background: false,
        })
        .await
        .expect(
            "denied-host curl should complete (proxy denial surfaces as a nonzero exit code, \
             not a transport error)",
        );
    assert_ne!(
        denied.exit_code, 0,
        "curl to the non-allowlisted host example.com must be blocked by the egress proxy: {}",
        denied.output
    );
    assert!(
        denied.output.contains("403"),
        "expected curl to report the egress proxy's 403 Forbidden CONNECT denial; got: {}",
        denied.output
    );

    // E1 bypass assertion (the arbiter for Task 2's amended network
    // topology — REQUIRED, per
    // `docs/plans/2026-07-21-persistent-sandbox-container-plan.md` Task 3).
    // Clear every proxy env var the container was given and dial a
    // non-allowlisted host DIRECTLY. If the container still has a route to
    // the internet (e.g. it is still on Docker's default bridge, which NATs
    // out), this connects — proving the proxy is merely advisory, not the
    // only way out. On the pinned internal `internal: true` network
    // (E1's fix — no default route off-host), this must time out /
    // fail to connect, with no help from the proxy at all.
    let bypass_hostname = process_port
        .run_command(CommandExecutionRequest {
            scope: scope.clone(),
            mounts: None,
            command: "env -u http_proxy -u https_proxy -u HTTP_PROXY -u HTTPS_PROXY \
                      -u IRONCLAW_REBORN_HTTP_PROXY curl -sf --max-time 5 https://example.com"
                .to_string(),
            workdir: None,
            timeout_secs: Some(30),
            extra_env: HashMap::new(),
            output_limit_bytes: None,
            background: false,
        })
        .await
        .expect("bypass-attempt curl should complete (as a failure, not a transport error)");
    assert_ne!(
        bypass_hostname.exit_code, 0,
        "with proxy env cleared, a direct dial to a non-allowlisted host must fail to connect — \
         a success here means the container has a route off-host that skips the proxy \
         entirely (E1 is broken, egress enforcement is advisory only): {}",
        bypass_hostname.output
    );

    // Same bypass attempt against a raw IP literal, to rule out a DNS-only
    // enforcement mechanism (e.g. a container-local resolver override) that
    // would leave a route-level bypass open for anything not resolved by
    // name.
    let bypass_raw_ip = process_port
        .run_command(CommandExecutionRequest {
            scope: scope.clone(),
            mounts: None,
            command: "env -u http_proxy -u https_proxy -u HTTP_PROXY -u HTTPS_PROXY \
                      -u IRONCLAW_REBORN_HTTP_PROXY curl -sf --max-time 5 https://1.1.1.1"
                .to_string(),
            workdir: None,
            timeout_secs: Some(30),
            extra_env: HashMap::new(),
            output_limit_bytes: None,
            background: false,
        })
        .await
        .expect("bypass-attempt curl (raw IP) should complete (as a failure)");
    assert_ne!(
        bypass_raw_ip.exit_code, 0,
        "with proxy env cleared, a direct dial to a non-allowlisted raw IP must fail to \
         connect — DNS-only enforcement would let this through even though E1 blocks named \
         hosts: {}",
        bypass_raw_ip.output
    );

    // E2 hardening #1: a private-IP target reached THROUGH the proxy (env
    // left intact) must be refused — no SSRF to the dind host's cloud
    // metadata endpoint via the allowlist proxy.
    let metadata_ssrf = process_port
        .run_command(CommandExecutionRequest {
            scope,
            mounts: None,
            command: "curl -sf --max-time 5 http://169.254.169.254/latest/meta-data/".to_string(),
            workdir: None,
            timeout_secs: Some(30),
            extra_env: HashMap::new(),
            output_limit_bytes: None,
            background: false,
        })
        .await
        .expect("metadata-endpoint curl should complete (as a failure)");
    assert_ne!(
        metadata_ssrf.exit_code, 0,
        "the egress proxy must refuse a private-IP target (169.254.169.254, the cloud \
         metadata address) even through the allowlist path: {}",
        metadata_ssrf.output
    );
}
