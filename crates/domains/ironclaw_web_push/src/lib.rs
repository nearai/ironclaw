//! Web Push domain: subscription records and storage, RFC 8291 payload
//! encryption, VAPID key material generation, and push request planning for
//! the web app's browser-notification channel.
//!
//! Boundaries (family rules apply):
//! - **No transport.** This crate plans requests (`WebPushRequestPlan`) and
//!   never sends them; the channel adapter drives restricted egress and the
//!   host injects the `Authorization: vapid` header at the egress credential
//!   boundary.
//! - **No secret custody.** VAPID material is generated here, stored by
//!   composition through the channel credential path, and only ever read
//!   back by the host egress injector.
//! - **Storage** rides the scoped filesystem plane through the shared
//!   bounded CAS path; composition chooses backends.

pub mod crypto;
pub mod error;
pub mod grammar;
pub mod message;
pub mod runtime;
pub mod store;
pub mod subscription;
pub mod vapid;

pub use crypto::{MAX_ENCRYPTED_BODY_BYTES, MAX_PLAINTEXT_BYTES, encrypt_payload};
pub use error::WebPushError;
pub use grammar::{
    WEB_PUSH_CHANNEL_NAME, WEB_PUSH_EXTENSION_ID, WEB_PUSH_TARGET_ID,
    WEB_PUSH_VAPID_CREDENTIAL_HANDLE, decode_web_push_target_ref, encode_web_push_target_ref,
    is_web_push_target_ref,
};
pub use message::{
    DEFAULT_TTL_SECONDS, PushUrgency, WebPushNotificationPayload, WebPushRequestPlan,
    build_push_request,
};
pub use runtime::{WebPushRuntime, WebPushRuntimeSlot};
pub use store::{
    FilesystemWebPushSubscriptionStore, PushSubscriptionUpsertOutcome, WebPushSubscriptionStore,
};
pub use subscription::{
    MAX_SUBSCRIPTIONS_PER_USER, PushEndpoint, PushSubscriptionKeys, PushSubscriptionRecord,
};
pub use vapid::{GeneratedVapidKeyMaterial, generate_vapid_key_material, validate_vapid_subject};
