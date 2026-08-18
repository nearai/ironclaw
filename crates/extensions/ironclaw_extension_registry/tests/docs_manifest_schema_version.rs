//! Doc-fact contract: published docs teach the current manifest schema.
//!
//! Asserts every `reborn.extension_manifest.<version>` mention in a published
//! page names the current schema version (fenced code included — a tutorial
//! code block is where the drift lived), and that the tool-building tutorial
//! names the current schema version and `origin_gate_matrix`. Both the family
//! prefix and the expected version derive from `MANIFEST_SCHEMA_VERSION_V3`,
//! so the next schema bump retargets this scan through the constant instead
//! of a maintained retired-literal list. Scope is the published tree: `docs/`
//! minus the `.mintignore` fence, parsed from the authoritative file so a
//! removed fence entry widens this scan with it. Fenced areas may
//! legitimately name retired versions.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use ironclaw_extension_registry::MANIFEST_SCHEMA_VERSION_V3;

/// The schema family prefix (`reborn.extension_manifest.`), derived from the
/// current-version constant so a renamed family retargets the scan too.
fn schema_family_prefix() -> String {
    let (family, _version) = MANIFEST_SCHEMA_VERSION_V3
        .rsplit_once('.')
        .expect("schema version constant ends in a `.{version}` segment");
    format!("{family}.")
}

/// The publication fence, parsed from `docs/.mintignore`.
struct Fence {
    dir_prefixes: Vec<String>,
    suffixes: Vec<String>,
}

/// Only the syntax the repo's `.mintignore` actually uses (`dir/` and
/// `*.suffix`); anything else fails rather than being silently skipped.
fn parse_fence(docs_root: &Path) -> Fence {
    let path = docs_root.join(".mintignore");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let mut fence = Fence {
        dir_prefixes: Vec::new(),
        suffixes: Vec::new(),
    };
    for line in text.lines() {
        let pattern = line.trim();
        if pattern.is_empty() || pattern.starts_with('#') {
            continue;
        }
        if let Some(suffix) = pattern.strip_prefix("*.") {
            fence.suffixes.push(format!(".{suffix}"));
        } else if pattern.ends_with('/') && !pattern.contains(['*', '!']) {
            fence.dir_prefixes.push(pattern.to_string());
        } else {
            panic!("docs/.mintignore pattern {pattern:?} uses syntax this test does not model");
        }
    }
    assert!(
        !fence.dir_prefixes.is_empty(),
        "docs/.mintignore names no fenced directories — the fence moved or emptied"
    );
    fence
}

/// Extensionless page routes from docs.json navigation — the independent
/// definition of the published surface the walk is validated against
/// (instead of a count floor). Entries with spaces are generated OpenAPI
/// endpoint pages ("GET /users") with no source file.
fn nav_routes(docs_root: &Path) -> BTreeSet<String> {
    let path = docs_root.join("docs.json");
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display())),
    )
    .expect("parse docs/docs.json");
    let mut routes = BTreeSet::new();
    collect_nav_pages(&manifest["navigation"], &mut routes);
    routes.retain(|route| !route.contains(' '));
    assert!(
        !routes.is_empty(),
        "docs.json navigation names no source-backed pages — nothing to \
         validate the walk against; refusing rather than passing vacuously"
    );
    routes
}

fn collect_nav_pages(node: &serde_json::Value, out: &mut BTreeSet<String>) {
    match node {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                match (key.as_str(), value) {
                    ("pages", serde_json::Value::Array(entries)) => {
                        for entry in entries {
                            if let Some(page) = entry.as_str() {
                                out.insert(page.to_string());
                            } else {
                                collect_nav_pages(entry, out);
                            }
                        }
                    }
                    _ => collect_nav_pages(value, out),
                }
            }
        }
        serde_json::Value::Array(entries) => {
            for entry in entries {
                collect_nav_pages(entry, out);
            }
        }
        _ => {}
    }
}

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
    let fence = parse_fence(docs_root);
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
            if fence.dir_prefixes.iter().any(|prefix| {
                relative == prefix.trim_end_matches('/') || relative.starts_with(prefix)
            }) {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if (relative.ends_with(".md") || relative.ends_with(".mdx"))
                && !fence
                    .suffixes
                    .iter()
                    .any(|suffix| relative.ends_with(suffix))
            {
                pages.push(path);
            }
        }
    }
    pages.sort();
    pages
}

#[test]
fn published_docs_name_only_the_current_manifest_schema_version() {
    let root = repo_root();
    let docs_root = root.join("docs");
    let pages = published_pages(&docs_root);

    // Every navigation page must be among the walked pages, or the walker or
    // fence parse broke — refuse rather than scan a partial tree.
    let walked_routes: BTreeSet<String> = pages
        .iter()
        .map(|page| {
            page.strip_prefix(&docs_root)
                .expect("page under docs root")
                .with_extension("")
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    let unwalked: Vec<String> = nav_routes(&docs_root)
        .into_iter()
        .filter(|route| !walked_routes.contains(route))
        .collect();
    assert!(
        unwalked.is_empty(),
        "docs.json navigation publishes pages the walk did not visit: \
         {unwalked:?} — the walker or the fence parse broke",
    );

    let prefix = schema_family_prefix();
    let mut offenders = Vec::new();
    for page in &pages {
        let text = std::fs::read_to_string(page)
            .unwrap_or_else(|error| panic!("read {}: {error}", page.display()));
        for (index, line) in text.lines().enumerate() {
            let mut rest = line;
            while let Some(position) = rest.find(&prefix) {
                let after = &rest[position + prefix.len()..];
                let version: String = after
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric())
                    .collect();
                if format!("{prefix}{version}") != MANIFEST_SCHEMA_VERSION_V3 {
                    offenders.push(format!(
                        "{}:{}: `{prefix}{version}` — {}",
                        page.strip_prefix(&root).expect("page under root").display(),
                        index + 1,
                        line.trim()
                    ));
                }
                rest = after;
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "published docs name a manifest schema version other than the current \
         `{MANIFEST_SCHEMA_VERSION_V3}`:\n{}",
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
