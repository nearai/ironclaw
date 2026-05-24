//! First-party userland extension implementations for IronClaw.
//!
//! This crate owns concrete implementation behavior. Host runtime and
//! composition own declaration, authorization, accounting, lifecycle, and
//! loop-facing adapter wiring.
#![forbid(unsafe_code)]

mod gsuite;
pub mod coding;
pub mod skills;

pub use gsuite::{
    CALENDAR_ADD_ATTENDEES_CAPABILITY_ID, CALENDAR_CREATE_EVENT_CAPABILITY_ID,
    CALENDAR_DELETE_EVENT_CAPABILITY_ID, CALENDAR_EXTENSION_ID,
    CALENDAR_FIND_FREE_SLOTS_CAPABILITY_ID, CALENDAR_GET_EVENT_CAPABILITY_ID,
    CALENDAR_LIST_CALENDARS_CAPABILITY_ID, CALENDAR_LIST_EVENTS_CAPABILITY_ID,
    CALENDAR_SET_REMINDER_CAPABILITY_ID, CALENDAR_UPDATE_EVENT_CAPABILITY_ID, GMAIL_CREATE_DRAFT_CAPABILITY_ID,
    GMAIL_EXTENSION_ID, GMAIL_GET_MESSAGE_CAPABILITY_ID, GMAIL_LIST_MESSAGES_CAPABILITY_ID,
    GMAIL_REPLY_TO_MESSAGE_CAPABILITY_ID, GMAIL_SEND_MESSAGE_CAPABILITY_ID,
    GMAIL_TRASH_MESSAGE_CAPABILITY_ID, GOOGLE_PROVIDER_ID, GoogleCredential,
    GoogleCredentialError, GoogleCredentialResolver, GSUITE_OUTPUT_BYTES_LIMIT,
    GSUITE_RESPONSE_BODY_LIMIT, GSUITE_TIMEOUT_MS, GsuiteCapabilitySpec, GsuiteDispatchError,
    GsuiteDispatchRequest, GsuiteDispatchResult, GsuiteExecutor, GsuitePackageSpec,
    calendar_package_spec, gmail_package_spec, google_api_network_policy, google_provider_id,
    gsuite_package_specs, gsuite_resource_profile,
};
