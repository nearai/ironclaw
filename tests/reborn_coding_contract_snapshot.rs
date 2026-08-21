//! Pinned core-tool contract snapshot (issue #7392, first delivery slice).
//!
//! Validates the checked-in snapshot at `tests/fixtures/pinned_coding_contract/`
//! without any network access:
//!
//! - the snapshot is pinned to the reviewed upstream commit with MIT provenance
//!   (full license text vendored and integrity-tested),
//! - the inventory is exactly the eight pinned core coding tools
//!   (`read`, `write`, `edit`, `glob`, `grep`, `bash`, `ast_grep`, `ast_edit`),
//! - every vendored and derived file plus `manifest.json` matches its recorded
//!   SHA-256 checksum and byte count (unsnapshotted upstream records are
//!   capture-time pins, not byte-verified offline),
//! - no orphaned or missing fixture files,
//! - the rendered schemas, selector cases, error/output fixtures, and exact
//!   case-ID inventories pin the documented model-visible contract,
//! - the reusable differential comparison factory reports agreement and
//!   structured mismatches for both implementations.
//!
//! The support module (`support/pinned_coding_contract/`) exposes the loaders
//! used here as the reuse seam for the later pinned-vs-IronClaw differential
//! execution tests.

mod support;

use ironclaw_host_api::artifact::ArtifactRef;
use serde_json::json;
use support::pinned_coding_contract::{
    EXPECTED_TOOL_NAMES, PINNED_COMMIT, RunOutcome, compare_cases, error_entries, fixture_root,
    license_text, load_manifest, load_provenance, orphan_snapshot_files, read_snapshot_file,
    read_snapshot_text, rendered_tool_prompt, selector_cases, sha256_hex, tool_prompt, tool_schema,
    verify_snapshotted_checksums,
};

#[test]
fn snapshot_is_pinned_to_the_reviewed_upstream_commit() {
    let provenance = load_provenance();
    let manifest = load_manifest();

    assert_eq!(provenance.schema_version, 1, "provenance schema version");
    assert_eq!(manifest.schema_version, 1, "manifest schema version");
    assert_eq!(
        provenance.upstream.repository,
        "https://github.com/can1357/oh-my-pi"
    );
    assert_eq!(provenance.upstream.commit, PINNED_COMMIT);
    assert_eq!(manifest.pinned_commit, PINNED_COMMIT);
    assert_eq!(provenance.upstream.license, "MIT");
    assert_eq!(
        provenance.upstream.license_file.as_deref(),
        Some("licenses/LICENSE"),
        "license_file must point at the vendored full MIT license asset"
    );
    assert_eq!(
        provenance.upstream.license_upstream_path.as_deref(),
        Some("LICENSE"),
        "the upstream license path is recorded for provenance"
    );
    assert!(
        !read_snapshot_file(
            provenance
                .upstream
                .license_file
                .as_deref()
                .expect("vendored license")
        )
        .is_empty(),
        "the vendored license asset must exist"
    );
    assert!(
        !provenance
            .upstream
            .license_notice
            .as_deref()
            .unwrap_or("")
            .is_empty(),
        "the provenance record carries the MIT copyright notice for derived work"
    );

    let snapshotted = provenance
        .files
        .iter()
        .filter(|record| record.snapshotted)
        .count();
    assert!(
        snapshotted >= 17,
        "expected at least the verbatim prompt/grammar/source/license pins, got {snapshotted}"
    );
    assert!(
        !provenance.derived.is_empty(),
        "rendered schemas and golden cases must be checksummed in provenance.json"
    );
    assert!(
        provenance.manifest.is_some(),
        "manifest.json itself must be checksummed in provenance.json"
    );
    for record in &provenance.files {
        assert_eq!(record.sha256.len(), 64, "sha256 hex for {}", record.path);
        if record.snapshotted {
            assert!(
                record.snapshot_path.is_some(),
                "snapshotted record {} lacks a snapshot path",
                record.path
            );
        }
    }
}

