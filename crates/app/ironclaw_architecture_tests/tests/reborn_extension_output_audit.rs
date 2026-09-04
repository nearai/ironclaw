//! Reviewed encoded-output boundary ledger for extension and runtime producers.
//!
//! Indicator matches are candidates for review, not proof of a defect. Every
//! candidate must have an exact ledger entry naming its owner, exposure, and
//! executable evidence. New and stale entries both fail, so the ledger cannot
//! become an inert allowlist.

#[allow(dead_code)]
mod ratchet_support;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use ratchet_support::{
    crate_dir, production_rust_files, strip_line_anchored_cfg_test_items, workspace_root,
};

const LEDGER: &str = include_str!("fixtures/reborn_extension_output_boundaries.toml");

const INDICATORS: &[(&str, &str)] = &[
    ("content_base64", "content_base64"),
    ("body_base64", "body_base64"),
    ("bytes_base64", "bytes_base64"),
    ("gmail_base64url_decode", "decode_base64url"),
    ("binary_unsupported", "binary_unsupported"),
    ("mcp_structured_content", "structuredContent"),
    ("encrypted_marker", "\"encrypted\""),
    ("response_headers", "response_headers"),
    ("ciphertext", "ciphertext"),
    ("mime_transfer_encoding", "Content-Transfer-Encoding"),
    ("json_rpc", "jsonrpc"),
    ("json_rpc", "json-rpc"),
    ("inline_binary", "inline binary"),
    ("headers_field", "\"headers\""),
];

const ALLOWED_CLASSIFICATIONS: &[&str] = &[
    "producer_owned_decode",
    "semantic_unsupported_marker",
    "host_mediated_decode",
    "protocol_projection",
    "input_only",
    "opaque_custody",
    "compatibility_exception",
];

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
struct Candidate {
    path: String,
    indicator: String,
    occurrences: usize,
}

#[derive(Debug)]
struct ReviewedBoundary {
    candidate: Candidate,
    classification: String,
    exposure: String,
    owner: String,
    evidence_path: String,
    evidence_test: String,
    rationale: String,
}

fn discover_candidates(root: &Path, source_roots: &[PathBuf]) -> Result<Vec<Candidate>, String> {
    if source_roots.is_empty() {
        return Err("encoded-output scan received no source roots".to_string());
    }

    let mut candidates = BTreeSet::new();
    for source_root in source_roots {
        if !source_root.is_dir() {
            return Err(format!(
                "encoded-output source root does not exist: {}",
                source_root.display()
            ));
        }
        let source_files = production_rust_files(source_root);
        if source_files.is_empty() {
            return Err(format!(
                "encoded-output source root contains no production Rust files: {}",
                source_root.display()
            ));
        }
        for path in source_files {
            let raw = fs::read_to_string(&path)
                .map_err(|error| format!("read {}: {error}", path.display()))?;
            let source = strip_line_anchored_cfg_test_items(&raw);
            let relative = path
                .strip_prefix(root)
                .map_err(|error| {
                    format!("{} is outside {}: {error}", path.display(), root.display())
                })?
                .to_string_lossy()
                .replace('\\', "/");
            collect_candidate_matches(&source, &relative, &mut candidates);
        }
    }
    Ok(candidates.into_iter().collect())
}

fn collect_candidate_matches(
    source: &str,
    relative_path: &str,
    candidates: &mut BTreeSet<Candidate>,
) {
    let mut matches = BTreeMap::new();
    for (indicator, needle) in INDICATORS {
        let occurrences = source.matches(needle).count();
        if occurrences > 0 {
            *matches.entry(*indicator).or_insert(0) += occurrences;
        }
    }
    let provider_base64_decodes = count_provider_base64_decodes(source);
    if provider_base64_decodes > 0 {
        matches.insert("provider_base64_decode", provider_base64_decodes);
    }
    candidates.extend(
        matches
            .into_iter()
            .map(|(indicator, occurrences)| Candidate {
                path: relative_path.to_string(),
                indicator: indicator.to_string(),
                occurrences,
            }),
    );
}

fn count_provider_base64_decodes(source: &str) -> usize {
    ["BASE64_STANDARD", "general_purpose::STANDARD"]
        .into_iter()
        .map(|marker| {
            source
                .match_indices(marker)
                .filter(|(index, _)| {
                    source[index + marker.len()..]
                        .trim_start()
                        .starts_with(".decode")
                })
                .count()
        })
        .sum()
}

