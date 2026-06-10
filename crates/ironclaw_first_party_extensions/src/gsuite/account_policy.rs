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
        CredentialOwnership::SharedAdminManaged => account
            .granted_extensions
            .iter()
            .any(is_gsuite_extension_id),
        CredentialOwnership::System => false,
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
