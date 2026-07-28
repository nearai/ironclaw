use ironclaw_host_api::{
    EffectKind, ExtensionId, PermissionMode, ResourceCeiling, ResourceEstimate, ResourceProfile,
    SandboxQuota,
};

pub const CALENDAR_EXTENSION_ID: &str = "google-calendar";
pub const GOOGLE_DOCS_EXTENSION_ID: &str = "google-docs";
pub const GOOGLE_DRIVE_EXTENSION_ID: &str = "google-drive";
pub const GOOGLE_SHEETS_EXTENSION_ID: &str = "google-sheets";
pub const GOOGLE_SLIDES_EXTENSION_ID: &str = "google-slides";
pub const GMAIL_EXTENSION_ID: &str = "gmail";

pub const GSUITE_RESPONSE_BODY_LIMIT: u64 = 1024 * 1024;
pub const GSUITE_REQUEST_BODY_LIMIT: usize = 64 * 1024;
pub const GSUITE_OUTPUT_BYTES_LIMIT: u64 = GSUITE_RESPONSE_BODY_LIMIT + 4096;
pub const GSUITE_TIMEOUT_MS: u32 = 30_000;
const DEFAULT_NETWORK_EGRESS_BYTES: u64 = 16 * 1024;
const MAX_NETWORK_EGRESS_BYTES: u64 = 512 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GsuitePackageSpec {
    pub extension_id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub service: &'static str,
    pub schema_prefix: &'static str,
    pub credential_handle: &'static str,
    pub credential_host_pattern: &'static str,
    pub capabilities: &'static [GsuiteCapabilitySpec],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GsuiteCapabilitySpec {
    pub id: &'static str,
    pub short_name: &'static str,
    pub description: &'static str,
    pub default_permission: PermissionMode,
    pub effects: &'static [EffectKind],
    pub required_scopes: &'static [&'static str],
    pub operation: GsuiteCapabilityOperation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GsuiteCapabilityOperation {
    CalendarListCalendars,
    CalendarListEvents,
    CalendarGetEvent,
    CalendarFindFreeSlots,
    CalendarCreateEvent,
    CalendarUpdateEvent,
    CalendarDeleteEvent,
    CalendarAddAttendees,
    CalendarSetReminder,
    GmailListMessages,
    GmailGetMessage,
    GmailSendMessage,
    GmailCreateDraft,
    GmailReplyToMessage,
    GmailTrashMessage,
}

const CALENDAR_CAPABILITIES: &[GsuiteCapabilitySpec] = &[
    GsuiteCapabilitySpec {
        id: "google-calendar.list_calendars",
        short_name: "list_calendars",
        description: "List Google calendars.",
        default_permission: PermissionMode::Allow,
        effects: READ_EFFECTS,
        required_scopes: CALENDAR_READONLY_SCOPES,
        operation: GsuiteCapabilityOperation::CalendarListCalendars,
    },
    GsuiteCapabilitySpec {
        id: "google-calendar.list_events",
        short_name: "list_events",
        description: "List Google Calendar events. Defaults to upcoming expanded events ordered by start time; use include_all_calendars or calendar_ids to cover more than the primary calendar.",
        default_permission: PermissionMode::Allow,
        effects: READ_EFFECTS,
        required_scopes: CALENDAR_READONLY_SCOPES,
        operation: GsuiteCapabilityOperation::CalendarListEvents,
    },
    GsuiteCapabilitySpec {
        id: "google-calendar.get_event",
        short_name: "get_event",
        description: "Get a Google Calendar event.",
        default_permission: PermissionMode::Allow,
        effects: READ_EFFECTS,
        required_scopes: CALENDAR_READONLY_SCOPES,
        operation: GsuiteCapabilityOperation::CalendarGetEvent,
    },
    GsuiteCapabilitySpec {
        id: "google-calendar.find_free_slots",
        short_name: "find_free_slots",
        description: "Find Google Calendar free slots.",
        default_permission: PermissionMode::Allow,
        effects: READ_EFFECTS,
        required_scopes: CALENDAR_READONLY_SCOPES,
        operation: GsuiteCapabilityOperation::CalendarFindFreeSlots,
    },
    GsuiteCapabilitySpec {
        id: "google-calendar.create_event",
        short_name: "create_event",
        description: "Create a Google Calendar event.",
        default_permission: PermissionMode::Ask,
        effects: WRITE_EFFECTS,
        required_scopes: CALENDAR_EVENTS_SCOPES,
        operation: GsuiteCapabilityOperation::CalendarCreateEvent,
    },
    GsuiteCapabilitySpec {
        id: "google-calendar.update_event",
        short_name: "update_event",
        description: "Update a Google Calendar event.",
        default_permission: PermissionMode::Ask,
        effects: WRITE_EFFECTS,
        required_scopes: CALENDAR_EVENTS_SCOPES,
        operation: GsuiteCapabilityOperation::CalendarUpdateEvent,
    },
    GsuiteCapabilitySpec {
        id: "google-calendar.delete_event",
        short_name: "delete_event",
        description: "Delete a Google Calendar event.",
        default_permission: PermissionMode::Ask,
        effects: WRITE_EFFECTS,
        required_scopes: CALENDAR_EVENTS_SCOPES,
        operation: GsuiteCapabilityOperation::CalendarDeleteEvent,
    },
    GsuiteCapabilitySpec {
        id: "google-calendar.add_attendees",
        short_name: "add_attendees",
        description: "Add attendees to a Google Calendar event.",
        default_permission: PermissionMode::Ask,
        effects: WRITE_EFFECTS,
        required_scopes: CALENDAR_EVENTS_SCOPES,
        operation: GsuiteCapabilityOperation::CalendarAddAttendees,
    },
    GsuiteCapabilitySpec {
        id: "google-calendar.set_reminder",
        short_name: "set_reminder",
        description: "Set Google Calendar event reminders.",
        default_permission: PermissionMode::Ask,
        effects: WRITE_EFFECTS,
        required_scopes: CALENDAR_EVENTS_SCOPES,
        operation: GsuiteCapabilityOperation::CalendarSetReminder,
    },
];

