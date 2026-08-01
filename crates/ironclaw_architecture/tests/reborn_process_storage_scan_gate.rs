#[allow(dead_code)]
mod ratchet_support;

use std::path::Path;

use ratchet_support::workspace_root;

#[test]
fn process_and_thread_request_storage_paths_do_not_enumerate_collections() {
    let root = workspace_root();
    let process_store = scannable(&read(
        &root.join("crates/ironclaw_processes/src/journal_store.rs"),
    ));
    let thread_index = scannable(&read(
        &root.join("crates/ironclaw_threads/src/filesystem_service/thread_index.rs"),
    ));
    // The transcript rebuild moved out of `thread_index` into its own module;
    // the enumeration it performs is still migration-only and is gated below.
    let transcript_migration =
        read(&root.join("crates/ironclaw_threads/src/filesystem_service/transcript_migration.rs"));

    assert_calls_are_confined_to(
        &process_store,
        ".query(",
        "pub async fn migrate_row_native_indexes",
        "async fn initialize_materialized",
    );
    assert_calls_are_confined_to(
        &process_store,
        ".tail_bounded(",
        "async fn initialize_materialized",
        "\n}\n\n#[async_trait]",
    );
    assert_calls_are_confined_to(
        &thread_index,
        ".list_dir(",
        "pub async fn migrate_thread_index_for_scope",
        "pub(super) async fn thread_record_with_index_overlay",
    );
    // The listing projection module no longer enumerates at all: the only
    // `.query(` it had belonged to the transcript rebuild, which now lives in
    // its own module. Assert the absence directly rather than bounding it.
    assert!(
        !thread_index.contains(".query("),
        "thread_index must not enumerate collections; the transcript rebuild owns that call"
    );
    assert_calls_are_confined_to(
        &transcript_migration,
        ".query(",
        "pub async fn migrate_transcript_indexes_for_scope",
        "async fn migrate_transcript_page",
    );
    assert_calls_are_confined_to(
        &transcript_migration,
        ".list_dir(",
        "pub async fn migrate_transcript_indexes_for_scope",
        "async fn migrate_transcript_page",
    );
}

/// Scan a source with comments and string literals removed.
///
/// The gate matches call text, so prose that names `.query(` — a doc comment
/// explaining why enumeration is confined, for instance — would otherwise read
/// as a violation, and a call hidden inside a string would read as compliant.
fn scannable(source: &str) -> String {
    ratchet_support::strip_comments_and_strings(source)
}

fn assert_calls_are_confined_to(source: &str, call: &str, start: &str, end: &str) {
    let start_offset = source
        .find(start)
        .unwrap_or_else(|| panic!("migration boundary `{start}` must exist"));
    let end_offset = source[start_offset..]
        .find(end)
        .map(|offset| start_offset + offset)
        .unwrap_or_else(|| panic!("migration boundary `{end}` must exist after `{start}`"));

    let violations = source
        .match_indices(call)
        .filter(|(offset, _)| *offset < start_offset || *offset >= end_offset)
        .map(|(offset, _)| line_number(source, offset))
        .collect::<Vec<_>>();
    assert!(
        violations.is_empty(),
        "`{call}` may only appear in the explicit offline migration boundary \
         `{start}`..`{end}`; found request/startup uses on lines {violations:?}"
    );
}

fn line_number(source: &str, offset: usize) -> usize {
    source[..offset]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

/// Guardrails are code: the confinement scan needs its own coverage, or a
/// change to it can silently stop catching the thing it exists for.
#[cfg(test)]
mod self_tests {
    use super::*;

    const MIGRATION: &str = "pub async fn migrate_x() {\n    self.fs.query();\n}\n";
    const BOUNDARY: &str = "async fn after_migration() {}\n";

    #[test]
    fn a_call_inside_the_migration_range_is_allowed() {
        let source = scannable(&format!("{MIGRATION}{BOUNDARY}"));
        assert_calls_are_confined_to(
            &source,
            ".query(",
            "pub async fn migrate_x",
            "async fn after_migration",
        );
    }

    #[test]
    #[should_panic(expected = "may only appear in the explicit offline migration")]
    fn a_call_outside_the_migration_range_is_rejected() {
        let source = scannable(&format!(
            "async fn request_path() {{\n    self.fs.query();\n}}\n{MIGRATION}{BOUNDARY}"
        ));
        assert_calls_are_confined_to(
            &source,
            ".query(",
            "pub async fn migrate_x",
            "async fn after_migration",
        );
    }

    #[test]
    fn prose_naming_the_call_is_not_a_call_site() {
        // The doc comment and the string both name `.query(` without calling
        // it; only stripping them keeps this from reading as a violation.
        let source = scannable(&format!(
            "/// Enumeration via .query( belongs to the migration.\n\
             async fn request_path() {{\n    let _ = \".query(\";\n}}\n{MIGRATION}{BOUNDARY}"
        ));
        assert_calls_are_confined_to(
            &source,
            ".query(",
            "pub async fn migrate_x",
            "async fn after_migration",
        );
    }
}
