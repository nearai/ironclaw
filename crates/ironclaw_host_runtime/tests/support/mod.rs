#![allow(dead_code)]

pub(crate) fn legacy_capability_fixture_to_v2(manifest: &str) -> String {
    legacy_capability_fixture_to_v2_with_refs(manifest, |_| {
        (
            "schemas/test/input.v1.json".to_string(),
            "schemas/test/output.v1.json".to_string(),
        )
    })
}

pub(crate) fn legacy_capability_fixture_to_v2_with_schema_suffix(manifest: &str) -> String {
    legacy_capability_fixture_to_v2_with_refs(manifest, |line| {
        let schema_suffix = line.bytes().fold(0_u64, |acc, byte| {
            acc.wrapping_mul(31).wrapping_add(byte.into())
        });
        (
            format!("schemas/test/{schema_suffix}.input.v1.json"),
            format!("schemas/test/{schema_suffix}.output.v1.json"),
        )
    })
}

fn legacy_capability_fixture_to_v2_with_refs(
    manifest: &str,
    schema_refs: impl Fn(&str) -> (String, String),
) -> String {
    if manifest.contains("schema_version") {
        return manifest.to_string();
    }
    let mut converted = "schema_version = \"reborn.extension_manifest.v2\"\n".to_string();
    for line in manifest.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("parameters_schema") {
            let (input_schema_ref, output_schema_ref) = schema_refs(line);
            converted.push_str("visibility = \"model\"\n");
            converted.push_str(&format!("input_schema_ref = \"{input_schema_ref}\"\n"));
            converted.push_str(&format!("output_schema_ref = \"{output_schema_ref}\"\n"));
            converted.push_str("prompt_doc_ref = \"prompts/test.md\"\n");
        } else if trimmed.starts_with("backend =") {
            converted.push_str(&line.replacen("backend", "runner", 1));
            converted.push('\n');
        } else {
            converted.push_str(line);
            converted.push('\n');
        }
    }
    converted
}
