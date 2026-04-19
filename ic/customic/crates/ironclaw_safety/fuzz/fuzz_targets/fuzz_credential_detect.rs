#![no_main]
use ironclaw_safety::params_contain_manual_credentials;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        // Try parsing as JSON and exercising credential detection
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(s) {
            // Must not panic on any valid JSON input
            let _ = params_contain_manual_credentials(&value);
        }
    }
});
