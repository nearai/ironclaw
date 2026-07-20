//! Membership-model ownership tests (#5459 P1, 2026-07-08 pivot),
//! driven through the lifecycle facade/port like every production caller.
//! Split out of the parent test module, whose shared fixtures it reuses
//! via `super::*`.
//!
//! Contract under test (docs/plans/2026-07-01-private-tool-installs.md):
//! a tenant (operator) install makes a tool available to everyone; a
//! member install makes it available to that member — and any number of
//! members can independently install the same tool (they join the one
//! installation row's member set). An operator install evicts every
//! member's private installation by replacing the member set with
//! `Tenant` in a single row write.

use super::*;
use ironclaw_product_workflow::LifecycleInstallScope;

/// Membership install rules through the facade: members install
/// independently (second member JOINS, not "unavailable"), each sees a
/// private entry, non-members stay masked, and a duplicate install by
/// the same member is rejected as already installed.
#[tokio::test]
async fn members_install_the_same_tool_independently() {
    let (_dir, _root, facade, _registry, installation_store) = extension_lifecycle_fixture();
    let fixture_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture").expect("fixture ref");
    let installation_id = ExtensionInstallationId::new("fixture").expect("fixture installation id");
    let alice = UserId::new("alice").expect("user");
    let bob = UserId::new("bob").expect("user");
    let carol = UserId::new("carol").expect("user");

    let install = |user: &str| {
        facade.execute(
            lifecycle_surface_context_for_user(user),
            LifecycleProductAction::ExtensionInstall {
                package_ref: fixture_ref.clone(),
            },
        )
    };
    let list = |user: &str| {
        facade.execute(
            lifecycle_surface_context_for_user(user),
            LifecycleProductAction::ExtensionList,
        )
    };

    // alice installs → visible to alice only.
    let response = install("alice").await.expect("alice installs for herself");
    assert_eq!(response.phase, LifecyclePhase::Installed);
    let owner_row = || async {
        installation_store
            .get_installation(&installation_id)
            .await
            .expect("store read")
            .expect("installation row")
    };
    let installation = owner_row().await;
    assert!(
        !installation.owner().is_tenant(),
        "member install is not tenant-wide"
    );
    assert!(installation.owner().visible_to(&alice));
    assert!(!installation.owner().visible_to(&bob));

    // bob installs the SAME tool → joins; both members now hold it.
    let response = install("bob")
        .await
        .expect("bob installs the same tool for himself (membership, not a slot)");
    assert_eq!(response.phase, LifecyclePhase::Installed);
    let installation = owner_row().await;
    assert!(!installation.owner().is_tenant());
    assert!(
        installation.owner().visible_to(&alice),
        "alice keeps the tool"
    );
    assert!(installation.owner().visible_to(&bob), "bob gains the tool");
    assert!(!installation.owner().visible_to(&carol), "carol does not");

    // Each member sees their own PRIVATE entry; carol sees nothing.
    for member in ["alice", "bob"] {
        let member_list = list(member).await.expect("member lists");
        let Some(LifecycleProductPayload::ExtensionList {
            extensions,
            count: 1,
        }) = member_list.payload.as_ref()
        else {
            panic!("{member} must see the tool they installed: {member_list:?}");
        };
        assert_eq!(
            extensions[0].install_scope,
            Some(LifecycleInstallScope::Private)
        );
    }
    let carol_list = list("carol").await.expect("carol lists");
    let Some(LifecycleProductPayload::ExtensionList { count: 0, .. }) = carol_list.payload.as_ref()
    else {
        panic!("members' installs must be invisible to non-members: {carol_list:?}");
    };

    // Duplicate install by a member who already holds it is a real error…
    let error = install("alice")
        .await
        .expect_err("alice already holds the tool");
    assert!(
        error.to_string().contains("already installed"),
        "unexpected: {error}"
    );

    // …while activate works for both members (the bundle publishes once;
    // the second activate is an idempotent re-publish).
    for member in ["alice", "bob"] {
        let response = facade
            .execute(
                lifecycle_surface_context_for_user(member),
                LifecycleProductAction::ExtensionActivate {
                    package_ref: fixture_ref.clone(),
                },
            )
            .await
            .expect("member activates the tool they hold");
        assert_eq!(response.phase, LifecyclePhase::Active);
    }

    // Non-members (carol AND the operator) stay masked on activate/remove:
    // same "is not installed" a missing row produces, no owner leak.
    for context in [
        lifecycle_surface_context_for_user("carol"),
        lifecycle_surface_context(),
    ] {
        for action in [
            LifecycleProductAction::ExtensionActivate {
                package_ref: fixture_ref.clone(),
            },
            LifecycleProductAction::ExtensionRemove {
                package_ref: fixture_ref.clone(),
            },
        ] {
            let error = facade
                .execute(context.clone(), action)
                .await
                .expect_err("a tool the caller does not hold must be inoperable");
            let rendered = error.to_string();
            assert!(
                rendered.contains("is not installed"),
                "unexpected: {rendered}"
            );
            assert!(
                !rendered.contains("alice") && !rendered.contains("bob"),
                "masking must not leak member identities: {rendered}"
            );
        }
    }
}