fn package_inventory_root(root: &Path) -> Result<PathBuf, String> {
    let support = crate_dir(root, "ironclaw_extension_support");
    let packages = support
        .parent()
        .ok_or_else(|| "extension support crate has no family directory".to_string())?
        .join("packages");
    if !packages.is_dir() {
        return Err(format!(
            "extension package inventory does not exist: {}",
            packages.display()
        ));
    }
    Ok(packages)
}

fn package_source_roots(root: &Path) -> Result<Vec<PathBuf>, String> {
    let packages = package_inventory_root(root)?;
    let entries = fs::read_dir(&packages)
        .map_err(|error| format!("read package inventory {}: {error}", packages.display()))?;
    let mut roots = Vec::new();
    for entry in entries {
        let package = entry
            .map_err(|error| format!("read package inventory entry: {error}"))?
            .path();
        if !package.is_dir() {
            continue;
        }
        for relative in ["src", "wasm-src/src"] {
            let source = package.join(relative);
            if source.is_dir() {
                roots.push(source);
            }
        }
    }
    roots.sort();
    if roots.is_empty() {
        return Err(format!(
            "package inventory {} exposed no production source roots",
            packages.display()
        ));
    }
    Ok(roots)
}

fn collect_output_schemas(directory: &Path, output: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(directory).map_err(|error| {
        format!(
            "read output-schema directory {}: {error}",
            directory.display()
        )
    })?;
    for entry in entries {
        let path = entry
            .map_err(|error| format!("read output-schema entry: {error}"))?
            .path();
        if path.is_dir() {
            collect_output_schemas(&path, output)?;
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.ends_with(".json")
                    && (name.contains(".output.v") || name.starts_with("raw_output.v"))
            })
        {
            output.push(path);
        }
    }
    Ok(())
}

fn discover_output_schema_candidates(
    root: &Path,
    package_root: &Path,
) -> Result<Vec<Candidate>, String> {
    if !package_root.is_dir() {
        return Err(format!(
            "output-schema package root does not exist: {}",
            package_root.display()
        ));
    }
    let mut schemas = Vec::new();
    collect_output_schemas(package_root, &mut schemas)?;
    if schemas.is_empty() {
        return Err(format!(
            "output-schema scan found no schemas under {}",
            package_root.display()
        ));
    }
    let mut candidates = BTreeSet::new();
    for path in schemas {
        let source = fs::read_to_string(&path)
            .map_err(|error| format!("read output schema {}: {error}", path.display()))?;
        let relative = path
            .strip_prefix(root)
            .map_err(|error| format!("{} is outside {}: {error}", path.display(), root.display()))?
            .to_string_lossy()
            .replace('\\', "/");
        collect_candidate_matches(&source, &relative, &mut candidates);
    }
    Ok(candidates.into_iter().collect())
}

fn audited_source_roots(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut roots = vec![
        crate_dir(root, "ironclaw_extension_support").join("src"),
        crate_dir(root, "ironclaw_mcp").join("src"),
        crate_dir(root, "ironclaw_wasm").join("src"),
        crate_dir(root, "ironclaw_extension_host").join("src"),
        crate_dir(root, "ironclaw_host_runtime").join("src"),
        crate_dir(root, "ironclaw_threads").join("src"),
        crate_dir(root, "ironclaw_loop_host").join("src"),
        crate_dir(root, "ironclaw_composition").join("src"),
    ];
    roots.extend(package_source_roots(root)?);
    Ok(roots)
}

fn required_string(table: &toml::Table, field: &str, index: usize) -> String {
    table
        .get(field)
        .and_then(toml::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| panic!("boundary[{index}] requires non-empty `{field}`"))
        .to_string()
}

fn parse_ledger(source: &str) -> Vec<ReviewedBoundary> {
    let document: toml::Value =
        toml::from_str(source).expect("encoded-output ledger must be valid TOML");
    let rows = document
        .get("boundary")
        .and_then(toml::Value::as_array)
        .expect("encoded-output ledger requires at least one [[boundary]] row");
    assert!(!rows.is_empty(), "encoded-output ledger must not be empty");
    rows.iter()
        .enumerate()
        .map(|(index, row)| {
            let table = row
                .as_table()
                .unwrap_or_else(|| panic!("boundary[{index}] must be a TOML table"));
            ReviewedBoundary {
                candidate: Candidate {
                    path: required_string(table, "path", index),
                    indicator: required_string(table, "indicator", index),
                    occurrences: table
                        .get("occurrences")
                        .and_then(toml::Value::as_integer)
                        .map_or(1, |value| {
                            usize::try_from(value).unwrap_or_else(|error| {
                                panic!("boundary[{index}] has invalid `occurrences`: {error}")
                            })
                        }),
                },
                classification: required_string(table, "classification", index),
                exposure: required_string(table, "exposure", index),
                owner: required_string(table, "owner", index),
                evidence_path: required_string(table, "evidence_path", index),
                evidence_test: required_string(table, "evidence_test", index),
                rationale: required_string(table, "rationale", index),
            }
        })
        .collect()
}

