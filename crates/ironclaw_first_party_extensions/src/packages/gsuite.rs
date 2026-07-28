//! Google Workspace (gsuite) package family — Calendar (gsuite executor, no
//! WASM) plus Docs / Drive / Sheets / Slides (WASM tools). Grouped in one
//! module because they share the `google_wasm_assets!` asset macro and the
//! gsuite OAuth effect grant; each still contributes a distinct package to the
//! inventory. Assets: per-operation input/output JSON schemas, prompt docs, and
//! (for the four WASM tools) the tool WASM module.

use std::borrow::Cow;

use ironclaw_host_api::EffectKind;

use super::{PackageAsset, PackageBundle, PackageOnboarding, bytes_asset};

pub(super) const CALENDAR_ID: &str = "google-calendar";
pub(super) const DOCS_ID: &str = "google-docs";
pub(super) const DRIVE_ID: &str = "google-drive";
pub(super) const SHEETS_ID: &str = "google-sheets";
pub(super) const SLIDES_ID: &str = "google-slides";

const CALENDAR_MANIFEST: &str = include_str!("../../assets/google-calendar/manifest.toml");
const DOCS_MANIFEST: &str = include_str!("../../assets/google-docs/manifest.toml");
const DOCS_WASM: &[u8] = include_bytes!("../../assets/google-docs/wasm/google_docs_tool.wasm");
const DRIVE_MANIFEST: &str = include_str!("../../assets/google-drive/manifest.toml");
const DRIVE_WASM: &[u8] = include_bytes!("../../assets/google-drive/wasm/google_drive_tool.wasm");
const SHEETS_MANIFEST: &str = include_str!("../../assets/google-sheets/manifest.toml");
const SHEETS_WASM: &[u8] =
    include_bytes!("../../assets/google-sheets/wasm/google_sheets_tool.wasm");
const SLIDES_MANIFEST: &str = include_str!("../../assets/google-slides/manifest.toml");
const SLIDES_WASM: &[u8] =
    include_bytes!("../../assets/google-slides/wasm/google_slides_tool.wasm");

/// gsuite-family host effect grant: OAuth-backed Google API access with writes.
fn trust_effects() -> Vec<EffectKind> {
    vec![
        EffectKind::DispatchCapability,
        EffectKind::Network,
        EffectKind::UseSecret,
        EffectKind::ExternalWrite,
    ]
}

macro_rules! google_wasm_assets {
    ($id:literal, $manifest:expr, $wasm_file:literal, $wasm_module:expr, [$($operation:literal),+ $(,)?]) => {{
        vec![
            bytes_asset("manifest.toml", $manifest.as_bytes()),
            bytes_asset(
                concat!("schemas/", $id, "/raw_output.v1.json"),
                include_bytes!(concat!(
                    "../../assets/",
                    $id,
                    "/schemas/",
                    $id,
                    "/raw_output.v1.json"
                )),
            ),
            $(
                bytes_asset(
                    concat!("schemas/", $id, "/", $operation, ".input.v1.json"),
                    include_bytes!(concat!(
                        "../../assets/",
                        $id,
                        "/schemas/",
                        $id,
                        "/",
                        $operation,
                        ".input.v1.json"
                    )),
                ),
                bytes_asset(
                    concat!("prompts/", $id, "/", $operation, ".md"),
                    include_bytes!(concat!(
                        "../../assets/",
                        $id,
                        "/prompts/",
                        $id,
                        "/",
                        $operation,
                        ".md"
                    )),
                ),
            )+
            bytes_asset(concat!("wasm/", $wasm_file), $wasm_module),
        ]
    }};
}