const GMAIL_CAPABILITIES: &[GsuiteCapabilitySpec] = &[
    GsuiteCapabilitySpec {
        id: "gmail.list_messages",
        short_name: "list_messages",
        description: "List Gmail messages.",
        default_permission: PermissionMode::Allow,
        effects: READ_EFFECTS,
        required_scopes: GMAIL_READONLY_SCOPES,
        operation: GsuiteCapabilityOperation::GmailListMessages,
    },
    GsuiteCapabilitySpec {
        id: "gmail.get_message",
        short_name: "get_message",
        description: "Get a Gmail message.",
        default_permission: PermissionMode::Allow,
        effects: READ_EFFECTS,
        required_scopes: GMAIL_READONLY_SCOPES,
        operation: GsuiteCapabilityOperation::GmailGetMessage,
    },
    GsuiteCapabilitySpec {
        id: "gmail.send_message",
        short_name: "send_message",
        description: "Send a Gmail message.",
        default_permission: PermissionMode::Ask,
        effects: WRITE_EFFECTS,
        required_scopes: GMAIL_SEND_SCOPES,
        operation: GsuiteCapabilityOperation::GmailSendMessage,
    },
    GsuiteCapabilitySpec {
        id: "gmail.create_draft",
        short_name: "create_draft",
        description: "Create a Gmail draft.",
        default_permission: PermissionMode::Ask,
        effects: WRITE_EFFECTS,
        required_scopes: GMAIL_MODIFY_SCOPES,
        operation: GsuiteCapabilityOperation::GmailCreateDraft,
    },
    GsuiteCapabilitySpec {
        id: "gmail.reply_to_message",
        short_name: "reply_to_message",
        description: "Reply to a Gmail message.",
        default_permission: PermissionMode::Ask,
        effects: WRITE_EFFECTS,
        required_scopes: GMAIL_SEND_SCOPES,
        operation: GsuiteCapabilityOperation::GmailReplyToMessage,
    },
    GsuiteCapabilitySpec {
        id: "gmail.trash_message",
        short_name: "trash_message",
        description: "Move a Gmail message to trash.",
        default_permission: PermissionMode::Ask,
        effects: WRITE_EFFECTS,
        required_scopes: GMAIL_MODIFY_SCOPES,
        operation: GsuiteCapabilityOperation::GmailTrashMessage,
    },
];