/// Remove = leave the member set: the other member keeps the tool; the
/// LAST member's remove triggers the full teardown, after which the id
/// is free for a fresh install.
#[tokio::test]
async fn member_remove_leaves_others_and_last_member_remove_tears_down() {
    let (_dir, _root, facade, _registry, installation_store) = extension_lifecycle_fixture();
    let fixture_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture").expect("fixture ref");
    let installation_id = ExtensionInstallationId::new("fixture").expect("fixture installation id");
    let extension_id = ExtensionId::new("fixture").expect("extension id");
    let alice = UserId::new("alice").expect("user");
    let bob = UserId::new("bob").expect("user");

    for member in ["alice", "bob"] {
        facade
            .execute(
                lifecycle_surface_context_for_user(member),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: fixture_ref.clone(),
                },
            )
            .await
            .expect("member installs");
    }
    facade
        .execute(
            lifecycle_surface_context_for_user("alice"),
            LifecycleProductAction::ExtensionActivate {
                package_ref: fixture_ref.clone(),
            },
        )
        .await
        .expect("alice activates");

    // alice removes → she leaves the set; bob keeps the (still enabled) tool.
    let response = facade
        .execute(
            lifecycle_surface_context_for_user("alice"),
            LifecycleProductAction::ExtensionRemove {
                package_ref: fixture_ref.clone(),
            },
        )
        .await
        .expect("alice removes the tool for herself");
    assert_eq!(response.phase, LifecyclePhase::Removed);
    let installation = installation_store
        .get_installation(&installation_id)
        .await
        .expect("store read")
        .expect("row survives while another member holds the tool");
    assert!(!installation.owner().visible_to(&alice), "alice left");
    assert!(installation.owner().visible_to(&bob), "bob keeps it");
    let alice_list = facade
        .execute(
            lifecycle_surface_context_for_user("alice"),
            LifecycleProductAction::ExtensionList,
        )
        .await
        .expect("alice lists");
    let Some(LifecycleProductPayload::ExtensionList { count: 0, .. }) = alice_list.payload.as_ref()
    else {
        panic!("alice no longer sees the tool: {alice_list:?}");
    };
    let bob_list = facade
        .execute(
            lifecycle_surface_context_for_user("bob"),
            LifecycleProductAction::ExtensionList,
        )
        .await
        .expect("bob lists");
    let Some(LifecycleProductPayload::ExtensionList { count: 1, .. }) = bob_list.payload.as_ref()
    else {
        panic!("bob must still see the tool: {bob_list:?}");
    };

    // bob (last member) removes → full teardown: row and manifest gone…
    facade
        .execute(
            lifecycle_surface_context_for_user("bob"),
            LifecycleProductAction::ExtensionRemove {
                package_ref: fixture_ref.clone(),
            },
        )
        .await
        .expect("last member removes → teardown");
    assert!(
        installation_store
            .get_installation(&installation_id)
            .await
            .expect("store read")
            .is_none(),
        "last member's remove must delete the installation row"
    );
    assert!(
        installation_store
            .get_manifest(&extension_id)
            .await
            .expect("store read")
            .is_none(),
        "last member's remove must delete the manifest"
    );

    // …and the id is free again for a fresh install.
    facade
        .execute(
            lifecycle_surface_context_for_user("carol"),
            LifecycleProductAction::ExtensionInstall {
                package_ref: fixture_ref,
            },
        )
        .await
        .expect("id is installable again after teardown");
}

