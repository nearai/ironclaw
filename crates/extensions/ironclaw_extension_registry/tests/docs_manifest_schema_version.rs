//! Doc-fact contract: published docs teach the current manifest schema.
//!
//! Asserts no published page mentions the retired v2 schema literal (fenced
//! code included — a tutorial code block is where the drift lived), and that
//! the tool-building tutorial names the current schema version and
//! `origin_gate_matrix`. Scope is the published tree: `docs/` minus the
//! `.mintignore` fence, mirrored here as constants — that file is frozen
//! (remove-only, enforced by `scripts/ci/docs_publication_boundary.py`), so
//! the mirror cannot silently widen. Fenced areas may legitimately name the
//! retired literal.

use std::path::{Path, PathBuf};

use ironclaw_extension_registry::MANIFEST_SCHEMA_VERSION_V3;

const RETIRED_SCHEMA_LITERAL: &str = "reborn.extension_manifest.v2";

/// Mirror of the frozen `docs/.mintignore` fence.
const FENCED_PREFIXES: &[&str] = &["internal/", "drafts/"];
const FENCED_SUFFIX: &str = ".draft.mdx";

/// Fail-closed floor: ~130 published pages exist today. A walk that visits
/// fewer than this many means the walker broke, not that the docs shrank.
const MIN_SCANNED_PAGES: usize = 60;

fn repo_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        if dir.join("docs/docs.json").is_file() {
            return dir;
        }
        assert!(
            dir.pop(),
            "walked out of the filesystem without finding docs/docs.json"
        );
    }
}

fn published_pages(docs_root: &Path) -> Vec<PathBuf> {
    let mut pages = Vec::new();
    let mut stack = vec![docs_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)
            .unwrap_or_else(|error| panic!("read_dir {}: {error}", dir.display()))
        {
            let path = entry.expect("dir entry").path();
            let relative = path
                .strip_prefix(docs_root)
                .expect("path under docs root")
                .to_string_lossy()
                .replace('\\', "/");
            if FENCED_PREFIXES.iter().any(|prefix| {
                relative == prefix.trim_end_matches('/') || relative.starts_with(prefix)
            }) {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if (relative.ends_with(".md") || relative.ends_with(".mdx"))
                && !relative.ends_with(FENCED_SUFFIX)
            {
                pages.push(path);
            }
        }
    }
    pages.sort();
    pages
}

#[test]
fn published_docs_never_mention_the_retired_v2_schema() {
    let root = repo_root();
    let pages = published_pages(&root.join("docs"));
    assert!(
        pages.len() >= MIN_SCANNED_PAGES,
        "walked only {} published pages (floor is {MIN_SCANNED_PAGES}); the \
         walker or the fence mirror broke — refusing to verify almost nothing",
        pages.len(),
    );

    let mut offenders = Vec::new();
    for page in &pages {
        let text = std::fs::read_to_string(page)
            .unwrap_or_else(|error| panic!("read {}: {error}", page.display()));
        for (index, line) in text.lines().enumerate() {
            if line.contains(RETIRED_SCHEMA_LITERAL) {
                offenders.push(format!(
                    "{}:{}: {}",
                    page.strip_prefix(&root).expect("page under root").display(),
                    index + 1,
                    line.trim()
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "published docs still teach the retired `{RETIRED_SCHEMA_LITERAL}` schema \
         (current: `{MANIFEST_SCHEMA_VERSION_V3}`):\n{}",
        offenders.join("\n"),
    );
}

#[test]
fn tool_building_tutorial_teaches_the_current_schema_and_the_gate_matrix() {
    let page = repo_root().join("docs/extensions/building-a-tool.md");
    let text = std::fs::read_to_string(&page)
        .unwrap_or_else(|error| panic!("read {}: {error}", page.display()));
    assert!(
        text.contains(MANIFEST_SCHEMA_VERSION_V3),
        "docs/extensions/building-a-tool.md does not name the current schema \
         version `{MANIFEST_SCHEMA_VERSION_V3}`",
    );
    assert!(
        text.contains("origin_gate_matrix"),
        "docs/extensions/building-a-tool.md does not document `origin_gate_matrix`; \
         shipped packages must declare it (reborn_origin_gate_matrix_ratchet.rs), \
         so the tutorial teaching manifests without it produces broken extensions",
    );
}
