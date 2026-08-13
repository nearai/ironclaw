use crate::LeakMatch;

/// Stable semantic classes for leak findings whose consumers need behavior
/// beyond the detector's configured action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeakPatternClass {
    AmbiguousHexDigest,
    Secret,
}

impl LeakMatch {
    pub fn pattern_class(&self) -> LeakPatternClass {
        match self.pattern_name.as_str() {
            "high_entropy_hex" => LeakPatternClass::AmbiguousHexDigest,
            _ => LeakPatternClass::Secret,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{LeakDetector, LeakPatternClass};

    #[test]
    fn bare_sha256_digest_has_typed_ambiguous_classification() {
        let digest = "269cc57b4d0c4368d8b02738ab709c810adb6212729b24bbdc34efb539a3ed07";
        let finding = LeakDetector::new()
            .scan(digest)
            .matches
            .into_iter()
            .find(|finding| finding.pattern_class() == LeakPatternClass::AmbiguousHexDigest)
            .expect("bare SHA-256-shaped digest is classified as ambiguous hex");

        assert_eq!(
            finding.pattern_class(),
            LeakPatternClass::AmbiguousHexDigest
        );
    }
}
