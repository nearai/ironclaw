//! Host-owned egress allowlist for sandboxed (`UserSandbox`) shell profiles.
//!
//! The same validated [`NetworkPolicy`](ironclaw_host_api::action::NetworkPolicy)
//! supplies the `builtin.shell` grant and the per-user `iron-proxy` renderer.
//! The worker has no direct route; the proxy enforces hostnames and rejects
//! non-public destinations.
use ironclaw_common::env_helpers::env_or_override;
use ironclaw_host_api::action::{NetworkPolicy, NetworkTargetPattern};
use ironclaw_network::NetworkPolicyError;

/// Environment variable operators can set to add domains to the sandboxed
/// shell's egress allowlist, on top of [`DEFAULT_SANDBOX_ALLOWED_DOMAINS`].
/// Comma-separated hostnames (e.g. `example.com,*.internal.example.com`).
pub const SANDBOX_EXTRA_ALLOWED_DOMAINS_ENV: &str = "IRONCLAW_SANDBOX_EXTRA_ALLOWED_DOMAINS";
const RETIRED_SANDBOX_MAX_EGRESS_BYTES_ENV: &str = "IRONCLAW_SANDBOX_MAX_EGRESS_BYTES";

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
    "sum.golang.org",
    // GitHub (source, API, and release archives). Content hosts are enumerated
    // exactly: the managed-egress proxy cannot represent `*.` wildcards
    // with the canonical one-label semantics, so wildcard patterns fail
    // managed-egress profile construction.
    "github.com",
    "api.github.com",
    "raw.githubusercontent.com",
    "codeload.github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
    "github-releases.githubusercontent.com",
    "media.githubusercontent.com",
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
pub fn sandbox_extra_allowed_domains() -> Result<Vec<NetworkTargetPattern>, NetworkPolicyError> {
    let Some(raw) = env_or_override(SANDBOX_EXTRA_ALLOWED_DOMAINS_ENV) else {
        return Ok(Vec::new());
    };
    raw.split(',')
        .map(str::trim)
        .filter(|domain| !domain.is_empty())
        .map(ironclaw_network::parse_host_pattern)
        .collect()
}

/// The full sandboxed-shell egress allowlist, as validated
/// [`NetworkTargetPattern`]s: [`DEFAULT_SANDBOX_ALLOWED_DOMAINS`] plus any
/// operator-configured extras from [`sandbox_extra_allowed_domains`]. `Err`
/// when an extra domain fails hostname-shape validation — see
/// [`sandbox_extra_allowed_domains`].
///
/// Carries the [`NetworkTargetPattern`] [`sandbox_extra_allowed_domains`]
/// already validated straight through rather than downgrading it to a
/// `String` and reconstructing the pattern by hand later — the validated
/// value is the proof that it passed [`ironclaw_network::parse_host_pattern`];
/// discarding it and rebuilding the struct from a bare string would let the
/// two drift apart.
pub fn sandbox_allowed_domains() -> Result<Vec<NetworkTargetPattern>, NetworkPolicyError> {
    Ok(parse_host_patterns(DEFAULT_SANDBOX_ALLOWED_DOMAINS)?
        .into_iter()
        .chain(sandbox_extra_allowed_domains()?)
        .collect())
}

