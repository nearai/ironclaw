// Domain validation helpers are re-exported from `ironclaw_auth::domain` to
// avoid maintaining duplicate implementations.  All callers within this module
// continue to use `super::domain::{...}` imports unchanged.
pub(super) use ironclaw_auth::domain::{
    PreparedCallbackFlow, account_is_authorized_for_requester, prepare_callback_flow,
    recovery_projection_for_single_account, recovery_projection_for_unconfigured_accounts,
    update_account_from_exchange, update_account_from_request, validate_account_update_target,
    validate_bound_update_authority, validate_callback_claim,
    validate_credential_status_transition, validate_flow_update_binding,
    validate_manual_token_flow, validate_manual_token_update_binding,
    validate_new_credential_account, validate_selection_flow,
};