fn google_calendar_assets() -> Vec<PackageAsset> {
    vec![
        bytes_asset("manifest.toml", CALENDAR_MANIFEST.as_bytes()),
        bytes_asset(
            "schemas/google-calendar/list_calendars.input.v1.json",
            include_bytes!(
                "../../assets/google-calendar/schemas/google-calendar/list_calendars.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/list_calendars.output.v1.json",
            include_bytes!(
                "../../assets/google-calendar/schemas/google-calendar/list_calendars.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/list_events.input.v1.json",
            include_bytes!(
                "../../assets/google-calendar/schemas/google-calendar/list_events.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/list_events.output.v1.json",
            include_bytes!(
                "../../assets/google-calendar/schemas/google-calendar/list_events.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/get_event.input.v1.json",
            include_bytes!(
                "../../assets/google-calendar/schemas/google-calendar/get_event.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/get_event.output.v1.json",
            include_bytes!(
                "../../assets/google-calendar/schemas/google-calendar/get_event.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/find_free_slots.input.v1.json",
            include_bytes!(
                "../../assets/google-calendar/schemas/google-calendar/find_free_slots.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/find_free_slots.output.v1.json",
            include_bytes!(
                "../../assets/google-calendar/schemas/google-calendar/find_free_slots.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/create_event.input.v1.json",
            include_bytes!(
                "../../assets/google-calendar/schemas/google-calendar/create_event.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/create_event.output.v1.json",
            include_bytes!(
                "../../assets/google-calendar/schemas/google-calendar/create_event.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/update_event.input.v1.json",
            include_bytes!(
                "../../assets/google-calendar/schemas/google-calendar/update_event.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/update_event.output.v1.json",
            include_bytes!(
                "../../assets/google-calendar/schemas/google-calendar/update_event.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/delete_event.input.v1.json",
            include_bytes!(
                "../../assets/google-calendar/schemas/google-calendar/delete_event.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/delete_event.output.v1.json",
            include_bytes!(
                "../../assets/google-calendar/schemas/google-calendar/delete_event.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/add_attendees.input.v1.json",
            include_bytes!(
                "../../assets/google-calendar/schemas/google-calendar/add_attendees.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/add_attendees.output.v1.json",
            include_bytes!(
                "../../assets/google-calendar/schemas/google-calendar/add_attendees.output.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/set_reminder.input.v1.json",
            include_bytes!(
                "../../assets/google-calendar/schemas/google-calendar/set_reminder.input.v1.json"
            ),
        ),
        bytes_asset(
            "schemas/google-calendar/set_reminder.output.v1.json",
            include_bytes!(
                "../../assets/google-calendar/schemas/google-calendar/set_reminder.output.v1.json"
            ),
        ),
        bytes_asset(
            "prompts/google-calendar/list_calendars.md",
            include_bytes!(
                "../../assets/google-calendar/prompts/google-calendar/list_calendars.md"
            ),
        ),
        bytes_asset(
            "prompts/google-calendar/list_events.md",
            include_bytes!("../../assets/google-calendar/prompts/google-calendar/list_events.md"),
        ),
        bytes_asset(
            "prompts/google-calendar/get_event.md",
            include_bytes!("../../assets/google-calendar/prompts/google-calendar/get_event.md"),
        ),
        bytes_asset(
            "prompts/google-calendar/find_free_slots.md",
            include_bytes!(
                "../../assets/google-calendar/prompts/google-calendar/find_free_slots.md"
            ),
        ),
        bytes_asset(
            "prompts/google-calendar/create_event.md",
            include_bytes!("../../assets/google-calendar/prompts/google-calendar/create_event.md"),
        ),
        bytes_asset(
            "prompts/google-calendar/update_event.md",
            include_bytes!("../../assets/google-calendar/prompts/google-calendar/update_event.md"),
        ),
        bytes_asset(
            "prompts/google-calendar/delete_event.md",
            include_bytes!("../../assets/google-calendar/prompts/google-calendar/delete_event.md"),
        ),
        bytes_asset(
            "prompts/google-calendar/add_attendees.md",
            include_bytes!("../../assets/google-calendar/prompts/google-calendar/add_attendees.md"),
        ),
        bytes_asset(
            "prompts/google-calendar/set_reminder.md",
            include_bytes!("../../assets/google-calendar/prompts/google-calendar/set_reminder.md"),
        ),
    ]
}

pub(super) fn google_calendar_bundle() -> PackageBundle {
    PackageBundle {
        id: CALENDAR_ID,
        display_name: "Google Calendar",
        manifest_toml: Cow::Borrowed(CALENDAR_MANIFEST),
        assets: google_calendar_assets(),
        onboarding: Some(PackageOnboarding {
            instructions: "Google Calendar needs Google OAuth authorization before calendar tools \
                can run."
                .to_string(),
            credential_instructions: Some(
                "Authorize the Google account that IronClaw should use for Google Calendar."
                    .to_string(),
            ),
            setup_url: None,
            credential_next_step: "After authorization completes, IronClaw finishes Google \
                Calendar installation automatically and publishes its tools."
                .to_string(),
        }),
        trust_effects: Some(trust_effects()),
    }
}

pub(super) fn google_docs_bundle() -> PackageBundle {
    PackageBundle {
        id: DOCS_ID,
        display_name: "Google Docs",
        manifest_toml: Cow::Borrowed(DOCS_MANIFEST),
        assets: google_wasm_assets!(
            "google-docs",
            DOCS_MANIFEST,
            "google_docs_tool.wasm",
            DOCS_WASM,
            [
                "create_document",
                "get_document",
                "read_content",
                "insert_text",
                "delete_content",
                "replace_text",
                "format_text",
                "format_paragraph",
                "insert_table",
                "create_list",
                "batch_update"
            ]
        ),
        onboarding: None,
        trust_effects: Some(trust_effects()),
    }
}

pub(super) fn google_drive_bundle() -> PackageBundle {
    PackageBundle {
        id: DRIVE_ID,
        display_name: "Google Drive",
        manifest_toml: Cow::Borrowed(DRIVE_MANIFEST),
        assets: google_wasm_assets!(
            "google-drive",
            DRIVE_MANIFEST,
            "google_drive_tool.wasm",
            DRIVE_WASM,
            [
                "list_files",
                "get_file",
                "download_file",
                "upload_file",
                "update_file",
                "create_folder",
                "delete_file",
                "trash_file",
                "share_file",
                "list_permissions",
                "remove_permission",
                "list_shared_drives"
            ]
        ),
        onboarding: None,
        trust_effects: Some(trust_effects()),
    }
}

pub(super) fn google_sheets_bundle() -> PackageBundle {
    PackageBundle {
        id: SHEETS_ID,
        display_name: "Google Sheets",
        manifest_toml: Cow::Borrowed(SHEETS_MANIFEST),
        assets: google_wasm_assets!(
            "google-sheets",
            SHEETS_MANIFEST,
            "google_sheets_tool.wasm",
            SHEETS_WASM,
            [
                "create_spreadsheet",
                "get_spreadsheet",
                "read_values",
                "batch_read_values",
                "write_values",
                "append_values",
                "clear_values",
                "add_sheet",
                "delete_sheet",
                "rename_sheet",
                "format_cells"
            ]
        ),
        onboarding: None,
        trust_effects: Some(trust_effects()),
    }
}

pub(super) fn google_slides_bundle() -> PackageBundle {
    PackageBundle {
        id: SLIDES_ID,
        display_name: "Google Slides",
        manifest_toml: Cow::Borrowed(SLIDES_MANIFEST),
        assets: google_wasm_assets!(
            "google-slides",
            SLIDES_MANIFEST,
            "google_slides_tool.wasm",
            SLIDES_WASM,
            [
                "create_presentation",
                "get_presentation",
                "get_thumbnail",
                "create_slide",
                "delete_object",
                "insert_text",
                "delete_text",
                "replace_all_text",
                "create_shape",
                "insert_image",
                "format_text",
                "format_paragraph",
                "replace_shapes_with_image",
                "batch_update"
            ]
        ),
        onboarding: None,
        trust_effects: Some(trust_effects()),
    }
}
