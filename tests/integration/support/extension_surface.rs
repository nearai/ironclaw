//! Static Reborn extension capability surface used by binary-E2E tests.
//!
//! **Not production truth.** `EXTENSION_LIFECYCLE_CAPABILITY_IDS` and
//! `BUNDLED_EXTENSION_CAPABILITY_IDS` below are hand-transcribed test-support
//! literals — they duplicate (fully or partially) values that live in a
//! production crate, but are not themselves parsed or imported from one.
//! `tests/integration/wiring_parity.rs`'s capability-id subset check no
//! longer unions either constant into its production-surface RHS; see that
//! file's module doc for why (W5-WIRING-PARITY finding 1). They remain here
//! for the unrelated QA-smoke scripted-scenario assertions
//! (`tests/reborn_qa_smoke_scenarios_e2e.rs`) that still use them as fixed
//! literals to script a harness's own declared surface, not to verify it
//! against production.
//!
//! `bundled_extension_manifest_capability_ids` below IS production truth —
//! it parses the real `manifest.toml` assets bundled extensions ship with,
//! the same way `github::capability_ids()` parses github's.

pub const EXTENSION_SEARCH_CAPABILITY_ID: &str = "builtin.extension_search";
pub const EXTENSION_INSTALL_CAPABILITY_ID: &str = "builtin.extension_install";
pub const EXTENSION_REMOVE_CAPABILITY_ID: &str = "builtin.extension_remove";

pub const EXTENSION_LIFECYCLE_CAPABILITY_IDS: &[&str] = &[
    EXTENSION_SEARCH_CAPABILITY_ID,
    EXTENSION_INSTALL_CAPABILITY_ID,
    EXTENSION_REMOVE_CAPABILITY_ID,
];

pub const BUNDLED_EXTENSION_IDS: &[&str] = &[
    "github",
    "web-access",
    "slack",
    "gmail",
    "google-calendar",
    "google-docs",
    "google-sheets",
    "google-drive",
    "google-slides",
    "nearai",
    "notion",
];

pub const BUNDLED_EXTENSION_CAPABILITY_IDS: &[&str] = &[
    "github.get_repo",
    "github.create_repo",
    "github.list_issues",
    "github.create_issue",
    "github.update_issue",
    "github.add_issue_labels",
    "github.remove_issue_label",
    "github.add_issue_assignees",
    "github.remove_issue_assignees",
    "github.get_issue",
    "github.list_issue_comments",
    "github.create_issue_comment",
    "github.comment_issue",
    "github.list_pull_requests",
    "github.create_pull_request",
    "github.update_pull_request",
    "github.get_pull_request",
    "github.get_pull_request_files",
    "github.create_pr_review",
    "github.list_pull_request_comments",
    "github.reply_pull_request_comment",
    "github.get_pull_request_reviews",
    "github.list_pull_request_review_threads",
    "github.resolve_review_thread",
    "github.unresolve_review_thread",
    "github.get_combined_status",
    "github.merge_pull_request",
    "github.get_authenticated_user",
    "github.list_repos",
    "github.search_repositories",
    "github.search_code",
    "github.search_issues",
    "github.search_issues_pull_requests",
    "github.list_branches",
    "github.create_branch",
    "github.get_file_content",
    "github.create_or_update_file",
    "github.delete_file",
    "github.list_releases",
    "github.create_release",
    "github.trigger_workflow",
    "github.get_workflow_runs",
    "github.get_workflow_run_jobs",
    "github.get_job_logs",
    "github.get_workflow_run_artifacts",
    "github.rerun_failed_workflow_run_jobs",
    "github.rerun_workflow_job",
    "github.fork_repo",
    "github.handle_webhook",
    "web-access.search",
    "web-access.get_content",
    "slack.search_messages",
    "slack.list_conversations",
    "slack.get_conversation_info",
    "slack.get_conversation_history",
    "slack.get_thread_replies",
    "slack.get_user_info",
    "slack.whoami",
    "slack.send_message",
    "gmail.list_messages",
    "gmail.get_message",
    "gmail.send_message",
    "gmail.create_draft",
    "gmail.reply_to_message",
    "gmail.trash_message",
    "google-calendar.list_calendars",
    "google-calendar.list_events",
    "google-calendar.get_event",
    "google-calendar.find_free_slots",
    "google-calendar.create_event",
    "google-calendar.update_event",
    "google-calendar.delete_event",
    "google-calendar.add_attendees",
    "google-calendar.set_reminder",
    "google-docs.create_document",
    "google-docs.get_document",
    "google-docs.read_content",
    "google-docs.insert_text",
    "google-docs.delete_content",
    "google-docs.replace_text",
    "google-docs.format_text",
    "google-docs.format_paragraph",
    "google-docs.insert_table",
    "google-docs.create_list",
    "google-docs.batch_update",
    "google-sheets.create_spreadsheet",
    "google-sheets.get_spreadsheet",
    "google-sheets.read_values",
    "google-sheets.batch_read_values",
    "google-sheets.write_values",
    "google-sheets.append_values",
    "google-sheets.clear_values",
    "google-sheets.add_sheet",
    "google-sheets.delete_sheet",
    "google-sheets.rename_sheet",
    "google-sheets.format_cells",
    "google-drive.list_files",
    "google-drive.get_file",
    "google-drive.download_file",
    "google-drive.upload_file",
    "google-drive.update_file",
    "google-drive.create_folder",
    "google-drive.delete_file",
    "google-drive.trash_file",
    "google-drive.share_file",
    "google-drive.list_permissions",
    "google-drive.remove_permission",
    "google-drive.list_shared_drives",
    "google-slides.create_presentation",
    "google-slides.get_presentation",
    "google-slides.get_thumbnail",
    "google-slides.create_slide",
    "google-slides.delete_object",
    "google-slides.insert_text",
    "google-slides.delete_text",
    "google-slides.replace_all_text",
    "google-slides.create_shape",
    "google-slides.insert_image",
    "google-slides.format_text",
    "google-slides.format_paragraph",
    "google-slides.replace_shapes_with_image",
    "google-slides.batch_update",
    // NEAR AI pins its core search capability so it is available before live
    // discovery. Notion remains discovery-only.
    "nearai.web_search",
];

