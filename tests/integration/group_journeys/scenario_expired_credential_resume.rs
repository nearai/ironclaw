//! C-JOURNEY — the EXPIRED-credential arm of the auth gate: a credential the
//! user connected earlier is still stored and still injected, the provider
//! rejects it (401), the run parks on a re-auth gate, the user reconnects, and
//! the parked `github.get_repo` re-dispatches **with the new credential** and
//! completes.
//!
//! Distinct from `scenario_auth_gate_grant_resume`, which starts from NO
//! credential: there the gate is raised before any provider call, so there is
//! nothing stored that could be wrongly reused. Here a rejected credential
//! exists for the whole flow, and reusing it on resume is the actual failure
//! mode — the run would 401 again or loop. `auth/auth_gate.rs`'s
//! `runtime_401_after_injection_populates_provider_credential_requirement`
//! covers the same 401 park but drains it with `deny`, so the resume half of
//! the expired path was owned by neither.
//!
//! The two credentials must carry DIFFERENT material or this scenario cannot
//! fail: with one shared token string, a stale-credential reuse bug still
//! produces a request whose header matches, and the assertion passes on the
//! bug it exists to catch. Hence `seed_capability_credential_account_with_token`.

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;
use ironclaw_turns::TurnStatus;
use serde_json::json;

/// The credential the provider rejects. Must differ from the material
/// `resolve_auth_gate` mints (`itest-github-token`) — see the module doc.
const STALE_TOKEN: &str = "itest-github-expired-token";

/// What the user's reconnect mints, via the production manual-token flow.
const RECONNECTED_TOKEN: &str = "itest-github-token";

pub async fn run(g: &RebornIntegrationGroup) -> HarnessResult<()> {
    let h = g
        .thread("conv-expired-credential-resume")
        .script([
            // Gated tool-call turn = the call plus the one post-resume reply.
            RebornScriptedReply::tool_call(
                "github.get_repo",
                json!({"owner": "octocat", "repo": "hello-world"}),
            ),
            RebornScriptedReply::text(
                "EXPIREDRESUME repo info retrieved after reconnecting github",
            ),
        ])
        .build()
        .await?;

    // Auto-approve so `github.get_repo`'s Ask approval never fires — the auth
    // gate must be the only block in the path.
    h.enable_auto_approve().await?;

    // The user connected GitHub at some earlier point: a real Configured
    // account with real material, which dispatch will find and inject.
    h.seed_capability_credential_account_with_token("github", "stale github", &[], STALE_TOKEN)
        .await?;

    // First dispatch is rejected; the post-reconnect dispatch is accepted.
    // FIFO, one status consumed per call — so this also pins that exactly one
    // call happens before the park (a hot retry would eat the 200 and the
    // resume would fail).
    let capability = g
        .capability_harness()
        .ok_or("expired-credential resume needs the host-runtime capability")?;
    capability.install_network_status_script(401)?;
    capability.install_network_status_script(200)?;

    let (run, auth_gate) = h
        .submit_turn_until_auth_blocked("EXPIREDRESUME look up the repo")
        .await?;

    // The park is genuinely the expired path: the stored credential really was
    // injected and really was what the provider rejected. Without this the
    // scenario could be silently exercising the credential-MISSING path that
    // `scenario_auth_gate_grant_resume` already owns.
    h.assert_network_egress_header_contains("api.github.com", "authorization", STALE_TOKEN)
        .await?;

    // "User reconnects": the grant is stored through the production
    // manual-token flow, then the parked capability re-dispatches.
    h.resolve_auth_gate(run, &auth_gate).await?;
    h.wait_for_status(run, TurnStatus::Completed).await?;

    // The assertion this scenario exists for: the resumed dispatch carried the
    // RECONNECTED credential. A resume that reused the rejected one would still
    // reach `Completed` here — the scripted 200 does not care which token it
    // sees — so status alone cannot discriminate, and only the header can.
    h.assert_network_egress_header_contains("api.github.com", "authorization", RECONNECTED_TOKEN)
        .await?;

    // ...and the re-dispatch actually executed the tool rather than merely
    // unblocking: only the provider body surfacing back proves that.
    h.assert_tool_result_contains("octocat/hello-world").await?;

    // Exactly two provider calls: the rejected one and the resumed one. Pins
    // that the 401 was not hot-retried against the provider (#5878 shape).
    h.assert_network_egress_count(2).await?;
    Ok(())
}
