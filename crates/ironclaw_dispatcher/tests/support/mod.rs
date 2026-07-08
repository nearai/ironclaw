#![allow(dead_code)]

pub fn legacy_capability_fixture_to_v2(manifest: &str) -> String {
    if manifest.contains("schema_version") {
        return project_top_level_capabilities_to_host_api(manifest.to_string());
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
    project_top_level_capabilities_to_host_api(converted)
}

/// Project a v2-legacy fixture (top-level `[[capabilities]]`) onto the
/// host_api capability-provider form the parser requires.
fn project_top_level_capabilities_to_host_api(manifest: String) -> String {
    if !manifest.contains("[[capabilities]]") || manifest.contains("[[host_api]]") {
        return manifest;
    }
    let host_api_block = "[[host_api]]\nid = \"ironclaw.capability_provider/v1\"\nsection = \"capability_provider.tools\"\n\n[capability_provider.tools]\n\n";
    let idx = manifest.find("[[capabilities]]").expect("checked above");
    let mut out = String::with_capacity(manifest.len() + host_api_block.len());
    out.push_str(&manifest[..idx]);
    out.push_str(host_api_block);
    out.push_str(&manifest[idx..]);
    out.replace(
        "[[capabilities]]",
        "[[capability_provider.tools.capabilities]]",
    )
}