/// The operator installing a member-held tool EVICTS every member's
/// private installation — one row write replacing the member set with
/// `Tenant` — after which everyone sees the shared tool and only the
/// operator can remove it.
#[tokio::test]
async fn operator_install_evicts_member_installs_to_tenant_shared() {
    let (_dir, _root, facade, _registry, installation_store) = extension_lifecycle_fixture();
    let fixture_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture").expect("fixture ref");
    let installation_id = ExtensionInstallationId::new("fixture").expect("fixture installation id");

    for member in ["alice", "bob"] {
        facade
            .execute(
                lifecycle_surface_context_for_user(member),
                LifecycleProductAction::ExtensionInstall {
                    package_ref: fixture_ref.clone(),
                },
            )
            .await
            .expect("member installs");
    }
    facade
        .execute(
            lifecycle_surface_context_for_user("alice"),
            LifecycleProductAction::ExtensionActivate {
                package_ref: fixture_ref.clone(),
            },
        )
        .await
        .expect("alice activates");

    // Operator installs → evicts both members' private installs to Tenant.
    let response = facade
        .execute(
            lifecycle_surface_context(),
            LifecycleProductAction::ExtensionInstall {
                package_ref: fixture_ref.clone(),
            },
        )
        .await
        .expect("operator install evicts member installs and takes the id");
    assert_eq!(response.phase, LifecyclePhase::Installed);
    let installation = installation_store
        .get_installation(&installation_id)
        .await
        .expect("store read")
        .expect("installation row");
    assert!(
        installation.owner().is_tenant(),
        "operator install must own the id tenant-wide after eviction"
    );
    assert_eq!(
        installation.activation_state(),
        ExtensionActivationState::Enabled,
        "eviction must not disable the already-activated tool"
    );

    // Everyone — evicted members and never-installed users alike — now
    // sees the SHARED entry.
    for user in ["alice", "bob", "carol"] {
        let user_list = facade
            .execute(
                lifecycle_surface_context_for_user(user),
                LifecycleProductAction::ExtensionList,
            )
            .await
            .expect("list");
        let Some(LifecycleProductPayload::ExtensionList {
            extensions,
            count: 1,
        }) = user_list.payload.as_ref()
        else {
            panic!("shared install must be visible to {user}: {user_list:?}");
        };
        assert_eq!(
            extensions[0].install_scope,
            Some(LifecycleInstallScope::Shared)
        );
    }

    // Operator re-install is a real duplicate; members cannot remove the
    // shared tool; the operator can.
    let error = facade
        .execute(
            lifecycle_surface_context(),
            LifecycleProductAction::ExtensionInstall {
                package_ref: fixture_ref.clone(),
            },
        )
        .await
        .expect_err("shared tool is already installed");
    assert!(
        error.to_string().contains("already installed"),
        "unexpected: {error}"
    );
    for member in ["alice", "bob"] {
        let error = facade
            .execute(
                lifecycle_surface_context_for_user(member),
                LifecycleProductAction::ExtensionRemove {
                    package_ref: fixture_ref.clone(),
                },
            )
            .await
            .expect_err("members cannot remove a shared tool");
        assert!(
            error.to_string().contains("only the tenant admin"),
            "unexpected: {error}"
        );
    }
    facade
        .execute(
            lifecycle_surface_context(),
            LifecycleProductAction::ExtensionRemove {
                package_ref: fixture_ref,
            },
        )
        .await
        .expect("operator removes the shared tool");
}