fn declares_executable_test(source: &str, expected_name: &str) -> Result<bool, String> {
    fn items_contain_test(items: &[syn::Item], expected_name: &str) -> bool {
        items.iter().any(|item| match item {
            syn::Item::Fn(function) => {
                function.sig.ident == expected_name
                    && function.attrs.iter().any(|attribute| {
                        attribute
                            .path()
                            .segments
                            .last()
                            .is_some_and(|segment| segment.ident == "test")
                    })
            }
            syn::Item::Mod(module) => module
                .content
                .as_ref()
                .is_some_and(|(_, items)| items_contain_test(items, expected_name)),
            _ => false,
        })
    }

    let file = syn::parse_file(source).map_err(|error| format!("parse Rust evidence: {error}"))?;
    Ok(items_contain_test(&file.items, expected_name))
}

#[test]
fn reborn_candidate_scanner_finds_an_encoded_output_indicator() {
    let directory = tempfile::tempdir().expect("temporary fixture directory");
    fs::write(
        directory.path().join("producer.rs"),
        "let output = json!({ \"content_base64\": encoded });",
    )
    .expect("write scanner fixture");

    assert_eq!(
        discover_candidates(directory.path(), &[directory.path().to_path_buf()])
            .expect("fixture scan succeeds"),
        vec![Candidate {
            path: "producer.rs".to_string(),
            indicator: "content_base64".to_string(),
            occurrences: 1,
        }]
    );

    let package = directory.path().join("fixture-package");
    let schemas = package.join("schemas");
    fs::create_dir_all(&schemas).expect("create schema fixture directory");
    fs::write(
        package.join("manifest.toml"),
        r#"[[capabilities]]
output_schema_ref = "schemas/raw_output.v1.json"
"#,
    )
    .expect("write package manifest fixture");
    fs::write(
        schemas.join("raw_output.v1.json"),
        r#"{"properties":{"encoding":{"enum":["binary_unsupported"]}}}"#,
    )
    .expect("write output schema fixture");
    assert_eq!(
        discover_output_schema_candidates(directory.path(), directory.path())
            .expect("fixture output-schema scan succeeds"),
        vec![Candidate {
            path: "fixture-package/schemas/raw_output.v1.json".to_string(),
            indicator: "binary_unsupported".to_string(),
            occurrences: 1,
        }]
    );
}

#[test]
fn reborn_candidate_scanner_counts_each_boundary_and_provider_decode_operation() {
    let directory = tempfile::tempdir().expect("temporary fixture directory");
    fs::write(
        directory.path().join("producer.rs"),
        r#"
fn first(encoded: &str) {
    BASE64_STANDARD
        .decode(encoded.as_bytes());
}
fn second(encoded: &str) { BASE64_STANDARD.decode(encoded.as_bytes()); }
"#,
    )
    .expect("write scanner fixture");

    assert_eq!(
        discover_candidates(directory.path(), &[directory.path().to_path_buf()])
            .expect("fixture scan succeeds"),
        vec![Candidate {
            path: "producer.rs".to_string(),
            indicator: "provider_base64_decode".to_string(),
            occurrences: 2,
        }]
    );
}

#[test]
fn reborn_candidate_scanner_fails_closed_for_a_missing_root() {
    let directory = tempfile::tempdir().expect("temporary fixture directory");
    let error = discover_candidates(directory.path(), &[directory.path().join("missing")])
        .expect_err("a missing scan root must fail closed");
    assert!(
        error.contains("does not exist"),
        "unexpected error: {error}"
    );
}

#[test]
fn reborn_candidate_scanner_fails_closed_for_an_empty_root() {
    let directory = tempfile::tempdir().expect("temporary fixture directory");
    let error = discover_candidates(directory.path(), &[directory.path().to_path_buf()])
        .expect_err("an empty scan root must fail closed");
    assert!(
        error.contains("contains no production Rust files"),
        "unexpected error: {error}"
    );
}

