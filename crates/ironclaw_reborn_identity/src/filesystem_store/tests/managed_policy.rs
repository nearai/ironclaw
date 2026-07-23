use super::*;

#[tokio::test]
async fn private_user_is_claimed_only_by_verified_login_with_matching_email() {
    let store = store();
    let tenant = tenant("acme");
    let invited = store
        .create_user(
            &tenant,
            Some("Invited@Example.com".to_string()),
            Some("Invited Human".to_string()),
            RebornUserRole::Member,
            UserContentAccessPolicy::Private,
            &UserId::new("admin").expect("admin"),
        )
        .await
        .expect("create private invitation");

    let unverified = store
        .resolve_or_create(oauth(
            &tenant,
            "google",
            "unverified-subject",
            Some("invited@example.com"),
            false,
        ))
        .await
        .expect("unverified identity creates its own user");
    assert_ne!(unverified, invited.user_id);
    let claimed = store
        .resolve_or_create(oauth(
            &tenant,
            "github",
            "verified-subject",
            Some("invited@example.com"),
            true,
        ))
        .await
        .expect("verified identity claims invitation");
    assert_eq!(claimed, invited.user_id);
}

#[tokio::test]
async fn private_user_creation_rejects_an_email_already_reserved_in_tenant() {
    let store = store();
    let tenant = tenant("acme");
    let creator = UserId::new("admin").expect("admin");
    store
        .create_user(
            &tenant,
            Some("same@example.com".to_string()),
            None,
            RebornUserRole::Member,
            UserContentAccessPolicy::Private,
            &creator,
        )
        .await
        .expect("first invitation");
    let error = store
        .create_user(
            &tenant,
            Some("SAME@example.com".to_string()),
            None,
            RebornUserRole::Member,
            UserContentAccessPolicy::Private,
            &creator,
        )
        .await
        .expect_err("duplicate tenant email must be rejected");
    assert!(matches!(error, RebornIdentityError::UserPolicyViolation(_)));
    assert_eq!(
        store
            .list_users(&tenant, None, None, 10)
            .await
            .expect("list")
            .len(),
        1,
        "the losing candidate user is cleaned up"
    );
}

#[tokio::test]
async fn admin_managed_authorization_requires_admin_same_tenant_and_managed_target() {
    let store = store();
    let acme = tenant("acme");
    let other = tenant("other");
    let creator = UserId::new("bootstrap").expect("creator");
    let admin = store
        .create_user(
            &acme,
            None,
            Some("Admin".to_string()),
            RebornUserRole::Admin,
            UserContentAccessPolicy::Private,
            &creator,
        )
        .await
        .expect("admin");
    let member = store
        .create_user(
            &acme,
            None,
            Some("Member".to_string()),
            RebornUserRole::Member,
            UserContentAccessPolicy::Private,
            &creator,
        )
        .await
        .expect("member");
    let private = store
        .create_user(
            &acme,
            Some("human@example.com".to_string()),
            Some("Human".to_string()),
            RebornUserRole::Member,
            UserContentAccessPolicy::Private,
            &admin.user_id,
        )
        .await
        .expect("private user");
    let managed = store
        .create_user(
            &acme,
            None,
            Some("Managed".to_string()),
            RebornUserRole::Member,
            UserContentAccessPolicy::TenantAdminManaged,
            &admin.user_id,
        )
        .await
        .expect("managed user");

    assert!(
        store
            .authorize_admin_managed_target(
                &acme,
                &admin.user_id,
                &managed.user_id,
                AdminManagedUserOperation::ManageSecrets
            )
            .await
            .expect("authorize")
    );
    assert!(
        !store
            .authorize_admin_managed_target(
                &acme,
                &admin.user_id,
                &private.user_id,
                AdminManagedUserOperation::ManageSecrets
            )
            .await
            .expect("private target denies")
    );
    assert!(
        !store
            .authorize_admin_managed_target(
                &acme,
                &member.user_id,
                &managed.user_id,
                AdminManagedUserOperation::ManageSecrets
            )
            .await
            .expect("non-admin denies")
    );
    assert!(
        !store
            .authorize_admin_managed_target(
                &other,
                &admin.user_id,
                &managed.user_id,
                AdminManagedUserOperation::ManageSecrets
            )
            .await
            .expect("cross-tenant denies")
    );
}

