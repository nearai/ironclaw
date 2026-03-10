#![no_main]
use libfuzzer_sys::fuzz_target;
use ironclaw::safety::Validator;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let validator = Validator::new();

        // Exercise input validation
        let result = validator.validate(s);
        // Invariant: empty input is always invalid
        if s.is_empty() {
            assert!(!result.is_valid);
        }

        // Exercise tool parameter validation with arbitrary JSON
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(s) {
            let _ = validator.validate_tool_params(&value);
        }
    }
});
