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
//! [`NetworkPolicy`](ironclaw_host_api::NetworkPolicy) `allowed_targets`, and
//! that a host-side allowlist proxy would enforce for real. It does not
//! itself enforce anything — enforcement is the transport/proxy wiring
//! (`RebornSandboxConfig::with_network_broker_proxy_url`, wired at
//! `crates/ironclaw_reborn_composition/src/sandbox_boot.rs`) plus, still
//! outstanding (see the `TODO(follow-up, not built here)` there), the
//! actual proxy server.
use ironclaw_host_api::{NetworkPolicy, NetworkTargetPattern};

/// Environment variable operators can set to add domains to the sandboxed
/// shell's egress allowlist, on top of [`DEFAULT_SANDBOX_ALLOWED_DOMAINS`].
/// Comma-separated hostnames (e.g. `example.com,*.internal.example.com`).
pub const SANDBOX_EXTRA_ALLOWED_DOMAINS_ENV: &str = "IRONCLAW_SANDBOX_EXTRA_ALLOWED_DOMAINS";

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
/// extra-domains hook is optional.
pub fn sandbox_extra_allowed_domains() -> Vec<String> {
    std::env::var(SANDBOX_EXTRA_ALLOWED_DOMAINS_ENV)
        .ok()
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|domain| !domain.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The full sandboxed-shell egress allowlist: [`DEFAULT_SANDBOX_ALLOWED_DOMAINS`]
/// plus any operator-configured extras from
/// [`sandbox_extra_allowed_domains`].
pub fn sandbox_allowed_domains() -> Vec<String> {
    DEFAULT_SANDBOX_ALLOWED_DOMAINS
        .iter()
        .map(|domain| (*domain).to_string())
        .chain(sandbox_extra_allowed_domains())
        .collect()
}

/// [`sandbox_allowed_domains`], expressed as [`NetworkPolicy`] `allowed_targets`
/// — ready to carry on the sandboxed profile's `builtin.shell` grant so
/// `validate_network_policy_metadata` (which rejects an empty allowlist)
/// passes, and so the policy documents what the container is actually meant
/// to reach.
pub fn sandbox_network_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: sandbox_allowed_domains()
            .into_iter()
            .map(|host_pattern| NetworkTargetPattern {
                scheme: None,
                host_pattern,
                port: None,
            })
            .collect(),
        deny_private_ip_ranges: true,
        max_egress_bytes: None,
    }
}

#[cfg(test)]
mod tests {
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
        let policy = sandbox_network_policy();
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

    #[test]
    fn extra_domains_env_is_parsed_and_merged() {
        // SAFETY: test-local env var, no concurrent readers of this key in
        // this crate's test binary.
        unsafe {
            std::env::set_var(
                SANDBOX_EXTRA_ALLOWED_DOMAINS_ENV,
                " example.internal , , *.corp.example.com",
            );
        }
        let extras = sandbox_extra_allowed_domains();
        unsafe {
            std::env::remove_var(SANDBOX_EXTRA_ALLOWED_DOMAINS_ENV);
        }
        assert_eq!(extras, vec!["example.internal", "*.corp.example.com"]);
    }
}
