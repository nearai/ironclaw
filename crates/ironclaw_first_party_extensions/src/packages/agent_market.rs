//! Agent Market MCP package — agent.market marketplace tools (search/hire/jobs)
//! over a hosted MCP server, api-key credential delivered per user at
//! provisioning, host-mediated egress. Assets: per-tool input JSON schemas
//! (no bundled WASM; dispatched via MCP).
//!
//! The bundled manifest carries a `MARKET_PUBLIC_HOST` placeholder in
//! `[mcp].server`; [`bundle`] substitutes the deployment's marketplace origin
//! from `AGENT_MARKET_MCP_URL` (e.g. `https://market.example.com/mcp`). The
//! connection credential's audience is derived from the server host by the
//! v3 manifest normalizer, so the one substitution re-targets credential
//! injection too. Without the env var the placeholder stays — the catalog
//! entry is present but its server is unreachable, matching a deployment
//! that has no marketplace. A SET-but-malformed value fails loudly at
//! startup: silently shipping a corrupted manifest would surface only as
//! the extension mysteriously failing to load much later.

use std::borrow::Cow;

use ironclaw_host_api::EffectKind;

use super::{PackageBundle, PackageOnboarding, bytes_asset};

pub(super) const ID: &str = "agent-market";

const MANIFEST: &str = include_str!("../../assets/agent-market/manifest.toml");

/// Deployment-time override for the marketplace MCP origin.
const SERVER_URL_ENV: &str = "AGENT_MARKET_MCP_URL";

/// Validate the operator-supplied server URL. Requirements mirror what the
/// hosted-MCP endpoint parser accepts: https, host, no
/// userinfo/query/fragment. Panics with a message naming the env var — a
/// set-but-malformed (or set-but-blank) value is an operator error, and
/// failing at startup beats an extension that mysteriously never loads.
fn validated_server_url(raw: &str) -> &str {
    let trimmed = raw.trim();
    assert!(
        !trimmed.is_empty(),
        "{SERVER_URL_ENV} is set but blank — unset it entirely for a \
         deployment without a marketplace, or set a real https URL"
    );
    let parsed = url::Url::parse(trimmed).unwrap_or_else(|error| {
        panic!("{SERVER_URL_ENV} is not a valid URL ({error}): {trimmed:?}")
    });
    let ok = parsed.scheme() == "https"
        && parsed.host_str().is_some()
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.query().is_none()
        && parsed.fragment().is_none();
    assert!(
        ok,
        "{SERVER_URL_ENV} must be a plain https URL (host + path only, no \
         userinfo/query/fragment): {trimmed:?}"
    );
    trimmed
}

/// Substitute `[mcp].server` through the TOML model rather than raw string
/// replacement: parse → set the one field → re-serialize. No placeholder
/// string is duplicated between this module and the manifest (an endpoint
/// edit in the asset cannot silently detach the env override), and the value
/// never touches TOML source syntax, so no character class of the URL can
/// corrupt adjacent keys.
fn manifest_with_server_url(url: &str) -> String {
    let mut manifest: toml::Value =
        toml::from_str(MANIFEST).expect("bundled agent-market manifest is valid TOML");
    let mcp = manifest
        .get_mut("mcp")
        .and_then(toml::Value::as_table_mut)
        .expect("bundled agent-market manifest declares [mcp]");
    mcp.insert("server".to_string(), toml::Value::String(url.to_string()));
    toml::to_string(&manifest).expect("patched agent-market manifest re-serializes")
}

