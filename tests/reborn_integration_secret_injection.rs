//! Reborn integration-test tier — T0-SECRET-INJECT.
//!
//! Proves credential/secret injection reaches the wire: a scripted `github.*`
//! tool call executes the real first-party GitHub WASM capability behind a
//! `GithubHarnessAuthorizer` that attaches an `InjectCredentialAccountOnce`
//! obligation. The host egress pipeline resolves the synthetic access token
//! (`ghp_fake_fixture_token`, from the harness `StaticSecretStore`) and injects
//! it as `Authorization: Bearer <token>` onto the outbound request before the
//! recording network egress captures it. The assertion reads that captured
//! request and confirms the injected credential is present on the header.
//!
//! Note on the egress lane: this harness's runtime egress recorder
//! (`runtime_http_requests()`) is inert — `try_with_host_http_egress` overwrites
//! the runtime port with the host pipeline over the recording *network* egress —
//! so injection is observable on the network lane. See
//! `assert_network_egress_header_contains` for the full mechanism.
//!
//! Security: the token is a synthetic test fixture, never a real credential.

// The support tree is large and shared; a single-test file exercises only a
// slice of it, so suppress dead-code warnings on the includes (matches
// `reborn_integration_greeting.rs`).
#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
mod support;

use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

#[tokio::test]
async fn injects_credential_onto_github_egress() {
    let harness = RebornIntegrationHarness::test_default()
        .with_github_issue_tools()
        .script([
            RebornScriptedReply::tool_call(
                "github.get_repo",
                json!({"owner": "nearai", "repo": "ironclaw"}),
            ),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("harness builds");
    harness
        .submit_turn("fetch the ironclaw repo")
        .await
        .expect("turn completes");
    harness
        .assert_reply_contains("done")
        .await
        .expect("reply finalized in thread history");
    // The synthetic access token was injected onto the outbound request as a
    // Bearer credential by the host egress pipeline — proving injection reaches
    // the wire, not just the authorizer's obligation.
    harness
        .assert_network_egress_header_contains(
            "api.github.com/repos/nearai/ironclaw",
            "authorization",
            "Bearer ghp_fake_fixture_token",
        )
        .await
        .expect("injected credential present on github egress request");

    // Negative-path coverage on the SAME captured request: a regression that
    // ignored the url/header-name/value inputs must not let this assertion
    // pass vacuously (review comment on PR #5483).
    let wrong_url = harness
        .assert_network_egress_header_contains(
            "api.github.com/repos/nonexistent/repo",
            "authorization",
            "Bearer ghp_fake_fixture_token",
        )
        .await
        .expect_err("no captured request should match an unrelated url");
    assert!(
        wrong_url
            .to_string()
            .contains("no captured network egress request matching url")
    );

    let wrong_header_name = harness
        .assert_network_egress_header_contains(
            "api.github.com/repos/nearai/ironclaw",
            "x-not-a-real-header",
            "Bearer ghp_fake_fixture_token",
        )
        .await
        .expect_err("matching url has no such header name");
    assert!(wrong_header_name.to_string().contains("has header"));

    let wrong_value = harness
        .assert_network_egress_header_contains(
            "api.github.com/repos/nearai/ironclaw",
            "authorization",
            "Bearer wrong-token",
        )
        .await
        .expect_err("matching url/header present but value doesn't match");
    assert!(wrong_value.to_string().contains("has header"));
}
