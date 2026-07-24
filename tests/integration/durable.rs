//! Reborn integration test — cross-reopen capability durability (E-DURABLE seam).
//!
//! Installs an extension through a real turn, then reopens a FRESH, independent
//! `ExtensionInstallationStorePort` at the capability harness's on-disk storage root
//! and asserts the install survived — proving capability-produced state persists
//! to disk, not just to in-memory state. Parallels
//! `assert_reply_persists_after_reopen` for capability state.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

#[test]
fn extension_install_survives_independent_reopen() {
    run_async_test_with_stack(
        "extension_install_survives_independent_reopen",
        extension_install_survives_independent_reopen_async,
    );
}

async fn extension_install_survives_independent_reopen_async() {
    let group = RebornIntegrationGroup::extension_lifecycle()
        .await
        .expect("extension-lifecycle group builds");
    let harness = group
        .thread("conv-durable")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "github"}),
            ),
            RebornScriptedReply::text("installed"),
        ])
        .build()
        .await
        .expect("thread builds");
    harness
        .seed_capability_credential_account("github", "durable github ready path", &[])
        .await
        .expect("GitHub credential is ready for the durable install path");

    harness
        .submit_turn("install github")
        .await
        .expect("turn completes");
    harness
        .assert_tool_result_contains("\"installed\":true")
        .await
        .expect("install reported success");

    harness
        .assert_extension_install_persists_after_reopen("github")
        .await
        .expect("installed extension survives an independent reopen");
}

fn run_async_test_with_stack<F, Fut>(name: &'static str, test: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + 'static,
{
    let handle = std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio test runtime")
                .block_on(test());
        })
        .expect("spawn stack-sized test thread");
    if let Err(panic) = handle.join() {
        std::panic::resume_unwind(panic);
    }
}
