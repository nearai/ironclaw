#![no_main]
use libfuzzer_sys::fuzz_target;
use ironclaw::safety::Validator;
use ironclaw::tools::validate_tool_schema;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        // Try parsing as JSON and validating as tool parameters
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(s) {
            // Exercise Validator::validate_tool_params with arbitrary JSON
            let validator = Validator::new();
            let result = validator.validate_tool_params(&value);
            // Invariant: result should always be well-formed
            if !result.is_valid {
                assert!(!result.errors.is_empty());
            }

            // Exercise validate_tool_schema with arbitrary JSON as a schema
            let _ = validate_tool_schema(&value, "fuzz");
        }
    }
});