#[test]
fn snapshot_inventory_is_exactly_the_eight_core_tools() {
    let manifest = load_manifest();

    assert_eq!(manifest.tool_names, EXPECTED_TOOL_NAMES.to_vec());
    assert_eq!(manifest.tools.len(), EXPECTED_TOOL_NAMES.len());
    for name in EXPECTED_TOOL_NAMES {
        let entry = manifest
            .tools
            .get(name)
            .unwrap_or_else(|| panic!("tool {name} missing from manifest"));
        assert_eq!(entry.name, name, "tool key must match its declared name");
        let schema = read_snapshot_text(&entry.schema);
        assert!(
            schema.starts_with('{'),
            "schema for {name} must be a rendered JSON object"
        );
        let prompt = entry
            .prompt
            .as_ref()
            .unwrap_or_else(|| panic!("tool {name} must carry a prompt asset"));
        assert!(
            !read_snapshot_text(prompt).is_empty(),
            "prompt for {name} must not be empty"
        );
        assert!(
            entry.errors_fixture.is_some(),
            "tool {name} must carry a representative error fixture"
        );
    }
}

#[test]
fn snapshotted_files_match_provenance_checksums() {
    let provenance = load_provenance();
    let mismatches = verify_snapshotted_checksums(&provenance);
    assert!(
        mismatches.is_empty(),
        "snapshotted files drifted from their pinned upstream checksums:\n{}",
        mismatches
            .iter()
            .map(|mismatch| format!("  {mismatch}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn no_orphan_files_under_the_snapshot_root() {
    let provenance = load_provenance();
    let orphans = orphan_snapshot_files(&provenance);
    assert!(
        orphans.is_empty(),
        "files under the snapshot root that provenance.json does not reference:\n{}",
        orphans.join("\n")
    );
}

#[test]
fn schemas_pin_the_default_model_visible_contract() {
    let read = tool_schema("read");
    assert_eq!(read["type"], json!("object"));
    assert_eq!(read["additionalProperties"], json!(false));
    assert_eq!(read["required"], json!(["path"]));
    assert_eq!(read["properties"]["path"]["type"], json!("string"));
    assert!(
        read["properties"]["path"]["description"]
            .as_str()
            .is_some_and(|description| description.starts_with("Local path, internal URI"))
    );

    let write = tool_schema("write");
    assert_eq!(write["required"], json!(["path", "content"]));
    assert_eq!(
        write["properties"]["content"]["description"],
        json!("file content")
    );

    let edit = tool_schema("edit");
    assert_eq!(edit["required"], json!(["input"]));
    assert_eq!(edit["properties"]["input"]["type"], json!("string"));
    assert_eq!(
        edit["properties"]
            .as_object()
            .map(|properties| properties.len()),
        Some(1),
        "the hashline edit payload is a single required `input` string"
    );

    let glob = tool_schema("glob");
    assert!(
        glob["required"]
            .as_array()
            .is_none_or(|required| required.is_empty())
    );
    for property in ["path", "hidden", "gitignore", "limit"] {
        assert!(
            glob["properties"][property].is_object(),
            "glob property {property} missing"
        );
    }
    assert_eq!(glob["properties"]["limit"]["type"], json!("number"));

    let grep = tool_schema("grep");
    assert_eq!(grep["required"], json!(["pattern"]));
    assert_eq!(
        grep["properties"]["pattern"]["description"],
        json!("regex pattern")
    );
    assert_eq!(
        grep["properties"]["skip"]["type"],
        json!(["number", "null"])
    );

    let bash = tool_schema("bash");
    assert_eq!(bash["required"], json!(["command"]));
    assert_eq!(
        bash["properties"]["timeout"]["description"],
        json!(
            "timeout in seconds; 0 disables the command deadline; nonzero values are clamped to 1-3600"
        )
    );
    assert_eq!(
        bash["properties"]["pty"]["description"],
        json!("run in pty mode")
    );

    let ast_grep = tool_schema("ast_grep");
    assert_eq!(ast_grep["required"], json!(["pat"]));
    assert_eq!(
        ast_grep["properties"]["pat"]["description"],
        json!("ast pattern")
    );

    let ast_edit = tool_schema("ast_edit");
    assert_eq!(ast_edit["required"], json!(["ops", "paths"]));
    assert_eq!(ast_edit["properties"]["ops"]["minItems"], json!(1));
    assert_eq!(
        ast_edit["properties"]["ops"]["items"]["required"],
        json!(["pat", "out"])
    );
    assert_eq!(ast_edit["properties"]["paths"]["minItems"], json!(1));
    assert_eq!(
        ast_edit["properties"]["paths"]["items"]["type"],
        json!("string")
    );
}

#[test]
fn golden_error_fixtures_reference_provenanced_upstream_sources() {
    let provenance = load_provenance();
    let known_paths: Vec<&str> = provenance
        .files
        .iter()
        .map(|record| record.path.as_str())
        .collect();

    for tool in EXPECTED_TOOL_NAMES {
        let entries = error_entries(tool);
        assert!(
            !entries.is_empty(),
            "tool {tool} must have representative error entries"
        );
        for entry in &entries {
            assert!(
                known_paths.contains(&entry.source_path.as_str()),
                "error entry {}.{} references {} which is not in provenance.json",
                tool,
                entry.case,
                entry.source_path
            );
            assert!(
                !entry.template.is_empty(),
                "error entry {}.{} has an empty template",
                tool,
                entry.case
            );
        }
    }

    // Output-format examples exist for the three tools with pinned output formats,
    // and every cited source path must be provened (in provenance.json).
    let manifest = load_manifest();
    for tool in ["read", "grep", "edit"] {
        let fixture = manifest.tools[tool]
            .output_fixture
            .as_deref()
            .unwrap_or_else(|| panic!("tool {tool} must carry an output fixture"));
        let value: serde_json::Value =
            serde_json::from_str(&read_snapshot_text(fixture)).expect("output fixture must parse");
        let formats = value["formats"]
            .as_array()
            .unwrap_or_else(|| panic!("output fixture {fixture} must carry format examples"));
        assert!(
            !formats.is_empty(),
            "output fixture {fixture} must carry format examples"
        );
        for format in formats {
            let source_path = format["source_path"]
                .as_str()
                .unwrap_or_else(|| panic!("output case {} lacks source_path", format["case"]));
            assert!(
                known_paths.contains(&source_path),
                "output case {} of {tool} references {} which is not in provenance.json",
                format["case"],
                source_path
            );
        }
    }
}

#[test]
fn selector_golden_cases_cover_the_documented_grammar() {
    let cases = selector_cases();
    assert!(
        cases.len() >= 20,
        "expected a broad selector case battery, got {}",
        cases.len()
    );
    for case in &cases {
        assert!(
            case.error.is_some() || case.selector.is_some(),
            "selector case {:?} must produce a parse result or an error",
            case.sel
        );
    }

    let by_sel = |sel: &str| {
        cases
            .iter()
            .find(|case| case.sel == sel)
            .unwrap_or_else(|| panic!("missing selector case {sel:?}"))
    };

    let inclusive = by_sel("50-200");
    assert_eq!(
        inclusive.selector,
        Some(json!({"kind": "lines", "ranges": [{"startLine": 50, "endLine": 200}]}))
    );
    assert_eq!(
        inclusive.offset_limit,
        Some(json!({"offset": 50, "limit": 151}))
    );

    let counted = by_sel("50+150");
    assert_eq!(
        counted.selector,
        Some(json!({"kind": "lines", "ranges": [{"startLine": 50, "endLine": 199}]}))
    );

    let merged = by_sel("1-10,5-20");
    assert_eq!(
        merged.selector,
        Some(json!({"kind": "lines", "ranges": [{"startLine": 1, "endLine": 20}]})),
        "overlapping ranges must merge in one forward pass"
    );

    let raw = by_sel("raw");
    assert_eq!(raw.selector, Some(json!({"kind": "raw"})));

    let compound = by_sel("raw:50-100");
    assert_eq!(
        compound.selector,
        Some(json!({"kind": "lines", "ranges": [{"startLine": 50, "endLine": 100}], "raw": true}))
    );

    let zero = by_sel("0");
    assert_eq!(
        zero.error.as_deref(),
        Some("Line selector 0 is invalid; lines are 1-indexed. Use :1.")
    );

    let invalid = by_sel("raw:raw");
    assert!(
        invalid
            .error
            .as_deref()
            .is_some_and(|message| message.starts_with("Invalid selector ':raw:raw'.")),
        "malformed compounds must be rejected, not silently widened"
    );
}

#[test]
fn snapshot_accessors_expose_reusable_differential_cases() {
    // The accessor API is the seam the later pinned-vs-IronClaw differential
    // execution tests drive: load once, run both implementations per case.
    let read_schema = tool_schema("read");
    let read_schema_again = tool_schema("read");
    assert_eq!(
        read_schema, read_schema_again,
        "schema loading must be deterministic"
    );

    let edit_prompt = tool_prompt("edit");
    assert!(
        edit_prompt.contains("PUT"),
        "hashline prompt must document the PUT op"
    );

    let edit_errors = error_entries("edit");
    assert!(
        edit_errors
            .iter()
            .any(|entry| entry.case == "stale_anchor_hash_recognized"),
        "the stale-anchor (file changed between read and edit) error shape must be pinned"
    );

    let cases = selector_cases();
    let cases_again = selector_cases();
    let serialize = |cases: &[support::pinned_coding_contract::SelectorCase]| {
        serde_json::to_string(cases).expect("selector cases serialize")
    };
    assert_eq!(
        serialize(&cases),
        serialize(&cases_again),
        "selector cases must be deterministic"
    );

    let provenance = load_provenance();
    assert_eq!(
        provenance
            .files
            .iter()
            .filter(|record| record.snapshotted)
            .count(),
        provenance
            .files
            .iter()
            .filter(|record| record.snapshot_path.is_some())
            .count(),
        "every vendored file must be checksummed and every checksummed file vendored"
    );

    // Explicitly exercise the offline property: everything above read from
    // the fixture tree only, and the root resolves inside the repo.
    assert!(fixture_root().starts_with(env!("CARGO_MANIFEST_DIR")));
    assert!(read_snapshot_file("manifest.json").len() > 100);
}

#[test]
fn differential_factory_reports_agreement_and_structured_mismatches() {
    // The factory is engine-agnostic: the checked-in selector cases stand in
    // for the later real pinned-vs-IronClaw engine pairs.
    let cases = selector_cases();
    let parse_result =
        |case: &support::pinned_coding_contract::SelectorCase| -> Result<serde_json::Value, String> {
            match (&case.selector, &case.error) {
                (Some(selector), _) => Ok(selector.clone()),
                (None, Some(error)) => Err(error.clone()),
                (None, None) => Err("no parse result recorded".to_string()),
            }
        };

    // Identical implementations agree on every case.
    let mismatches = compare_cases(
        &cases,
        |index, case| format!("{index}:{}", case.sel),
        parse_result,
        parse_result,
    );
    assert!(
        mismatches.is_empty(),
        "identical implementations must agree, got {:?}",
        mismatches
    );

    // A divergent candidate is reported structurally, one entry per case.
    let divergent =
        |_case: &support::pinned_coding_contract::SelectorCase| -> Result<serde_json::Value, String> {
            Ok(json!({"kind": "divergent"}))
        };
    let mismatches = compare_cases(
        &cases,
        |index, case| format!("{index}:{}", case.sel),
        parse_result,
        divergent,
    );
    assert_eq!(mismatches.len(), cases.len(), "every case must mismatch");
    for mismatch in &mismatches {
        assert!(!mismatch.case.is_empty(), "case name must be carried");
        assert_ne!(
            mismatch.baseline, mismatch.candidate,
            "case {} must disagree",
            mismatch.case
        );
    }
    let raw_case = mismatches
        .iter()
        .find(|mismatch| mismatch.case.ends_with(":raw"))
        .expect("the 'raw' selector case must be present");
    assert_eq!(raw_case.baseline, RunOutcome::Ok(json!({"kind": "raw"})));
    assert_eq!(
        raw_case.candidate,
        RunOutcome::Ok(json!({"kind": "divergent"}))
    );

    // A failing candidate produces an error outcome, and the structured entry
    // distinguishes baseline Ok from candidate Err.
    let failing =
        |_case: &support::pinned_coding_contract::SelectorCase| -> Result<serde_json::Value, String> {
            Err("candidate crashed".to_string())
        };
    let mismatches = compare_cases(
        &cases,
        |index, case| format!("{index}:{}", case.sel),
        parse_result,
        failing,
    );
    assert_eq!(mismatches.len(), cases.len());
    assert!(
        mismatches
            .iter()
            .all(|mismatch| matches!(mismatch.candidate, RunOutcome::Err(_))),
        "every mismatch must carry the candidate error"
    );

    // Reusable across fixture kinds: error entries work as named cases too.
    let edit_errors = error_entries("edit");
    let transcribe =
        |entry: &support::pinned_coding_contract::ErrorEntry| -> Result<serde_json::Value, String> {
            Ok(json!(entry.template.clone()))
        };
    let agree = compare_cases(
        &edit_errors,
        |_index, entry| entry.case.clone(),
        transcribe,
        transcribe,
    );
    assert!(agree.is_empty(), "same transcription must agree");
    let divergent_errors = compare_cases(
        &edit_errors,
        |_index, entry| entry.case.clone(),
        transcribe,
        |entry| Err(format!("{} not implemented", entry.case)),
    );
    assert_eq!(divergent_errors.len(), edit_errors.len());
    assert!(
        divergent_errors
            .iter()
            .any(|mismatch| mismatch.case == "stale_anchor_hash_recognized"
                && matches!(mismatch.candidate, RunOutcome::Err(_))),
        "named error-case mismatches must be reported"
    );
}

#[test]
fn vendored_license_is_the_full_pinned_mit_text() {
    let provenance = load_provenance();

    let license_record = provenance
        .files
        .iter()
        .find(|record| record.role == "license")
        .expect("provenance must carry the vendored license record");
    assert_eq!(license_record.path, "LICENSE", "upstream license path");
    let snapshot_path = license_record
        .snapshot_path
        .as_deref()
        .expect("the license record must be snapshotted");
    assert_eq!(snapshot_path, "licenses/LICENSE");
    assert_eq!(
        provenance.upstream.license_file.as_deref(),
        Some(snapshot_path),
        "upstream.license_file must point at the checked-in asset"
    );

    let text = read_snapshot_text(snapshot_path);
    assert_eq!(
        text,
        license_text(),
        "the license_text() accessor must expose the vendored asset"
    );
    assert_eq!(
        text.len(),
        license_record.bytes,
        "vendored license byte count"
    );
    assert_eq!(
        sha256_hex(text.as_bytes()),
        license_record.sha256,
        "vendored license checksum"
    );
    assert!(text.starts_with("MIT License"), "full MIT license header");
    assert!(
        text.contains(
            "Permission is hereby granted, free of charge, to any person obtaining a copy"
        ),
        "full grant clause must be present"
    );
    assert!(
        text.contains(
            "THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR\nIMPLIED"
        ),
        "full warranty disclaimer must be present"
    );
    assert!(text.contains("Copyright (c) 2025 Mario Zechner"));
    assert!(text.contains("Copyright (c) 2025-2026 Can B\u{00f6}l\u{00fc}k"));

    // The vendored license is covered by the offline checksum sweep too.
    let mismatches = verify_snapshotted_checksums(&provenance);
    assert!(
        mismatches.is_empty(),
        "vendored and derived assets drifted from their pinned checksums:\n{}",
        mismatches
            .iter()
            .map(|mismatch| format!("  {mismatch}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn manifest_itself_is_checksummed_and_only_provenance_is_self_exempt() {
    let provenance = load_provenance();
    let record = provenance
        .manifest
        .as_ref()
        .expect("manifest.json must have a checksum record in provenance.json");
    assert_eq!(record.path, "manifest.json");
    let bytes = read_snapshot_file("manifest.json");
    assert_eq!(
        bytes.len(),
        record.bytes,
        "manifest byte count must match its record"
    );
    assert_eq!(
        sha256_hex(&bytes),
        record.sha256,
        "manifest checksum must match its record"
    );

    // The checksum sweep covers it offline; nothing is left unrecorded except
    // provenance.json itself.
    assert!(
        verify_snapshotted_checksums(&provenance).is_empty(),
        "checksum sweep must pass including manifest.json"
    );
    let orphans = orphan_snapshot_files(&provenance);
    assert!(
        orphans.is_empty(),
        "files under the snapshot root that provenance.json does not reference:\n{}",
        orphans.join("\n")
    );
}

#[test]
fn every_manifest_referenced_asset_is_checksummed() {
    let provenance = load_provenance();
    let manifest = load_manifest();

    let mut checksummed: Vec<&str> = Vec::new();
    for record in &provenance.files {
        if let Some(snapshot_path) = &record.snapshot_path {
            checksummed.push(snapshot_path);
        }
    }
    for record in &provenance.derived {
        checksummed.push(&record.path);
    }
    if let Some(record) = &provenance.manifest {
        checksummed.push(&record.path);
    }

    for (tool, entry) in &manifest.tools {
        let mut referenced = vec![entry.schema.as_str()];
        for path in [
            entry.prompt.as_deref(),
            entry.grammar.as_deref(),
            entry.selectors_source.as_deref(),
            entry.errors_fixture.as_deref(),
            entry.output_fixture.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            referenced.push(path);
        }
        if let Some(rendered) = &entry.rendered_prompt {
            referenced.push(&rendered.path);
        }
        for path in referenced {
            assert!(
                checksummed.contains(&path),
                "manifest asset {path} of tool {tool} has no offline checksum record"
            );
        }
    }
}

#[test]
fn fixture_case_inventories_are_exact() {
    let manifest = load_manifest();

    // Selector cases: the ordered sel list must equal the recorded inventory.
    let actual_selector_ids: Vec<String> = selector_cases()
        .iter()
        .map(|case| case.sel.clone())
        .collect();
    assert_eq!(
        actual_selector_ids, manifest.selector_case_ids,
        "golden/selectors.json must match the recorded selector case inventory exactly"
    );

    // Per-tool error and output case IDs must match their fixtures exactly.
    for tool in EXPECTED_TOOL_NAMES {
        let entry = &manifest.tools[tool];
        let actual_error_ids: Vec<String> = error_entries(tool)
            .iter()
            .map(|case| case.case.clone())
            .collect();
        assert_eq!(
            actual_error_ids, entry.errors_case_ids,
            "error fixture for {tool} must match its recorded case inventory exactly"
        );

        let expected_output_ids = entry.output_case_ids.clone().unwrap_or_default();
        match &entry.output_fixture {
            Some(fixture) => {
                let value: serde_json::Value = serde_json::from_str(&read_snapshot_text(fixture))
                    .expect("output fixture must parse");
                let actual_output_ids: Vec<String> = value["formats"]
                    .as_array()
                    .expect("output fixture has formats")
                    .iter()
                    .map(|format| format["case"].as_str().expect("output case id").to_string())
                    .collect();
                assert_eq!(
                    actual_output_ids, expected_output_ids,
                    "output fixture {fixture} for {tool} must match its recorded case inventory exactly"
                );
            }
            None => assert!(
                expected_output_ids.is_empty(),
                "tool {tool} has no output fixture so its output case inventory must be empty"
            ),
        }
    }
}

#[test]
fn rendered_read_prompt_pins_the_issue_target_description() {
    let manifest = load_manifest();
    let rendered = manifest.tools["read"]
        .rendered_prompt
        .as_ref()
        .expect("the read tool must record a rendered prompt");
    assert_eq!(rendered.path, "prompts/read.rendered.md");

    // The recorded render context is the issue #7392 target context.
    assert_eq!(rendered.context["DEFAULT_LIMIT"], json!("3000"));
    assert_eq!(rendered.context["DEFAULT_MAX_LINES"], json!("3000"));
    assert_eq!(rendered.context["IS_HL_MODE"], json!(true));
    assert_eq!(rendered.context["IS_LINE_NUMBER_MODE"], json!(false));
    assert_eq!(rendered.context["INSPECT_IMAGE_ENABLED"], json!(false));

    // Exact checked-in bytes and checksum, covered by the derived record.
    let text = read_snapshot_text(&rendered.path);
    let provenance = load_provenance();
    let record = provenance
        .derived
        .iter()
        .find(|record| record.path == rendered.path)
        .expect("the rendered prompt must have a derived checksum record");
    assert_eq!(record.bytes, text.len());
    assert_eq!(record.sha256, sha256_hex(text.as_bytes()));

    // The rendered text is the model-visible description for that context:
    // hashline display on, inspect-image off, no leftover template markers.
    assert!(
        !text.contains("{{"),
        "rendered prompt must not retain template markers"
    );
    assert!(text.contains("File + selector → `[foo.ts#1A2B]` snapshot header + numbered lines"));
    assert!(
        text.contains("Images → decoded inline"),
        "inspect-image off renders 'decoded inline'"
    );
    assert!(
        !text.contains("call `inspect_image`"),
        "inspect-image off must not mention inspect_image"
    );
    assert!(
        !text.contains("{{#if"),
        "no conditional markers may survive rendering"
    );

    // The verbatim template is retained separately and stays renderable by the
    // same pinned context (template markers present, template checksummed).
    let template = read_snapshot_text("prompts/read.md");
    assert!(
        template.contains("{{#if IS_HL_MODE}}"),
        "verbatim template retained"
    );
    let template_record = provenance
        .files
        .iter()
        .find(|record| record.snapshot_path.as_deref() == Some("prompts/read.md"))
        .expect("the verbatim template must stay checksummed in provenance");
    assert_eq!(template_record.sha256, sha256_hex(template.as_bytes()));

    // The accessor seam loads the same bytes the manifest points at.
    assert_eq!(
        rendered_tool_prompt("read").as_deref(),
        Some(text.as_str()),
        "rendered_tool_prompt must expose the rendered asset"
    );
    assert!(
        rendered_tool_prompt("write").is_none(),
        "tools without a pinned render context expose no rendered prompt"
    );
}

#[test]
fn artifact_uri_fixture_matches_the_pinned_parser_contract() {
    let fixture: serde_json::Value =
        serde_json::from_str(&read_snapshot_text("golden/artifacts.json"))
            .expect("artifact fixture parses");
    assert_eq!(fixture["source_commit"], PINNED_COMMIT);
    assert_eq!(fixture["allocation"]["first_id"], 0);
    assert_eq!(
        fixture["allocation"]["scope"],
        "parent_and_subagent_tree_share_one_namespace"
    );

    let cases = fixture["cases"].as_array().expect("artifact cases");
    let zero = cases
        .iter()
        .find(|case| case["id"] == "zero_is_valid")
        .expect("zero case");
    let parsed: ArtifactRef = zero["input"]
        .as_str()
        .expect("zero input")
        .parse()
        .expect("zero parses");
    assert_eq!(parsed.id().get(), 0);

    for case_id in ["missing_id", "non_numeric_id"] {
        let case = cases
            .iter()
            .find(|case| case["id"] == case_id)
            .unwrap_or_else(|| panic!("missing {case_id}"));
        let error = case["input"]
            .as_str()
            .expect("error input")
            .parse::<ArtifactRef>()
            .expect_err("fixture input must fail");
        assert_eq!(error.to_string(), case["error"]);
    }
}
