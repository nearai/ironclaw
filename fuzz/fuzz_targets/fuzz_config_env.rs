#![no_main]
use libfuzzer_sys::fuzz_target;

use ironclaw::safety::{LeakDetector, Sanitizer, Validator};

fuzz_target!(|data: &[u8]| {
    if let Ok(input) = std::str::from_utf8(data) {
        // Exercise Sanitizer: detect and neutralize prompt injection attempts.
        let sanitizer = Sanitizer::new();
        let sanitized = sanitizer.sanitize(input);
        // The sanitized content must never be empty when input is non-empty,
        // because sanitization wraps/escapes rather than deleting.
        if !input.is_empty() {
            assert!(
                !sanitized.content.is_empty(),
                "sanitize() produced empty content for non-empty input"
            );
        }
        // If no modification occurred, content must equal input.
        if !sanitized.was_modified {
            assert_eq!(sanitized.content, input);
        }

        // Exercise Validator: input validation (length, encoding, patterns).
        let validator = Validator::new();
        let result = validator.validate(input);
        // ValidationResult must always be well-formed: if valid, no errors.
        if result.is_valid {
            assert!(
                result.errors.is_empty(),
                "valid result should have no errors"
            );
        }

        // Exercise LeakDetector: secret detection (API keys, tokens, etc.).
        let detector = LeakDetector::new();
        let scan = detector.scan(input);
        // scan_and_clean must not panic and must return valid UTF-8.
        let cleaned = detector.scan_and_clean(input);
        if let Ok(ref clean_str) = cleaned {
            // Cleaned output must never be longer than original + redaction markers.
            // At minimum it should be valid UTF-8 (guaranteed by String type).
            let _ = clean_str.len();
        }
        // If scan found no matches, scan_and_clean should return the input unchanged.
        if scan.matches.is_empty() {
            if let Ok(ref clean_str) = cleaned {
                assert_eq!(
                    clean_str, input,
                    "scan_and_clean changed content despite no matches"
                );
            }
        }
    }
});
