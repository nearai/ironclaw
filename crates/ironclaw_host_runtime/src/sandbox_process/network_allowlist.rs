//! Default egress allowlist for the sandboxed (`TenantSandbox`) shell
//! profile.
//!
//! IronClaw's sandboxed shell needs outbound network access for ordinary
//! package-manager workflows (`pip install`, `npm install`, `git clone`,
//! `curl` against a registry) without granting the container unrestricted
//! internet access. The model mirrors legacy IronClaw's sandbox: soft
//! enforcement through an HTTP(S) forward proxy that only permits requests
//! to an allowlist of known package-registry and source-hosting domains,
//! plus any extra domains an operator configures.
//!
//! This module owns the *domain list* — the set of hosts the sandboxed
//! profile's `builtin.shell` grant should carry in its
//! [`NetworkPolicy`](ironclaw_host_api::NetworkPolicy) `allowed_targets`. It
//! does not itself enforce anything; it is the policy input consumed by two
//! things that do: the CONNECT/forward proxy
//! (`ironclaw_host_runtime::sandbox_process::egress_proxy`), spawned and
//! bound via `crates/ironclaw_reborn_composition/src/sandbox_egress_proxy_task.rs`
//! and `sandbox_boot.rs`'s `with_sandbox_network_broker`, and the topological
//! guardrail that the container's Docker network is pinned `internal: true`
//! with no default route off the host, so the proxy is the container's only
//! path to the outside world.
//!
//! Ships unwired: neither of those two enforcers is on `main` yet, so nothing
//! reads this list in production today. It arrives first because both of them
//! take it as an input, and because a list of hostnames is reviewable on its
//! own in a way it will not be once it is buried in a proxy PR.
use ironclaw_common::env_helpers::env_or_override;
use ironclaw_host_api::NetworkPolicy;
use ironclaw_network::NetworkPolicyError;

/// Environment variable operators can set to add domains to the sandboxed
/// shell's egress allowlist, on top of [`DEFAULT_SANDBOX_ALLOWED_DOMAINS`].
/// Comma-separated hostnames (e.g. `example.com,*.internal.example.com`).
pub const SANDBOX_EXTRA_ALLOWED_DOMAINS_ENV: &str = "IRONCLAW_SANDBOX_EXTRA_ALLOWED_DOMAINS";

/// Environment variable operators can set to override
/// [`DEFAULT_SANDBOX_MAX_EGRESS_BYTES`] — the per-request egress volume cap
/// carried on the sandboxed shell's [`NetworkPolicy`]. Same
/// `env_or_override` precedence as [`SANDBOX_EXTRA_ALLOWED_DOMAINS_ENV`] and
/// `connect::DOCKER_HOST_ENV`.
pub const SANDBOX_MAX_EGRESS_BYTES_ENV: &str = "IRONCLAW_SANDBOX_MAX_EGRESS_BYTES";

