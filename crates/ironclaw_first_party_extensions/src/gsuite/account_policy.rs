use ironclaw_auth::{CredentialAccount, CredentialOwnership};
use ironclaw_host_api::ExtensionId;

use super::manifest::is_gsuite_extension_id;

pub fn gsuite_google_account_visible_to_requester(
    account: &CredentialAccount,
    requester_extension: &ExtensionId,
) -> bool {
    if account_explicitly_bound_to_requester(account, requester_extension) {
        return true;
    }
    if !is_gsuite_extension_id(requester_extension) {
        return false;
    }
    google_account_available_to_gsuite_family(account)
}

fn google_account_available_to_gsuite_family(account: &CredentialAccount) -> bool {
    match account.ownership {
        CredentialOwnership::UserReusable => true,
        CredentialOwnership::ExtensionOwned => account
            .owner_extension
            .as_ref()
            .is_some_and(is_gsuite_extension_id),
        CredentialOwnership::SharedAdminManaged | CredentialOwnership::System => false,
    }
}

fn account_explicitly_bound_to_requester(
    account: &CredentialAccount,
    requester_extension: &ExtensionId,
) -> bool {
    match account.ownership {
        CredentialOwnership::ExtensionOwned => account
            .owner_extension
            .as_ref()
            .is_some_and(|owner_extension| owner_extension == requester_extension),
        CredentialOwnership::SharedAdminManaged => {
            account.granted_extensions.contains(requester_extension)
        }
        CredentialOwnership::UserReusable | CredentialOwnership::System => false,
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_auth::{
        AuthProductScope, AuthProviderId, AuthSurface, CredentialAccount, CredentialAccountId,
        CredentialAccountLabel, CredentialAccountStatus, CredentialOwnership, GOOGLE_PROVIDER_ID,
        Timestamp,
    };
    use ironclaw_host_api::{ExtensionId, InvocationId, ResourceScope, UserId};

    use super::gsuite_google_account_visible_to_requester;

    #[test]
    fn shared_admin_google_account_requires_exact_gsuite_grant() {
        let account = google_account(
            CredentialOwnership::SharedAdminManaged,
            None,
            vec![ExtensionId::new("gmail").unwrap()],
        );

        assert!(gsuite_google_account_visible_to_requester(
            &account,
            &ExtensionId::new("gmail").unwrap()
        ));
        assert!(!gsuite_google_account_visible_to_requester(
            &account,
            &ExtensionId::new("google-calendar").unwrap()
        ));
    }

    #[test]
    fn extension_owned_google_account_can_be_reused_within_gsuite_family() {
        let account = google_account(
            CredentialOwnership::ExtensionOwned,
            Some(ExtensionId::new("gmail").unwrap()),
            Vec::new(),
        );

        assert!(gsuite_google_account_visible_to_requester(
            &account,
            &ExtensionId::new("google-calendar").unwrap()
        ));
    }

    fn google_account(
        ownership: CredentialOwnership,
        owner_extension: Option<ExtensionId>,
        granted_extensions: Vec<ExtensionId>,
    ) -> CredentialAccount {
        let scope =
            ResourceScope::local_default(UserId::new("alice").unwrap(), InvocationId::new())
                .unwrap();
        let now = Timestamp::from_timestamp(0, 0).unwrap();
        CredentialAccount {
            id: CredentialAccountId::new(),
            scope: AuthProductScope::new(scope, AuthSurface::Api),
            provider: AuthProviderId::new(GOOGLE_PROVIDER_ID).unwrap(),
            label: CredentialAccountLabel::new("google").unwrap(),
            status: CredentialAccountStatus::Configured,
            ownership,
            owner_extension,
            granted_extensions,
            access_secret: None,
            refresh_secret: None,
            scopes: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }
}
