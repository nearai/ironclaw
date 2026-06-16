//! First-party userland extension implementations for IronClaw.
//!
//! This crate owns concrete implementation behavior. Host runtime and
//! composition own declaration, authorization, accounting, lifecycle, and
//! loop-facing adapter wiring.
#![forbid(unsafe_code)]

pub mod coding;
mod gsuite;
pub mod skills;
pub mod web_access;

pub use gsuite::{
    CALENDAR_ADD_ATTENDEES_CAPABILITY_ID, CALENDAR_CREATE_EVENT_CAPABILITY_ID,
    CALENDAR_DELETE_EVENT_CAPABILITY_ID, CALENDAR_EXTENSION_ID,
    CALENDAR_FIND_FREE_SLOTS_CAPABILITY_ID, CALENDAR_GET_EVENT_CAPABILITY_ID,
    CALENDAR_LIST_CALENDARS_CAPABILITY_ID, CALENDAR_LIST_EVENTS_CAPABILITY_ID,
    CALENDAR_SET_REMINDER_CAPABILITY_ID, CALENDAR_UPDATE_EVENT_CAPABILITY_ID,
    GMAIL_CREATE_DRAFT_CAPABILITY_ID, GMAIL_EXTENSION_ID, GMAIL_GET_MESSAGE_CAPABILITY_ID,
    GMAIL_LIST_MESSAGES_CAPABILITY_ID, GMAIL_REPLY_TO_MESSAGE_CAPABILITY_ID,
    GMAIL_SEND_MESSAGE_CAPABILITY_ID, GMAIL_TRASH_MESSAGE_CAPABILITY_ID, GOOGLE_DOCS_EXTENSION_ID,
    GOOGLE_DRIVE_EXTENSION_ID, GOOGLE_SHEETS_EXTENSION_ID, GOOGLE_SLIDES_EXTENSION_ID,
    GSUITE_EXTENSION_IDS, GSUITE_OUTPUT_BYTES_LIMIT, GSUITE_PROVIDER_SCOPES,
    GSUITE_REQUEST_BODY_LIMIT, GSUITE_RESPONSE_BODY_LIMIT, GSUITE_TIMEOUT_MS, GoogleCredential,
    GoogleCredentialError, GoogleCredentialResolver, GsuiteCapabilityOperation,
    GsuiteCapabilitySpec, GsuiteCredentialDispatchReason, GsuiteCredentialStageError,
    GsuiteCredentialStageRequest, GsuiteCredentialStager, GsuiteDispatchError,
    GsuiteDispatchRequest, GsuiteDispatchResult, GsuiteExecutor, GsuitePackageSpec,
    calendar_package_spec, find_gsuite_capability, gmail_package_spec, google_api_network_policy,
    google_provider_id, gsuite_google_account_visible_to_requester, gsuite_network_policy_for,
    gsuite_package_specs, gsuite_resource_profile, is_gsuite_extension_id,
};
pub use web_access::{
    EXA_MCP_HOST, NETWORK_EGRESS_LIMIT, WEB_ACCESS_EXTENSION_ID, WEB_GET_CONTENT_CAPABILITY_ID,
    WEB_SEARCH_CAPABILITY_ID, WebAccessDispatchError, WebAccessDispatchRequest,
    WebAccessDispatchResult, WebAccessExecutor,
};