/// Applies [`ironclaw_network::parse_host_pattern`] to each `domain`, so
/// [`sandbox_allowed_domains`]'s hardcoded defaults validate through the
/// identical chokepoint operator-supplied extras already go through in
/// [`sandbox_extra_allowed_domains`] — see that function's doc comment for
/// why hand-building [`NetworkTargetPattern`] here instead would let the two
/// drift apart.
fn parse_host_patterns(domains: &[&str]) -> Result<Vec<NetworkTargetPattern>, NetworkPolicyError> {
    domains
        .iter()
        .map(|domain| ironclaw_network::parse_host_pattern(domain))
        .collect()
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
    if let Some(raw) = env_or_override(RETIRED_SANDBOX_MAX_EGRESS_BYTES_ENV) {
        return Err(NetworkPolicyError::InvalidEgressLimit {
            raw,
            reason: "opaque TLS tunnels cannot enforce host-mediated HTTP request estimates"
                .to_string(),
        });
    }
    Ok(NetworkPolicy {
        allowed_targets: sandbox_allowed_domains()?,
        deny_private_ip_ranges: true,
        // CONNECT carries opaque TLS. Do not advertise the host-mediated HTTP
        // request-estimate ceiling as a wire-byte limit the proxy cannot enforce.
        max_egress_bytes: None,
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
            "sum.golang.org",
            "github.com",
            "api.github.com",
            "raw.githubusercontent.com",
            "objects.githubusercontent.com",
            "release-assets.githubusercontent.com",
            "media.githubusercontent.com",
        ] {
            assert!(
                DEFAULT_SANDBOX_ALLOWED_DOMAINS.contains(&expected),
                "expected {expected} in the default sandbox allowlist"
            );
        }
    }

    #[test]
    fn sandbox_network_policy_is_non_empty_and_denies_private_ips() {
        // This test holds the environment lock because sibling allowlist tests
        // mutate the same process-global override.
        let _guard = lock_env();
        remove_runtime_env(SANDBOX_EXTRA_ALLOWED_DOMAINS_ENV);
        remove_runtime_env(RETIRED_SANDBOX_MAX_EGRESS_BYTES_ENV);

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
        assert_eq!(policy.max_egress_bytes, None);
    }

    #[test]
    fn sandbox_network_policy_rejects_unenforceable_byte_ceiling_override() {
        let _guard = lock_env();
        set_runtime_env(RETIRED_SANDBOX_MAX_EGRESS_BYTES_ENV, "1048576");

        let result = sandbox_network_policy();

        remove_runtime_env(RETIRED_SANDBOX_MAX_EGRESS_BYTES_ENV);
        assert!(result.is_err());
    }

    // Defaults must fail the same way operator-supplied extras do when an
    // entry doesn't pass `parse_host_pattern` — pins that `sandbox_allowed_
    // domains` routes `DEFAULT_SANDBOX_ALLOWED_DOMAINS` through the same
    // validator instead of hand-building `NetworkTargetPattern` for them
    // (coderabbitai review on PR #6746: the round-trip through `String` and
    // back left a second construction site where `scheme`/`port` could
    // drift from the validator's contract, and defaults could ship
    // unvalidated where extras could not).
    #[test]
    fn parse_host_patterns_rejects_a_malformed_entry_like_extras_do() {
        assert!(parse_host_patterns(&["github.com", "not a host!"]).is_err());
    }

    #[test]
    fn parse_host_patterns_matches_the_real_default_list() {
        let parsed = parse_host_patterns(DEFAULT_SANDBOX_ALLOWED_DOMAINS).unwrap();
        let hosts: Vec<&str> = parsed.iter().map(|p| p.host_pattern.as_str()).collect();
        assert_eq!(hosts, DEFAULT_SANDBOX_ALLOWED_DOMAINS);
    }

    #[test]
    fn extra_domains_env_absent_yields_no_extras() {
        let _guard = lock_env();
        remove_runtime_env(SANDBOX_EXTRA_ALLOWED_DOMAINS_ENV);

        assert_eq!(
            sandbox_extra_allowed_domains().unwrap(),
            Vec::<NetworkTargetPattern>::new()
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

        let extra_hosts: Vec<&str> = extras
            .iter()
            .map(|pattern| pattern.host_pattern.as_str())
            .collect();
        assert_eq!(extra_hosts, vec!["example.internal", "*.corp.example.com"]);
        let all_hosts: Vec<&str> = all.iter().map(|p| p.host_pattern.as_str()).collect();
        assert!(
            all_hosts.contains(&"crates.io"),
            "operator extras must not displace the defaults: {all_hosts:?}"
        );
        assert!(all_hosts.contains(&"example.internal"));
        assert!(all_hosts.contains(&"*.corp.example.com"));
    }
}
