use ironclaw_host_api::{
    EffectKind, PermissionMode, ResourceCeiling, ResourceEstimate, ResourceProfile, SandboxQuota,
};

pub const CALENDAR_EXTENSION_ID: &str = "google-calendar";
pub const GMAIL_EXTENSION_ID: &str = "gmail";

pub const GSUITE_RESPONSE_BODY_LIMIT: u64 = 1024 * 1024;
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
    pub capabilities: &'static [GsuiteCapabilitySpec],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GsuiteCapabilitySpec {
    pub short_name: &'static str,
    pub description: &'static str,
    pub default_permission: PermissionMode,
    pub effects: &'static [EffectKind],
}

const CALENDAR_CAPABILITIES: &[GsuiteCapabilitySpec] = &[
    GsuiteCapabilitySpec {
        short_name: "list_calendars",
        description: "List Google calendars.",
        default_permission: PermissionMode::Allow,
        effects: READ_EFFECTS,
    },
    GsuiteCapabilitySpec {
        short_name: "list_events",
        description: "List Google Calendar events.",
        default_permission: PermissionMode::Allow,
        effects: READ_EFFECTS,
    },
    GsuiteCapabilitySpec {
        short_name: "get_event",
        description: "Get a Google Calendar event.",
        default_permission: PermissionMode::Allow,
        effects: READ_EFFECTS,
    },
    GsuiteCapabilitySpec {
        short_name: "find_free_slots",
        description: "Find Google Calendar free slots.",
        default_permission: PermissionMode::Allow,
        effects: READ_EFFECTS,
    },
    GsuiteCapabilitySpec {
        short_name: "create_event",
        description: "Create a Google Calendar event.",
        default_permission: PermissionMode::Ask,
        effects: WRITE_EFFECTS,
    },
    GsuiteCapabilitySpec {
        short_name: "update_event",
        description: "Update a Google Calendar event.",
        default_permission: PermissionMode::Ask,
        effects: WRITE_EFFECTS,
    },
    GsuiteCapabilitySpec {
        short_name: "delete_event",
        description: "Delete a Google Calendar event.",
        default_permission: PermissionMode::Ask,
        effects: WRITE_EFFECTS,
    },
    GsuiteCapabilitySpec {
        short_name: "add_attendees",
        description: "Add attendees to a Google Calendar event.",
        default_permission: PermissionMode::Ask,
        effects: WRITE_EFFECTS,
    },
    GsuiteCapabilitySpec {
        short_name: "set_reminder",
        description: "Set Google Calendar event reminders.",
        default_permission: PermissionMode::Ask,
        effects: WRITE_EFFECTS,
    },
];

const GMAIL_CAPABILITIES: &[GsuiteCapabilitySpec] = &[
    GsuiteCapabilitySpec {
        short_name: "list_messages",
        description: "List Gmail messages.",
        default_permission: PermissionMode::Allow,
        effects: READ_EFFECTS,
    },
    GsuiteCapabilitySpec {
        short_name: "get_message",
        description: "Get a Gmail message.",
        default_permission: PermissionMode::Allow,
        effects: READ_EFFECTS,
    },
    GsuiteCapabilitySpec {
        short_name: "send_message",
        description: "Send a Gmail message.",
        default_permission: PermissionMode::Ask,
        effects: WRITE_EFFECTS,
    },
    GsuiteCapabilitySpec {
        short_name: "create_draft",
        description: "Create a Gmail draft.",
        default_permission: PermissionMode::Ask,
        effects: WRITE_EFFECTS,
    },
    GsuiteCapabilitySpec {
        short_name: "reply_to_message",
        description: "Reply to a Gmail message.",
        default_permission: PermissionMode::Ask,
        effects: WRITE_EFFECTS,
    },
    GsuiteCapabilitySpec {
        short_name: "trash_message",
        description: "Move a Gmail message to trash.",
        default_permission: PermissionMode::Ask,
        effects: WRITE_EFFECTS,
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

pub fn gsuite_package_specs() -> &'static [GsuitePackageSpec] {
    &GSUITE_PACKAGE_SPECS
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
        capabilities: GMAIL_CAPABILITIES,
    }
}

pub fn gsuite_resource_profile() -> ResourceProfile {
    ResourceProfile {
        default_estimate: ResourceEstimate {
            wall_clock_ms: Some(u64::from(GSUITE_TIMEOUT_MS)),
            output_bytes: Some(GSUITE_OUTPUT_BYTES_LIMIT),
            network_egress_bytes: Some(DEFAULT_NETWORK_EGRESS_BYTES),
            ..ResourceEstimate::default()
        },
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