/// A member installing a tool the operator already shares gets the real
/// "already installed" error — the tool is already available to them.
#[tokio::test]
async fn member_install_of_a_shared_tool_reports_already_installed() {
    let (_dir, _root, facade, _registry, _store) = extension_lifecycle_fixture();
    let fixture_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture").expect("fixture ref");

    facade
        .execute(
            lifecycle_surface_context(),
            LifecycleProductAction::ExtensionInstall {
                package_ref: fixture_ref.clone(),
            },
        )
        .await
        .expect("operator installs shared");
    let error = facade
        .execute(
            lifecycle_surface_context_for_user("alice"),
            LifecycleProductAction::ExtensionInstall {
                package_ref: fixture_ref,
            },
        )
        .await
        .expect_err("the shared tool is already available to alice");
    assert!(
        error.to_string().contains("already installed"),
        "unexpected: {error}"
    );
}

/// #5525 review: `LifecycleProductCommandService` dispatches every
/// `/extension_*` command as `LifecycleProductContext::Command`, so the
/// facade must derive the caller from the verified command auth claim
/// instead of rejecting non-surface contexts outright — and the
/// membership masking must hold on the command path too.
#[tokio::test]
async fn extension_lifecycle_commands_derive_caller_from_command_auth_claim() {
    let (_dir, _root, facade, _registry, installation_store) = extension_lifecycle_fixture();
    let fixture_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture").expect("fixture ref");
    let installation_id = ExtensionInstallationId::new("fixture").expect("fixture installation id");
    let alice = UserId::new("alice").expect("user");
    let bob = UserId::new("bob").expect("user");

    // alice installs through the command path → membership derives from the claim.
    let response = facade
        .execute(
            lifecycle_command_context_for_user("alice"),
            LifecycleProductAction::ExtensionInstall {
                package_ref: fixture_ref.clone(),
            },
        )
        .await
        .expect("command install derives caller from auth claim");
    assert_eq!(response.phase, LifecyclePhase::Installed);
    let installation = installation_store
        .get_installation(&installation_id)
        .await
        .expect("store read")
        .expect("installation row");
    assert!(
        installation.owner().visible_to(&alice) && !installation.owner().is_tenant(),
        "command install must be held by the claim subject"
    );
    assert!(!installation.owner().visible_to(&bob));

    // alice's command list sees it; bob's command list stays masked.
    let alice_list = facade
        .execute(
            lifecycle_command_context_for_user("alice"),
            LifecycleProductAction::ExtensionList,
        )
        .await
        .expect("alice lists via command");
    let Some(LifecycleProductPayload::ExtensionList { count: 1, .. }) = alice_list.payload.as_ref()
    else {
        panic!("alice must see her install via command: {alice_list:?}");
    };
    let bob_list = facade
        .execute(
            lifecycle_command_context_for_user("bob"),
            LifecycleProductAction::ExtensionList,
        )
        .await
        .expect("bob lists via command");
    let Some(LifecycleProductPayload::ExtensionList { count: 0, .. }) = bob_list.payload.as_ref()
    else {
        panic!("alice's install must stay invisible on the command path: {bob_list:?}");
    };

    // Membership masking holds for command-path mutations by a non-member.
    let error = facade
        .execute(
            lifecycle_command_context_for_user("bob"),
            LifecycleProductAction::ExtensionActivate {
                package_ref: fixture_ref.clone(),
            },
        )
        .await
        .expect_err("a tool bob does not hold must be inoperable via command");
    assert!(
        error.to_string().contains("is not installed"),
        "unexpected: {error}"
    );

    // bob JOINS through the command path, then alice activates and removes
    // hers; bob keeps the tool.
    facade
        .execute(
            lifecycle_command_context_for_user("bob"),
            LifecycleProductAction::ExtensionInstall {
                package_ref: fixture_ref.clone(),
            },
        )
        .await
        .expect("bob joins via command");
    facade
        .execute(
            lifecycle_command_context_for_user("alice"),
            LifecycleProductAction::ExtensionActivate {
                package_ref: fixture_ref.clone(),
            },
        )
        .await
        .expect("alice activates via command");
    facade
        .execute(
            lifecycle_command_context_for_user("alice"),
            LifecycleProductAction::ExtensionRemove {
                package_ref: fixture_ref,
            },
        )
        .await
        .expect("alice removes via command");
    let installation = installation_store
        .get_installation(&installation_id)
        .await
        .expect("store read")
        .expect("bob still holds the tool");
    assert!(installation.owner().visible_to(&bob));
}

