#![allow(dead_code)]

pub fn legacy_capability_fixture_to_v2(manifest: &str) -> String {
    if manifest.contains("schema_version") {
        return manifest.to_string();
    }
    let mut converted = "schema_version = \"reborn.extension_manifest.v2\"\n".to_string();
    for line in manifest.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("parameters_schema") {
            converted.push_str("visibility = \"model\"\n");
            converted.push_str("input_schema_ref = \"schemas/test/input.v1.json\"\n");
            converted.push_str("output_schema_ref = \"schemas/test/output.v1.json\"\n");
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