pub(super) fn bundle() -> PackageBundle {
    // Read through the workspace's thread-safe env overlay (the repo-wide
    // replacement for raw `std::env`; tests inject overrides without process
    // env mutation). The `_present` variant surfaces a SET-but-empty value
    // instead of folding it into "unset": PRESENT → validate (blank or
    // malformed fails loudly — a set variable must be a real https URL) and
    // patch [mcp].server through the TOML model. Absent → ship the
    // placeholder manifest: the deployment has no marketplace and the
    // extension points nowhere.
    let manifest_toml =
        match ironclaw_common::env_helpers::env_or_override_present(SERVER_URL_ENV) {
            Some(url) => Cow::Owned(manifest_with_server_url(validated_server_url(&url))),
            None => Cow::Borrowed(MANIFEST),
        };
    // The `manifest.toml` asset must carry the SAME (possibly env-patched)
    // bytes the package validates with: install materializes the assets into
    // the extension dir, and a divergent copy would change the manifest hash
    // the installation records pin.
    let assets = assets(manifest_toml.as_bytes());
    PackageBundle {
        id: ID,
        display_name: "Agent Market",
        manifest_toml,
        assets,
        onboarding: Some(PackageOnboarding {
            instructions: "Agent Market needs the marketplace-issued API token before its \
                search and hire tools can run."
                .to_string(),
            credential_instructions: Some(
                "Paste the `axm_` bearer the marketplace issued for this account. Managed \
                deployments deliver it automatically at provisioning."
                    .to_string(),
            ),
            setup_url: None,
            credential_next_step: "After saving the token, IronClaw finishes Agent Market \
                installation automatically and publishes its MCP tools."
                .to_string(),
        }),
        // MCP api-key extension: Dispatch + Network + UseSecret + ExternalWrite
        // (hire/submit mutate marketplace state) + Financial (hire_agent
        // spends the user's money; the trust grant must supply the effect the
        // hard approval floor keys on).
        trust_effects: Some(vec![
            EffectKind::DispatchCapability,
            EffectKind::Network,
            EffectKind::UseSecret,
            EffectKind::ExternalWrite,
            EffectKind::Financial,
        ]),
    }
}

fn assets(manifest: &[u8]) -> Vec<super::PackageAsset> {
    macro_rules! agent_market_schema_asset {
        ($path:literal) => {
            bytes_asset(
                concat!("schemas/", $path),
                include_bytes!(concat!("../../assets/agent-market/schemas/", $path)),
            )
        };
    }

    vec![
        bytes_asset("manifest.toml", manifest),
        agent_market_schema_asset!("search_agents.input.v1.json"),
        agent_market_schema_asset!("hire_agent.input.v1.json"),
        agent_market_schema_asset!("create_job.input.v1.json"),
        agent_market_schema_asset!("get_job_result.input.v1.json"),
        agent_market_schema_asset!("submit_deliverable.input.v1.json"),
        agent_market_schema_asset!("read_messages.input.v1.json"),
        agent_market_schema_asset!("list_jobs.input.v1.json"),
        agent_market_schema_asset!("cancel_job.input.v1.json"),
    ]
}

#[cfg(test)]
mod tests {
    use super::validated_server_url;

    /// The caller-level contract (not just the helper): with the env set,
    /// `bundle()` patches `[mcp].server` AND ships the same patched bytes as
    /// the `manifest.toml` asset — the pair the installation hash pins.
    /// Uses the workspace runtime-env overlay (no process env mutation).
    #[test]
    fn bundle_patches_manifest_and_asset_from_env() {
        let _env = ironclaw_common::env_helpers::lock_env();
        let snapshot =
            ironclaw_common::env_helpers::snapshot_runtime_env(super::SERVER_URL_ENV);
        ironclaw_common::env_helpers::set_runtime_env(
            super::SERVER_URL_ENV,
            "https://market.test.example/mcp",
        );
        let bundle = super::bundle();
        ironclaw_common::env_helpers::restore_runtime_env(snapshot);

        let parsed: toml::Value =
            toml::from_str(&bundle.manifest_toml).expect("patched manifest parses");
        assert_eq!(
            parsed["mcp"]["server"].as_str(),
            Some("https://market.test.example/mcp")
        );
        let manifest_asset = bundle
            .assets
            .iter()
            .find(|a| a.path == "manifest.toml")
            .expect("manifest.toml asset present");
        let super::super::PackageAssetContent::Bytes(bytes) = &manifest_asset.content;
        assert_eq!(
            bytes.as_slice(),
            bundle.manifest_toml.as_bytes(),
            "asset must carry the SAME patched bytes the package validates with"
        );
    }