/// #5459 P1: the owner join in `active_model_visible_capabilities` — a
/// member-installed+activated extension's capabilities carry the member
/// set, which is what the grant-minting filter keys on.
#[tokio::test]
async fn active_capabilities_carry_installation_owner() {
    let (_dir, _root, port, _registry, _store) =
        extension_management_port_fixture_with_catalog_and_service(
            AvailableExtensionCatalog::from_packages(vec![fixture_extension_package()]),
            ExtensionLifecycleService::new(ExtensionRegistry::new()),
        );
    let alice = UserId::new("alice").expect("valid user");
    let bob = UserId::new("bob").expect("valid user");
    let package_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "fixture").expect("fixture ref");
    port.install(package_ref.clone(), &alice)
        .await
        .expect("alice installs for herself");
    port.activate(package_ref, ExtensionActivationMode::Static, &alice)
        .await
        .expect("alice activates");

    let capabilities = port
        .active_model_visible_capabilities()
        .await
        .expect("active capabilities");
    assert!(!capabilities.is_empty(), "fixture capability published");
    for capability in &capabilities {
        assert!(
            capability.owner.visible_to(&alice) && !capability.owner.visible_to(&bob),
            "capability must carry the member set for grant filtering"
        );
    }

    // The operator/settings tool catalog joins THIS owner map to hide a
    // tool from users who don't hold it (#5459 P1 leak fix). Pin that the
    // map reports the membership keyed by extension id.
    let owners = port
        .installation_owners()
        .await
        .expect("installation owners");
    let owner = owners
        .get(&ExtensionId::new("fixture").unwrap())
        .expect("fixture owner present");
    assert!(
        owner.visible_to(&alice) && !owner.visible_to(&bob) && !owner.is_tenant(),
        "installation_owners must report the membership the catalog filters on"
    );
}

/// Command-path context whose verified auth claim subject is `user` —
/// mirrors `LifecycleProductCommandService` dispatching `/extension_*`
/// commands as `LifecycleProductContext::Command`.
fn lifecycle_command_context_for_user(user: &str) -> LifecycleProductContext {
    use ironclaw_product_adapters::{
        AdapterInstallationId, AuthRequirement, ExternalActorRef, ExternalConversationRef,
        ExternalEventId, ProductAdapterId, ProductTriggerReason, ProtocolAuthEvidence,
    };
    use ironclaw_product_workflow::{
        ActionFingerprintKey, ProductActionId, ProductCommandContext, SourceBindingKey,
    };

    let adapter_id = ProductAdapterId::new("test_adapter").expect("valid adapter");
    let installation_id = AdapterInstallationId::new("install_alpha").expect("valid installation");
    let actor = ExternalActorRef::new("test", user, Option::<String>::None).expect("valid actor");
    let conversation =
        ExternalConversationRef::new(None, "conv1", None, None).expect("valid conversation");
    let auth_claim = ProtocolAuthEvidence::test_verified(
        AuthRequirement::SharedSecretHeader {
            header_name: "X-Secret".into(),
        },
        user,
    )
    .claim()
    .cloned()
    .expect("verified claim");
    LifecycleProductContext::Command(Box::new(ProductCommandContext {
        action_id: ProductActionId::new(),
        fingerprint: ActionFingerprintKey::new(
            adapter_id.clone(),
            installation_id.clone(),
            actor.clone(),
            SourceBindingKey::new("space:0:;conversation:5:conv1;topic:0:;").expect("binding key"),
            ExternalEventId::new("evt:lifecycle-command").expect("valid event"),
        ),
        adapter_id,
        installation_id,
        external_actor_ref: actor,
        external_conversation_ref: conversation,
        auth_claim,
        trigger: ProductTriggerReason::BotCommand,
        received_at: chrono::Utc::now(),
    }))
}
