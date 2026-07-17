//! Tiny shared golden-JSON-fixture helper for the SSE wire-contract
//! fixtures (`webui_v2_schema_contract.rs`'s
//! `sse_wire_contract_fixtures_match_committed_json` and
//! `webui_v2_handlers_contract.rs`'s facade-error fixture test).
//!
//! Deliberately not `insta` (already a workspace dependency and used this
//! way by `tests/integration/support/golden.rs`): these specific fixtures
//! are also read directly by the frontend Vitest suite
//! (`frontend/src/pages/chat/lib/sse-wire-contract.test.ts`) via
//! `JSON.parse`, so they must be plain JSON files with no YAML frontmatter,
//! which `insta`'s default `.snap` format carries.
#![allow(dead_code)]

use std::path::PathBuf;

use serde_json::Value;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/sse_wire_contract"
    ))
}

/// Compare `value` (pretty-printed, trailing newline) against the committed
/// fixture `<name>.json`. With `UPDATE_SSE_FIXTURES=1` set, writes/
/// overwrites the fixture instead of comparing — the regeneration path for
/// an intentional wire-schema change. Missing fixture + no
/// `UPDATE_SSE_FIXTURES` fails loudly rather than silently creating one, so
/// a typo'd fixture name cannot "pass" on its first run.
pub fn assert_or_update_json_fixture(name: &str, value: &Value) {
    let rendered = format!(
        "{}\n",
        serde_json::to_string_pretty(value).expect("fixture value serializes to JSON")
    );
    let path = fixtures_dir().join(format!("{name}.json"));

    if std::env::var_os("UPDATE_SSE_FIXTURES").is_some() {
        std::fs::create_dir_all(path.parent().expect("fixture path has a parent directory"))
            .expect("create SSE wire-contract fixtures directory");
        std::fs::write(&path, &rendered).expect("write SSE wire-contract fixture");
        return;
    }

    let committed = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "missing SSE wire-contract fixture {path:?}: {error}\n\
             run `UPDATE_SSE_FIXTURES=1 cargo test -p ironclaw_webui_v2` to create it, \
             review the diff, and commit it"
        )
    });
    assert_eq!(
        committed, rendered,
        "SSE wire-contract fixture {path:?} is out of date.\n\
         Run `UPDATE_SSE_FIXTURES=1 cargo test -p ironclaw_webui_v2` to regenerate, \
         review the diff, and commit it. The frontend Vitest suite \
         (frontend/src/pages/chat/lib/sse-wire-contract.test.ts) reads the same file — \
         re-run it too after regenerating."
    );
}
