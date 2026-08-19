//! The web-app channel's identity grammar: extension/channel names, the
//! constant owner-scoped catalog target id, and the reply-target binding-ref
//! format. Both halves of the grammar (encode + decode) live here so the
//! channel package's codec, target provider, and adapter cannot drift from
//! each other.

use ironclaw_host_api::ids::{TenantId, UserId};
use ironclaw_host_api::turn::ReplyTargetBindingRef;

use crate::error::WebAppError;

/// The extension identity the channel binding, manifest, and delivery
/// resolution share. Renamed from `web-push` by the unified-channel-model
/// train (2026-08-10): the id names the product surface (the web app), not
/// the push protocol. Deliberately distinct from the retired
/// `builtin:web_app` pseudo-target string: this channel is real external
/// egress to push services.
pub const WEB_APP_EXTENSION_ID: &str = "web-app";

/// Catalog channel label shown beside the target in pickers.
pub const WEB_APP_CHANNEL_NAME: &str = "web-app";

/// The constant, owner-scoped catalog target id. Target resolution is always
/// scoped to the requesting owner, so one stable id per user is unambiguous
/// ("this user's enrolled browsers").
///
/// The VALUE keeps the pre-rename `web-push` spelling for the same reason
/// [`WEB_APP_VAPID_CREDENTIAL_HANDLE`] does: the notification-channel picker
/// persists its selection as catalog target ids in each user's communication
/// preferences, so this is a persisted per-user identity, not a display
/// label. Renaming it would resolve every stored selection to `Missing` and
/// silently drop those users from notification fan-out until they re-ticked
/// the box. The user-visible channel name is [`WEB_APP_CHANNEL_NAME`].
pub const WEB_APP_TARGET_ID: &str = "web-push";

/// Credential handle for the deployment's VAPID key material, referenced by
/// the manifest's `[[channel.egress]]` declarations and seeded at boot.
/// The VALUE keeps the pre-rename `web_push_vapid` spelling on purpose: it
/// is a persisted secret-store key, and renaming it would orphan every
/// deployment's seeded keypair — rotating the VAPID identity breaks all
/// existing browser subscriptions cryptographically.
pub const WEB_APP_VAPID_CREDENTIAL_HANDLE: &str = "web_push_vapid";

const REF_PREFIX: &str = "web-app/v1/";
/// Pre-rename binding refs persisted before 2026-08-10 (legacy notification
/// slots, historical delivery-attempt rows). Decoded forever; never minted.
const LEGACY_REF_PREFIX: &str = "web-push/v1/";

/// Encode the reply-target binding ref for one user's browsers:
/// `web-app/v1/<tenant>/<user>`.
pub fn encode_web_app_target_ref(
    tenant_id: &TenantId,
    user_id: &UserId,
) -> Result<ReplyTargetBindingRef, WebAppError> {
    let tenant = tenant_id.to_string();
    if tenant.contains('/') {
        // Tenant ids never carry '/' today; refuse rather than mint an
        // ambiguous ref if that ever changes.
        return Err(WebAppError::InvalidScope {
            reason: "tenant id cannot be encoded into a web-app target ref".to_string(),
        });
    }
    ReplyTargetBindingRef::new(format!("{REF_PREFIX}{tenant}/{user_id}")).map_err(|error| {
        WebAppError::InvalidScope {
            reason: format!("web-app target ref rejected: {error}"),
        }
    })
}

/// Decode a binding ref minted by [`encode_web_app_target_ref`], accepting
/// the pre-rename `web-push/v1/` prefix for refs persisted before the
/// 2026-08-10 identity rename.
pub fn decode_web_app_target_ref(reference: &str) -> Option<(TenantId, UserId)> {
    let remainder = reference
        .strip_prefix(REF_PREFIX)
        .or_else(|| reference.strip_prefix(LEGACY_REF_PREFIX))?;
    let (tenant, user) = remainder.split_once('/')?;
    let tenant_id = TenantId::new(tenant).ok()?;
    let user_id = UserId::new(user).ok()?;
    Some((tenant_id, user_id))
}

/// Whether a binding ref belongs to the web-app grammar at all (either
/// prefix era).
pub fn is_web_app_target_ref(reference: &str) -> bool {
    reference.starts_with(REF_PREFIX) || reference.starts_with(LEGACY_REF_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_refs_round_trip() {
        let tenant = TenantId::new("tenant1").expect("tenant");
        let user = UserId::new("user-42").expect("user");
        let reference = encode_web_app_target_ref(&tenant, &user).expect("encode");
        assert_eq!(reference.as_str(), "web-app/v1/tenant1/user-42");
        let (decoded_tenant, decoded_user) =
            decode_web_app_target_ref(reference.as_str()).expect("decode");
        assert_eq!(decoded_tenant, tenant);
        assert_eq!(decoded_user, user);
        assert!(is_web_app_target_ref(reference.as_str()));
    }

    /// Refs persisted before the 2026-08-10 rename keep decoding: a legacy
    /// notification slot or replayed delivery attempt carrying
    /// `web-push/v1/…` must resolve to the same scope forever.
    #[test]
    fn legacy_prefix_refs_still_decode() {
        let (tenant, user) =
            decode_web_app_target_ref("web-push/v1/tenant1/user-42").expect("legacy decode");
        assert_eq!(tenant, TenantId::new("tenant1").expect("tenant"));
        assert_eq!(user, UserId::new("user-42").expect("user"));
        assert!(is_web_app_target_ref("web-push/v1/tenant1/user-42"));
        // Minting stays on the new prefix only.
        let fresh = encode_web_app_target_ref(&tenant, &user).expect("encode");
        assert!(fresh.as_str().starts_with("web-app/v1/"));
    }

    #[test]
    fn foreign_refs_do_not_decode() {
        for foreign in [
            "slack/v1/team/channel",
            "web-app/v2/tenant1/user",
            "web-app/v1/",
            "web-app/v1/tenant-only",
            "builtin:web_app",
        ] {
            assert!(
                decode_web_app_target_ref(foreign).is_none(),
                "{foreign:?} must not decode"
            );
        }
        assert!(!is_web_app_target_ref("slack/v1/team/channel"));
    }
}