/// Default per-request egress volume cap for the sandboxed shell profile, in
/// bytes: **2 GiB** (`2 * 1024 * 1024 * 1024`).
///
/// This is a product tradeoff between two failure modes: too low, and a
/// legitimate `cargo fetch`/`npm install`/`docker pull`-style dependency
/// resolve (which can pull single artifacts in the hundreds-of-MB range —
/// large ML/CUDA wheels, `node_modules` tarballs, monorepo shallow clones)
/// gets blocked and reported as if it were malicious; too high, and the cap
/// stops meaning anything. 2 GiB is generous enough to pass those ordinary
/// package-manager workloads (which sit one to two orders of magnitude
/// below it) while still bounding a single request far short of what a
/// deliberate bulk-exfiltration attempt would need to move — this is a
/// **per-request** ceiling (`NetworkPolicy::max_egress_bytes` is checked
/// against one request's `estimated_bytes` by `ironclaw_network`'s policy
/// enforcer), not a cumulative session budget, so it does not by itself
/// bound how much data a long-running sandboxed session moves in total
/// across many requests.
///
/// That per-request model holds today only because the sole consumer of
/// this constant is `PolicyNetworkHttpEgress`, a host-mediated HTTP client
/// that sees plaintext method/URL/headers/body and computes
/// `estimated_bytes` for a request before sending it
/// (`estimate_http_request_bytes`,
/// `crates/ironclaw_network/src/egress.rs`). It is **not established** that
/// the same model transfers to the CONNECT/forward proxy this list feeds
/// (see the module doc above): a CONNECT proxy tunnels opaque TLS bytes
/// after the handshake and never sees plaintext, so it cannot compute "one
/// request's size" the way `estimate_http_request_bytes` does — streamed
/// per-connection byte metering is the natural fit for that transport, not
/// discrete pre-flight request sizing. Whether `max_egress_bytes` should
/// mean "per request" or "cumulative per connection" once the proxy exists
/// is an open design question for that proxy-wiring PR to resolve, not
/// something this constant already answers. [`SANDBOX_MAX_EGRESS_BYTES_ENV`]
/// lets an operator tighten (or loosen) this for their own risk tolerance
/// and workload shape in the meantime.
///
/// Mirrors `MCP_NETWORK_EGRESS_LIMIT`'s shape
/// (`crates/ironclaw_extension_host/src/mcp.rs`, `activation_transaction.rs`)
/// — a named constant feeding `NetworkPolicy::max_egress_bytes` — but not
/// its value: that one is sized for MCP tool-call JSON responses (2 MiB) and
/// is not a template here; the sandboxed shell's legitimate payloads are
/// orders of magnitude larger.
// TODO(security): this value is sized and validated against
// `PolicyNetworkHttpEgress`'s per-request `estimated_bytes` check
// (`crates/ironclaw_network/src/egress.rs`). The CONNECT-proxy wiring PR
// (see module doc above) must explicitly decide streaming vs. cumulative
// byte metering for that transport before assuming `max_egress_bytes`
// carries the same "per request" meaning there — a proxy that reuses this
// constant without that decision is not enforcing what this doc comment
// describes.
pub const DEFAULT_SANDBOX_MAX_EGRESS_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Default egress allowlist for the sandboxed shell profile — the package
/// registries and source hosts ordinary `pip`/`npm`/`git`/`curl` workflows
/// need, mirroring legacy IronClaw's sandbox allowlist.
pub const DEFAULT_SANDBOX_ALLOWED_DOMAINS: &[&str] = &[
    // Rust
    "crates.io",
    "static.crates.io",
    "index.crates.io",
    // Node/npm
    "registry.npmjs.org",
    "nodejs.org",
    // Python
    "pypi.org",
    "files.pythonhosted.org",
    // Go
    "proxy.golang.org",
    // GitHub (source + release archives)
    "github.com",
    "raw.githubusercontent.com",
    "api.github.com",
    "codeload.github.com",
];

/// Reads [`SANDBOX_EXTRA_ALLOWED_DOMAINS_ENV`] and returns the operator's
/// configured extra domains, trimmed and with empty entries dropped. Returns
/// an empty `Vec` (never an error) when the variable is unset or empty — the
/// extra-domains hook itself is optional. Returns `Err` when a configured
/// entry fails [`ironclaw_network::parse_host_pattern`]'s hostname-shape
/// check (empty, bare `*`, or not a valid hostname / `*.`-wildcard) — see
/// that function's doc comment for why a typo here is not survivable.
///
/// Read through `env_or_override` rather than a bare `std::env::var`, so this
/// key honors the same runtime-override precedence as the sibling
/// `connect::DOCKER_HOST_ENV` — one env-reading convention across
/// `sandbox_process`, not two.
pub fn sandbox_extra_allowed_domains() -> Result<Vec<String>, NetworkPolicyError> {
    let Some(raw) = env_or_override(SANDBOX_EXTRA_ALLOWED_DOMAINS_ENV) else {
        return Ok(Vec::new());
    };
    raw.split(',')
        .map(str::trim)
        .filter(|domain| !domain.is_empty())
        .map(|domain| {
            ironclaw_network::parse_host_pattern(domain).map(|pattern| pattern.host_pattern)
        })
        .collect()
}

/// The full sandboxed-shell egress allowlist: [`DEFAULT_SANDBOX_ALLOWED_DOMAINS`]
/// plus any operator-configured extras from
/// [`sandbox_extra_allowed_domains`]. `Err` when an extra domain fails
/// hostname-shape validation — see [`sandbox_extra_allowed_domains`].
pub fn sandbox_allowed_domains() -> Result<Vec<String>, NetworkPolicyError> {
    Ok(DEFAULT_SANDBOX_ALLOWED_DOMAINS
        .iter()
        .map(|domain| (*domain).to_string())
        .chain(sandbox_extra_allowed_domains()?)
        .collect())
}