    /// Without the env the placeholder manifest ships untouched.
    #[test]
    fn bundle_without_env_ships_the_placeholder() {
        let _env = ironclaw_common::env_helpers::lock_env();
        let snapshot =
            ironclaw_common::env_helpers::snapshot_runtime_env(super::SERVER_URL_ENV);
        // Mask instead of remove: also shields the test from a real value in
        // the process environment.
        ironclaw_common::env_helpers::mask_runtime_env(super::SERVER_URL_ENV);
        let bundle = super::bundle();
        ironclaw_common::env_helpers::restore_runtime_env(snapshot);
        assert!(
            bundle.manifest_toml.contains("MARKET_PUBLIC_HOST"),
            "absent env keeps the unreachable placeholder"
        );
    }

    #[test]
    fn accepts_a_plain_https_url() {
        assert_eq!(
            validated_server_url(" https://market.example.com/mcp "),
            "https://market.example.com/mcp"
        );
    }

    /// A set-but-blank value is an operator error, not "unset".
    #[test]
    #[should_panic(expected = "AGENT_MARKET_MCP_URL")]
    fn rejects_blank_value() {
        validated_server_url("   ");
    }

    /// The caller-level contract for the blank case: `AGENT_MARKET_MCP_URL=""`
    /// must fail `bundle()` loudly, not silently ship the placeholder — the
    /// `_present` env read keeps a set-but-empty value visible to validation.
    /// Uses the runtime-env overlay (no process env mutation); the snapshot is
    /// restored before the panic assert so the poisoned lock can't leak state.
    #[test]
    fn bundle_rejects_a_set_but_blank_value() {
        let _env = ironclaw_common::env_helpers::lock_env();
        let snapshot =
            ironclaw_common::env_helpers::snapshot_runtime_env(super::SERVER_URL_ENV);
        ironclaw_common::env_helpers::set_runtime_env(super::SERVER_URL_ENV, "");
        let panic = std::panic::catch_unwind(super::bundle);
        ironclaw_common::env_helpers::restore_runtime_env(snapshot);
        let message = match panic {
            Ok(_) => panic!("bundle() accepted a set-but-blank {}", super::SERVER_URL_ENV),
            Err(payload) => payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default(),
        };
        assert!(
            message.contains("set but blank"),
            "panic must name the operator error: {message}"
        );
    }

    /// A fragment (or any other non-plain component) is rejected by URL
    /// validation before it reaches the manifest.
    #[test]
    #[should_panic(expected = "AGENT_MARKET_MCP_URL")]
    fn rejects_fragment() {
        validated_server_url("https://market.example.com/mcp#frag");
    }

    /// The substitution goes through the TOML model, so even URL-legal
    /// characters that are TOML-significant (quotes in a path segment) can
    /// never corrupt adjacent keys — they are escaped structurally.
    #[test]
    fn toml_patching_is_structural() {
        let url = "https://market.example.com/mc\"p";
        let patched = super::manifest_with_server_url(url);
        let parsed: toml::Value = toml::from_str(&patched).expect("patched manifest stays valid TOML");
        assert_eq!(
            parsed["mcp"]["server"].as_str(),
            Some(url),
            "the exact URL round-trips through the TOML model"
        );
        assert!(parsed["mcp"].get("hack").is_none());
    }

    #[test]
    #[should_panic(expected = "AGENT_MARKET_MCP_URL")]
    fn rejects_non_https() {
        validated_server_url("http://market.example.com/mcp");
    }
}