/// Bundled first-party extension asset directories under
/// `crates/ironclaw_first_party_extensions/assets/`, parsed by
/// [`bundled_extension_manifest_capability_ids`]. Excludes `github` (parsed
/// separately by `github::capability_ids()`, which this list intentionally
/// does not duplicate).
const BUNDLED_EXTENSION_MANIFEST_ASSET_DIRS: &[&str] = &[
    "web-access",
    "slack",
    "gmail",
    "google-calendar",
    "google-docs",
    "google-sheets",
    "google-drive",
    "google-slides",
    "nearai-mcp",
    "notion-mcp",
];

/// Real capability ids declared by every non-github bundled first-party
/// extension's production `manifest.toml` asset — parsed the same way
/// `github::capability_ids()` parses github's
/// (`ExtensionManifest::parse` over the actual
/// shipped asset file), so this is production truth, not a second
/// hand-transcribed test-only id list like `BUNDLED_EXTENSION_CAPABILITY_IDS`
/// above.
pub fn bundled_extension_manifest_capability_ids()
-> Result<Vec<ironclaw_host_api::CapabilityId>, Box<dyn std::error::Error + Send + Sync>> {
    let mut registry = ironclaw_extensions::ExtensionRegistry::new();
    for dir_name in BUNDLED_EXTENSION_MANIFEST_ASSET_DIRS {
        let asset_root = repo_root()
            .join("crates/ironclaw_first_party_extensions/assets")
            .join(dir_name);
        // Parse through the single record entry point (the bundled assets
        // are manifest v3 documents since the first-party rewrite).
        let record = ironclaw_extensions::ExtensionManifestRecord::from_toml(
            std::fs::read_to_string(asset_root.join("manifest.toml"))?,
            ironclaw_extensions::ManifestSource::HostBundled,
            &ironclaw_host_runtime::default_host_port_catalog()?,
            None,
            &ironclaw_host_runtime::default_host_api_contract_registry()?,
        )?;
        let manifest = ironclaw_extensions::ExtensionManifest::try_from(record.manifest().clone())?;
        // The manifest's OWN `id` (not the asset directory name) must match
        // the `ExtensionPackage` root's last segment — they differ for
        // `nearai-mcp`/`notion-mcp` (manifest id `nearai`/`notion`).
        let extension_id = manifest.id.as_str().to_string();
        let package = ironclaw_extensions::ExtensionPackage::from_manifest(
            manifest,
            ironclaw_host_api::VirtualPath::new(format!("/system/extensions/{extension_id}"))?,
        )?;
        registry.insert(package)?;
    }
    Ok(registry
        .capabilities()
        .map(|descriptor| descriptor.id.clone())
        .collect())
}

fn repo_root() -> &'static std::path::Path {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
}