const READ_EFFECTS: &[EffectKind] = &[
    EffectKind::DispatchCapability,
    EffectKind::Network,
    EffectKind::UseSecret,
];
const WRITE_EFFECTS: &[EffectKind] = &[
    EffectKind::DispatchCapability,
    EffectKind::Network,
    EffectKind::UseSecret,
    EffectKind::ExternalWrite,
];
const CALENDAR_READONLY_SCOPES: &[&str] = &[ironclaw_auth::GOOGLE_CALENDAR_READONLY_SCOPE];
const CALENDAR_EVENTS_SCOPES: &[&str] = &[ironclaw_auth::GOOGLE_CALENDAR_EVENTS_SCOPE];
const GMAIL_READONLY_SCOPES: &[&str] = &[ironclaw_auth::GOOGLE_GMAIL_READONLY_SCOPE];
const GMAIL_SEND_SCOPES: &[&str] = &[ironclaw_auth::GOOGLE_GMAIL_SEND_SCOPE];
const GMAIL_MODIFY_SCOPES: &[&str] = &[ironclaw_auth::GOOGLE_GMAIL_MODIFY_SCOPE];
pub const GSUITE_PROVIDER_SCOPES: &[&str] = &[
    ironclaw_auth::GOOGLE_CALENDAR_READONLY_SCOPE,
    ironclaw_auth::GOOGLE_CALENDAR_EVENTS_SCOPE,
    ironclaw_auth::GOOGLE_GMAIL_READONLY_SCOPE,
    ironclaw_auth::GOOGLE_GMAIL_SEND_SCOPE,
    ironclaw_auth::GOOGLE_GMAIL_MODIFY_SCOPE,
];
pub const GSUITE_EXTENSION_IDS: &[&str] = &[
    CALENDAR_EXTENSION_ID,
    GMAIL_EXTENSION_ID,
    GOOGLE_DOCS_EXTENSION_ID,
    GOOGLE_DRIVE_EXTENSION_ID,
    GOOGLE_SHEETS_EXTENSION_ID,
    GOOGLE_SLIDES_EXTENSION_ID,
];

pub fn is_gsuite_extension_id(extension: &ExtensionId) -> bool {
    GSUITE_EXTENSION_IDS.contains(&extension.as_str())
}

pub fn gsuite_package_specs() -> &'static [GsuitePackageSpec] {
    &GSUITE_PACKAGE_SPECS
}

pub fn find_gsuite_capability(
    id: &str,
) -> Option<(&'static GsuitePackageSpec, &'static GsuiteCapabilitySpec)> {
    gsuite_package_specs().iter().find_map(|package| {
        package
            .capabilities
            .iter()
            .find(|capability| capability.id == id)
            .map(|capability| (package, capability))
    })
}

const GSUITE_PACKAGE_SPECS: [GsuitePackageSpec; 2] =
    [calendar_package_spec(), gmail_package_spec()];

pub const fn calendar_package_spec() -> GsuitePackageSpec {
    GsuitePackageSpec {
        extension_id: CALENDAR_EXTENSION_ID,
        name: "Google Calendar",
        description: "First-party Google Calendar capabilities for Reborn.",
        service: "google-calendar",
        schema_prefix: "google-calendar",
        credential_handle: "google_calendar_account",
        credential_host_pattern: "www.googleapis.com",
        capabilities: CALENDAR_CAPABILITIES,
    }
}

pub const fn gmail_package_spec() -> GsuitePackageSpec {
    GsuitePackageSpec {
        extension_id: GMAIL_EXTENSION_ID,
        name: "Gmail",
        description: "First-party Gmail capabilities for Reborn.",
        service: "gmail",
        schema_prefix: "gmail",
        credential_handle: "gmail_account",
        credential_host_pattern: "gmail.googleapis.com",
        capabilities: GMAIL_CAPABILITIES,
    }
}

pub fn gsuite_resource_profile() -> ResourceProfile {
    ResourceProfile {
        default_estimate: ResourceEstimate::default()
            .set_wall_clock_ms(u64::from(GSUITE_TIMEOUT_MS))
            .set_output_bytes(GSUITE_OUTPUT_BYTES_LIMIT)
            .set_network_egress_bytes(DEFAULT_NETWORK_EGRESS_BYTES),
        hard_ceiling: Some(ResourceCeiling {
            max_usd: None,
            max_input_tokens: None,
            max_output_tokens: None,
            max_wall_clock_ms: Some(u64::from(GSUITE_TIMEOUT_MS)),
            max_output_bytes: Some(GSUITE_OUTPUT_BYTES_LIMIT),
            sandbox: Some(SandboxQuota {
                network_egress_bytes: Some(MAX_NETWORK_EGRESS_BYTES),
                ..SandboxQuota::default()
            }),
        }),
    }
}