#[tokio::test]
async fn managed_users_cannot_be_promoted_or_record_a_login() {
    let store = store();
    let tenant = tenant("acme");
    let managed = store
        .create_user(
            &tenant,
            None,
            Some("Managed".to_string()),
            RebornUserRole::Member,
            UserContentAccessPolicy::TenantAdminManaged,
            &UserId::new("admin").expect("admin"),
        )
        .await
        .expect("managed user");
    let promote = store
        .update_role(&managed.user_id, RebornUserRole::Admin)
        .await
        .expect_err("managed target cannot be promoted");
    assert!(matches!(
        promote,
        RebornIdentityError::UserPolicyViolation(_)
    ));
    let login = store
        .record_last_login(&managed.user_id, "2026-07-22T00:00:00Z".to_string())
        .await
        .expect_err("managed target cannot record a successful login");
    assert!(matches!(
        login,
        RebornIdentityError::ManagedUserLoginDisabled(_)
    ));

    store
        .bind(
            ExternalIdentityKey {
                tenant_id: tenant.clone(),
                surface_kind: SurfaceKind::Oauth,
                provider_kind: ProviderKind::new("google").expect("provider"),
                provider_instance_id: None,
                external_subject_id: ExternalSubjectId::new("managed-subject").expect("subject"),
            },
            &managed.user_id,
        )
        .await
        .expect("bind migration identity");
    let resolve = store
        .resolve_or_create(oauth(
            &tenant,
            "google",
            "managed-subject",
            Some("managed@example.com"),
            true,
        ))
        .await
        .expect_err("managed target cannot sign in through a bound identity");
    assert!(matches!(
        resolve,
        RebornIdentityError::ManagedUserLoginDisabled(_)
    ));
}

#[tokio::test]
async fn reusable_login_policy_rechecks_actor_and_subject_state() {
    let store = Arc::new(store());
    let acme = tenant("acme");
    let other = tenant("other");
    let creator = UserId::new("bootstrap").expect("creator");
    let admin = store
        .create_user(
            &acme,
            None,
            Some("Admin".to_string()),
            RebornUserRole::Admin,
            UserContentAccessPolicy::Private,
            &creator,
        )
        .await
        .expect("admin");
    let private = store
        .create_user(
            &acme,
            None,
            Some("Private".to_string()),
            RebornUserRole::Member,
            UserContentAccessPolicy::Private,
            &admin.user_id,
        )
        .await
        .expect("private");
    let managed = store
        .create_user(
            &acme,
            None,
            Some("Managed".to_string()),
            RebornUserRole::Member,
            UserContentAccessPolicy::TenantAdminManaged,
            &admin.user_id,
        )
        .await
        .expect("managed");
    let directory: Arc<dyn RebornUserDirectory> = store.clone();
    let policy = crate::login_policy(directory);

    assert!(
        policy
            .authorize_admin_login_token_issuance(&acme, &admin.user_id)
            .await
            .expect("issuance policy")
    );
    assert!(
        !policy
            .authorize_admin_login_token_issuance(&other, &admin.user_id)
            .await
            .expect("cross-tenant issuance policy")
    );
    assert!(
        !policy
            .authorize_admin_login_token_issuance(&acme, &private.user_id)
            .await
            .expect("member issuance policy")
    );
    assert!(
        policy
            .authorize_reusable_login_token(&acme, &private.user_id)
            .await
            .expect("private login policy")
    );
    assert!(
        !policy
            .authorize_reusable_login_token(&acme, &managed.user_id)
            .await
            .expect("managed login policy")
    );
    assert!(
        !policy
            .authorize_reusable_login_token(&other, &private.user_id)
            .await
            .expect("cross-tenant login policy")
    );

    store
        .update_status(&private.user_id, RebornUserStatus::Suspended)
        .await
        .expect("suspend");
    assert!(
        !policy
            .authorize_reusable_login_token(&acme, &private.user_id)
            .await
            .expect("suspended login policy")
    );
    store
        .update_status(&admin.user_id, RebornUserStatus::Suspended)
        .await
        .expect("suspend admin");
    assert!(
        !policy
            .authorize_admin_login_token_issuance(&acme, &admin.user_id)
            .await
            .expect("suspended admin issuance policy")
    );
    store
        .delete_user(&acme, &private.user_id)
        .await
        .expect("delete");
    assert!(
        !policy
            .authorize_reusable_login_token(&acme, &private.user_id)
            .await
            .expect("deleted login policy")
    );
}
