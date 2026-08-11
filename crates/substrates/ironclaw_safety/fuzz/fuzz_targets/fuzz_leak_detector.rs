#![no_main]
use ironclaw_safety::LeakDetector;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let detector = LeakDetector::new();

        // Exercise scan path
        let result = detector.scan(s);
        // Invariant: if should_block, there must be matches
        if result.should_block {
            assert!(!result.matches.is_empty());
        }
        // Invariant: match locations must be valid
        for m in &result.matches {
            assert!(m.location.end <= s.len());
        }

        // Exercise scan_and_clean path
        let _ = detector.scan_and_clean(s);
    }
});
