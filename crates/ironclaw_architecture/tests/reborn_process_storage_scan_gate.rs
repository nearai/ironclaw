#[allow(dead_code)]
mod ratchet_support;

use std::path::Path;

use ratchet_support::workspace_root;

#[test]
fn process_and_thread_request_storage_paths_do_not_enumerate_collections() {
    let root = workspace_root();
    let process_store = read(&root.join("crates/ironclaw_processes/src/journal_store.rs"));
    let thread_index =
        read(&root.join("crates/ironclaw_threads/src/filesystem_service/thread_index.rs"));

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
    assert_calls_are_confined_to(
        &thread_index,
        ".query(",
        "pub async fn migrate_transcript_indexes_for_scope",
        "pub(super) async fn thread_record_with_index_overlay",
    );
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