/// Reads [`SANDBOX_MAX_EGRESS_BYTES_ENV`] and returns the operator's
/// configured per-request egress cap, or [`DEFAULT_SANDBOX_MAX_EGRESS_BYTES`]
/// when unset. `Err` when the override is set but not a positive `u64` —
/// same fail-loud posture as [`sandbox_extra_allowed_domains`]: a malformed
/// override must refuse to start, not silently fall back to the default
/// while the operator believes their tighter value took effect.
pub fn sandbox_max_egress_bytes() -> Result<u64, NetworkPolicyError> {
    let Some(raw) = env_or_override(SANDBOX_MAX_EGRESS_BYTES_ENV) else {
        return Ok(DEFAULT_SANDBOX_MAX_EGRESS_BYTES);
    };
    ironclaw_network::parse_egress_limit(&raw)
}

/// [`sandbox_allowed_domains`], expressed as [`NetworkPolicy`] `allowed_targets`
/// — ready to carry on the sandboxed profile's `builtin.shell` grant so
/// `validate_network_policy_metadata` (which rejects an empty allowlist)
/// passes, and so the policy documents what the container is actually meant
/// to reach.
///
/// `Err` propagates a hostname-shape validation failure from
/// [`sandbox_allowed_domains`] rather than silently dropping the bad entry or
/// falling back to a narrowed allowlist — callers MUST treat this as a hard
/// boot-time failure (refuse to start), never as "log and continue with
/// whatever validated". A silently narrowed sandbox allowlist is invisible
/// until someone audits traffic; a boot failure is loud and immediate.
pub fn sandbox_network_policy() -> Result<NetworkPolicy, NetworkPolicyError> {
    Ok(NetworkPolicy {
        allowed_targets: sandbox_allowed_domains()?
            .into_iter()
            .map(|host_pattern| ironclaw_host_api::NetworkTargetPattern {
                scheme: None,
                host_pattern,
                port: None,
            })
            .collect(),
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(sandbox_max_egress_bytes()?),
    })
}

#[cfg(test)]
mod tests {
    use ironclaw_common::env_helpers::{lock_env, remove_runtime_env, set_runtime_env};

    use super::*;

    #[test]
    fn default_allowlist_covers_the_major_package_registries_and_github() {
        for expected in [
            "crates.io",
            "registry.npmjs.org",
            "pypi.org",
            "files.pythonhosted.org",
            "proxy.golang.org",
            "github.com",
            "raw.githubusercontent.com",
        ] {
            assert!(
                DEFAULT_SANDBOX_ALLOWED_DOMAINS.contains(&expected),
                "expected {expected} in the default sandbox allowlist"
            );
        }
    }

    #[test]
    fn sandbox_network_policy_is_non_empty_and_denies_private_ips() {
        // Guarded and reset like the env-mutating tests below: `sandbox_
        // network_policy` now reads the shared runtime-env overlay through
        // `sandbox_extra_allowed_domains`/`sandbox_max_egress_bytes`, so this
        // test must not race a sibling test's mid-flight override of either
        // env var (e.g. the deliberately-invalid `*` set by
        // `sandbox_network_policy_propagates_the_wildcard_rejection`).
        let _guard = lock_env();
        remove_runtime_env(SANDBOX_EXTRA_ALLOWED_DOMAINS_ENV);
        remove_runtime_env(SANDBOX_MAX_EGRESS_BYTES_ENV);

        let policy = sandbox_network_policy().unwrap();
        assert!(
            !policy.allowed_targets.is_empty(),
            "sandboxed shell network policy must not be the empty (deny-all) default"
        );
        assert!(policy.deny_private_ip_ranges);
        assert!(
            policy
                .allowed_targets
                .iter()
                .any(|target| target.host_pattern == "github.com")
        );
    }

    // The sandboxed shell is the one profile whose entire purpose is running
    // untrusted code; an absent egress cap there means a single request can
    // exfiltrate an unbounded amount of data through the (otherwise
    // allowlisted) proxy. `max_egress_bytes: None` is the repo-wide default
    // everywhere else, but not here.
    #[test]
    fn sandbox_network_policy_sets_a_concrete_egress_ceiling() {
        let _guard = lock_env();
        remove_runtime_env(SANDBOX_EXTRA_ALLOWED_DOMAINS_ENV);
        remove_runtime_env(SANDBOX_MAX_EGRESS_BYTES_ENV);

        let policy = sandbox_network_policy().unwrap();

        assert_eq!(
            policy.max_egress_bytes,
            Some(DEFAULT_SANDBOX_MAX_EGRESS_BYTES)
        );
    }

