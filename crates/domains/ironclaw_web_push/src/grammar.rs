//! The web-push channel's identity grammar: extension/channel names, the
//! constant owner-scoped catalog target id, and the reply-target binding-ref
//! format. Both halves of the grammar (encode + decode) live here so the
//! channel package's codec, target provider, and adapter cannot drift from
//! each other.

use ironclaw_host_api::ids::{TenantId, UserId};
use ironclaw_host_api::turn::ReplyTargetBindingRef;

use crate::error::WebPushError;

/// The extension identity the channel binding, manifest, and delivery
/// resolution share. Deliberately distinct from the retired `builtin:web_app`
/// pseudo-target: this channel is real external egress to push services.
pub const WEB_PUSH_EXTENSION_ID: &str = "web-push";

/// Catalog channel label shown beside the target in pickers.
pub const WEB_PUSH_CHANNEL_NAME: &str = "web-push";

/// The constant, owner-scoped catalog target id. Target resolution is always
/// scoped to the requesting owner, so one stable id per user is unambiguous
/// ("this user's enrolled browsers").
pub const WEB_PUSH_TARGET_ID: &str = "web-push";

/// Credential handle for the deployment's VAPID key material, referenced by
/// the manifest's `[[channel.egress]]` declarations and seeded at boot.
pub const WEB_PUSH_VAPID_CREDENTIAL_HANDLE: &str = "web_push_vapid";

const REF_PREFIX: &str = "web-push/v1/";

/// Encode the reply-target binding ref for one user's browsers:
/// `web-push/v1/<tenant>/<user>`.
pub fn encode_web_push_target_ref(
    tenant_id: &TenantId,
    user_id: &UserId,
) -> Result<ReplyTargetBindingRef, WebPushError> {
    let tenant = tenant_id.to_string();
    if tenant.contains('/') {
        // Tenant ids never carry '/' today; refuse rather than mint an
        // ambiguous ref if that ever changes.
        return Err(WebPushError::InvalidScope {
            reason: "tenant id cannot be encoded into a web-push target ref".to_string(),
        });
    }
    ReplyTargetBindingRef::new(format!("{REF_PREFIX}{tenant}/{user_id}")).map_err(|error| {
        WebPushError::InvalidScope {
            reason: format!("web-push target ref rejected: {error}"),
        }
    })
}

/// Decode a binding ref minted by [`encode_web_push_target_ref`].
pub fn decode_web_push_target_ref(reference: &str) -> Option<(TenantId, UserId)> {
    let remainder = reference.strip_prefix(REF_PREFIX)?;
    let (tenant, user) = remainder.split_once('/')?;
    let tenant_id = TenantId::new(tenant).ok()?;
    let user_id = UserId::new(user).ok()?;
    Some((tenant_id, user_id))
}

/// Whether a binding ref belongs to the web-push grammar at all.
pub fn is_web_push_target_ref(reference: &str) -> bool {
    reference.starts_with(REF_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_refs_round_trip() {
        let tenant = TenantId::new("tenant1").expect("tenant");
        let user = UserId::new("user-42").expect("user");
        let reference = encode_web_push_target_ref(&tenant, &user).expect("encode");
        assert_eq!(reference.as_str(), "web-push/v1/tenant1/user-42");
        let (decoded_tenant, decoded_user) =
            decode_web_push_target_ref(reference.as_str()).expect("decode");
        assert_eq!(decoded_tenant, tenant);
        assert_eq!(decoded_user, user);
        assert!(is_web_push_target_ref(reference.as_str()));
    }

    #[test]
    fn foreign_refs_do_not_decode() {
        for foreign in [
            "slack/v1/team/channel",
            "web-push/v2/tenant1/user",
            "web-push/v1/",
            "web-push/v1/tenant-only",
            "builtin:web_app",
        ] {
            assert!(
                decode_web_push_target_ref(foreign).is_none(),
                "{foreign:?} must not decode"
            );
        }
        assert!(!is_web_push_target_ref("slack/v1/team/channel"));
    }
}
