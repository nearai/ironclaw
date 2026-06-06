use serde_json::{Value, Map};
use std::fs;

/// IronClaw Custom Skill: Cleans and formats messy or scraped JSON data payloads 
/// within the agent's sandboxed local workspace filesystem.
pub fn clean_workspace_json(file_path: &str) -> Result<String, String> {
    // Read the file securely from the workspace path
    let data = fs::read_to_string(file_path)
        .map_err(|e| format!("Failed to read workspace file: {}", e))?;
        
    // Parse the data framework into a structural JSON value
    let mut json_value: Value = serde_json::from_str(&data)
        .map_err(|e| format!("Invalid JSON syntax or structure: {}", e))?;

    // Execute recursive sanitization if the root item is a JSON object
    if let Value::Object(ref mut map) = json_value {
        clean_map(map);
    }

    // Serialize back into pretty-printed, standardized spacing format
    let cleaned_data = serde_json::to_string_pretty(&json_value)
        .map_err(|e| format!("Failed to serialize clean JSON: {}", e))?;

    Ok(cleaned_data)
}

/// Recursive helper function to strip out null keys and empty strings to optimize context windows
fn clean_map(map: &mut Map<String, Value>) {
    map.retain(|_, v| {
        match v {
            Value::Null => false, // Strip out nulls
            Value::String(s) => !s.trim().is_empty(), // Strip out blank text
            Value::Object(ref mut nested_map) => {
                clean_map(nested_map);
                !nested_map.is_empty() // Remove nested maps if they become empty
            }
            _ => true, // Keep numbers, booleans, and arrays
        }
    });
}