    #[test]
    fn sandbox_max_egress_bytes_env_override_is_honored() {
        let _guard = lock_env();
        set_runtime_env(SANDBOX_MAX_EGRESS_BYTES_ENV, "1048576");

        let result = sandbox_max_egress_bytes();

        remove_runtime_env(SANDBOX_MAX_EGRESS_BYTES_ENV);

        assert_eq!(result.unwrap(), 1_048_576);
    }

    #[test]
    fn sandbox_max_egress_bytes_env_rejects_zero_and_non_numeric() {
        let _guard = lock_env();

        set_runtime_env(SANDBOX_MAX_EGRESS_BYTES_ENV, "0");
        assert!(sandbox_max_egress_bytes().is_err());

        set_runtime_env(SANDBOX_MAX_EGRESS_BYTES_ENV, "not-a-number");
        assert!(sandbox_max_egress_bytes().is_err());

        remove_runtime_env(SANDBOX_MAX_EGRESS_BYTES_ENV);
    }

    #[test]
    fn extra_domains_env_absent_yields_no_extras() {
        let _guard = lock_env();
        remove_runtime_env(SANDBOX_EXTRA_ALLOWED_DOMAINS_ENV);

        assert_eq!(
            sandbox_extra_allowed_domains().unwrap(),
            Vec::<String>::new()
        );
    }

    // A bare `*` turns `host_matches_pattern` (ironclaw_network::policy) into
    // "match every host" — one operator typo would silently convert the
    // package-registry allowlist into allow-all egress for the one profile
    // whose entire purpose is holding untrusted code. This must hard-fail,
    // not silently drop the bad entry: a boot failure is loud, a silently
    // narrowed allowlist is invisible until someone audits traffic.
    #[test]
    fn extra_domains_env_rejects_bare_wildcard() {
        let _guard = lock_env();
        set_runtime_env(SANDBOX_EXTRA_ALLOWED_DOMAINS_ENV, "example.com,*");

        let result = sandbox_extra_allowed_domains();

        remove_runtime_env(SANDBOX_EXTRA_ALLOWED_DOMAINS_ENV);

        assert!(
            result.is_err(),
            "bare `*` in the extra-domains env must be rejected, not silently allowed"
        );
    }

    #[test]
    fn extra_domains_env_rejects_malformed_hostname() {
        let _guard = lock_env();
        set_runtime_env(SANDBOX_EXTRA_ALLOWED_DOMAINS_ENV, "not a host!");

        let result = sandbox_extra_allowed_domains();

        remove_runtime_env(SANDBOX_EXTRA_ALLOWED_DOMAINS_ENV);

        assert!(result.is_err());
    }

    #[test]
    fn sandbox_network_policy_propagates_the_wildcard_rejection() {
        let _guard = lock_env();
        set_runtime_env(SANDBOX_EXTRA_ALLOWED_DOMAINS_ENV, "*");

        let result = sandbox_network_policy();

        remove_runtime_env(SANDBOX_EXTRA_ALLOWED_DOMAINS_ENV);

        assert!(
            result.is_err(),
            "sandbox_network_policy must hard-fail (not fall back to a \
             narrowed policy) when the operator-supplied allowlist is invalid"
        );
    }

    #[test]
    fn extra_domains_env_is_parsed_and_merged_into_the_full_allowlist() {
        // The guard is held across the whole set/read/clear window: this
        // process's env is global, and the sibling `connect` tests mutate it
        // too. Raw `std::env::set_var` here would race them.
        let _guard = lock_env();
        set_runtime_env(
            SANDBOX_EXTRA_ALLOWED_DOMAINS_ENV,
            " example.internal , , *.corp.example.com",
        );

        let extras = sandbox_extra_allowed_domains().unwrap();
        // Asserted while the override is still set: `sandbox_allowed_domains`
        // must *append* to the defaults, not replace them. Clearing first
        // would make a replace-instead-of-chain regression invisible.
        let all = sandbox_allowed_domains().unwrap();

        remove_runtime_env(SANDBOX_EXTRA_ALLOWED_DOMAINS_ENV);

        assert_eq!(extras, vec!["example.internal", "*.corp.example.com"]);
        assert!(
            all.contains(&"crates.io".to_string()),
            "operator extras must not displace the defaults: {all:?}"
        );
        assert!(all.contains(&"example.internal".to_string()));
        assert!(all.contains(&"*.corp.example.com".to_string()));
    }
}
