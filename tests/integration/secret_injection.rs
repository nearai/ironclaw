//! IronClaw integration-test tier — T0-SECRET-INJECT.
//!
//! Proves credential/secret injection reaches the wire: a scripted `github.*`
//! call executes the real first-party GitHub WASM capability behind a
//! `GithubHarnessAuthorizer` that attaches an `InjectCredentialAccountOnce`
//! obligation. The host egress pipeline resolves the synthetic access token
//! (from the harness `StaticSecretStore`) and injects it as `Authorization:
//! Bearer <token>` onto the outbound request before the recording *network*
//! egress captures it (the runtime egress lane is inert here —
//! `try_with_host_http_egress` overwrites it — see
//! `assert_network_egress_header_contains`).
//!
//! Security: the token is a synthetic test fixture, never a real credential.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod ironclaw_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use ironclaw_support::builder::IronClawIntegrationHarness;
use ironclaw_support::reply::IronClawScriptedReply;
use serde_json::json;

#[tokio::test]
async fn injects_credential_onto_github_egress() {
    let harness = IronClawIntegrationHarness::test_default()
        .with_github_issue_tools()
        .script([
            IronClawScriptedReply::tool_call(
                "github.get_repo",
                json!({"owner": "nearai", "repo": "ironclaw"}),
            ),
            IronClawScriptedReply::text("done"),
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
    // Proves injection reached the wire, not just the authorizer's obligation.
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
    // pass vacuously.
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