#[test]
fn reborn_evidence_requires_an_executable_test_item() {
    let inert = r##"
fn helper() {}
// #[test] fn commented_out() {}
const TEXT: &str = "#[test] fn string_only() {}";
"##;
    assert!(!declares_executable_test(inert, "helper").expect("parse inert fixture"));
    assert!(!declares_executable_test(inert, "commented_out").expect("parse inert fixture"));
    assert!(!declares_executable_test(inert, "string_only").expect("parse inert fixture"));

    let executable = r#"
#[test]
fn sync_test() {}

#[tokio::test]
async fn async_test() {}
"#;
    assert!(declares_executable_test(executable, "sync_test").expect("parse test fixture"));
    assert!(declares_executable_test(executable, "async_test").expect("parse test fixture"));
}

#[test]
fn reborn_every_encoded_output_candidate_has_a_live_reviewed_boundary() {
    let root = workspace_root();
    let mut candidates = discover_candidates(
        &root,
        &audited_source_roots(&root).expect("resolve encoded-output source roots"),
    )
    .expect("scan encoded-output candidates");
    candidates.extend(
        discover_output_schema_candidates(
            &root,
            &package_inventory_root(&root).expect("resolve extension package inventory"),
        )
        .expect("scan encoded-output schemas"),
    );
    candidates.sort();
    candidates.dedup();
    assert!(
        !candidates.is_empty(),
        "encoded-output scan found nothing; the gate is not inspecting production"
    );

    let reviewed = parse_ledger(LEDGER);
    let reviewed_candidates: BTreeSet<_> = reviewed
        .iter()
        .map(|boundary| boundary.candidate.clone())
        .collect();
    assert_eq!(
        reviewed_candidates.len(),
        reviewed.len(),
        "encoded-output ledger contains duplicate (path, indicator) rows"
    );
    let discovered: BTreeSet<_> = candidates.into_iter().collect();
    assert_eq!(
        discovered, reviewed_candidates,
        "encoded-output candidate inventory drifted. Classify each new candidate after review; remove stale rows when a boundary disappears"
    );

    for boundary in reviewed {
        assert!(
            ALLOWED_CLASSIFICATIONS.contains(&boundary.classification.as_str()),
            "{}:{} has unsupported classification {:?}; `model_facing_defect` must remain zero",
            boundary.candidate.path,
            boundary.candidate.indicator,
            boundary.classification
        );
        assert!(
            !boundary.exposure.trim().is_empty()
                && !boundary.owner.trim().is_empty()
                && !boundary.rationale.trim().is_empty(),
            "{}:{} lacks an accountable review record",
            boundary.candidate.path,
            boundary.candidate.indicator
        );
        let evidence = root.join(&boundary.evidence_path);
        assert_eq!(
            evidence
                .extension()
                .and_then(|extension| extension.to_str()),
            Some("rs"),
            "evidence for {}:{} must be a Rust test source",
            boundary.candidate.path,
            boundary.candidate.indicator
        );
        let evidence_source = fs::read_to_string(&evidence)
            .unwrap_or_else(|error| panic!("read evidence {}: {error}", evidence.display()));
        assert!(
            declares_executable_test(&evidence_source, &boundary.evidence_test)
                .unwrap_or_else(|error| panic!("parse evidence {}: {error}", evidence.display())),
            "stale evidence for {}:{}: {} no longer declares executable test {:?}",
            boundary.candidate.path,
            boundary.candidate.indicator,
            boundary.evidence_path,
            boundary.evidence_test
        );
    }
}

#[test]
fn reborn_generic_durable_writer_and_reader_remain_producer_blind() {
    let root = workspace_root();
    let generic_files = [
        crate_dir(&root, "ironclaw_loop_host").join("src/capability_port.rs"),
        crate_dir(&root, "ironclaw_composition").join("src/runtime/capability_host.rs"),
        crate_dir(&root, "ironclaw_loop_host").join("src/result_read.rs"),
        crate_dir(&root, "ironclaw_threads").join("src/tool_result_records.rs"),
    ];
    let forbidden = [
        "content_base64",
        "body_base64",
        "bytes_base64",
        "binary_unsupported",
        "decode_base64url",
        "OutputNormalizer",
        "NormalizerRegistry",
        "normalize_capability_output",
        "parse_capability_output",
    ];

    for path in generic_files {
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read generic result path {}: {error}", path.display()));
        for needle in forbidden {
            assert!(
                !source.contains(needle),
                "generic durable result path {} contains producer-specific output logic `{needle}`",
                path.display()
            );
        }
    }
}
