use regex::Regex;

/// Result of privacy classification, including confidence level.
///
/// Confidence enables downstream callers to apply thresholds (e.g., only
/// redirect above 0.8) and supports future upgrade to LLM-based classifiers
/// that produce probabilistic scores.
#[derive(Debug, Clone)]
pub struct SensitivityResult {
    pub is_sensitive: bool,
    pub confidence: f32,
}

/// Classifies content as potentially sensitive for privacy purposes.
///
/// Used to guard writes to shared memory layers -- if content is flagged
/// as sensitive, it can be redirected to the private layer instead.
pub trait PrivacyClassifier: Send + Sync {
    /// Classify content and return sensitivity with confidence score.
    fn classify(&self, content: &str) -> SensitivityResult;
}

/// Pattern-based privacy classifier using regex matching.
///
/// Default patterns target hard PII (SSN, credit card numbers) where silent
/// redirect is clearly correct. Ambiguous terms (health vocabulary, contact
/// info) are intentionally excluded — they cause false positives in household
/// contexts and silently redirect content the user intended to share.
///
/// Operators who need broader coverage should use `ConfigurablePrivacyClassifier`
/// with domain-specific patterns.
pub struct PatternPrivacyClassifier {
    patterns: Vec<Regex>,
}

impl PatternPrivacyClassifier {
    pub fn new() -> Result<Self, regex::Error> {
        let pattern_strs = [
            // SSN — always PII
            r"\b\d{3}-\d{2}-\d{4}\b",
            // Credit card (basic) — always PII
            r"\b\d{4}[\s-]?\d{4}[\s-]?\d{4}[\s-]?\d{4}\b",
            // Credentials and auth tokens — high-confidence PII
            r"(?i)\b(password|passwd|api[_-]?key|auth[_-]?token|secret[_-]?key)\b",
        ];
        let patterns = pattern_strs
            .iter()
            .map(|p| Regex::new(p))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { patterns })
    }
}

impl PrivacyClassifier for PatternPrivacyClassifier {
    fn classify(&self, content: &str) -> SensitivityResult {
        let is_sensitive = self.patterns.iter().any(|p| p.is_match(content));
        SensitivityResult {
            is_sensitive,
            // Regex is binary — matched or not. Always full confidence.
            confidence: if is_sensitive { 1.0 } else { 0.0 },
        }
    }
}

/// User-configurable privacy classifier.
///
/// Accepts custom regex patterns at construction time, allowing operators
/// to tune sensitivity for their use case (e.g., drop health terms that
/// cause false positives, add domain-specific patterns).
///
/// ```
/// use ironclaw::workspace::privacy::ConfigurablePrivacyClassifier;
/// use ironclaw::workspace::privacy::PrivacyClassifier;
///
/// let classifier = ConfigurablePrivacyClassifier::new(vec![
///     r"\b\d{3}-\d{2}-\d{4}\b".into(),  // SSN only
/// ]).unwrap();
/// assert!(classifier.classify("SSN: 123-45-6789").is_sensitive);
/// assert!(!classifier.classify("saw the doctor today").is_sensitive);
/// ```
pub struct ConfigurablePrivacyClassifier {
    patterns: Vec<Regex>,
}

impl ConfigurablePrivacyClassifier {
    /// Create a classifier from user-supplied regex strings.
    ///
    /// Returns an error if any pattern fails to compile.
    pub fn new(pattern_strs: Vec<String>) -> Result<Self, regex::Error> {
        let patterns = pattern_strs
            .iter()
            .map(|p| Regex::new(p))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { patterns })
    }
}

impl PrivacyClassifier for ConfigurablePrivacyClassifier {
    fn classify(&self, content: &str) -> SensitivityResult {
        let is_sensitive = self.patterns.iter().any(|p| p.is_match(content));
        SensitivityResult {
            is_sensitive,
            confidence: if is_sensitive { 1.0 } else { 0.0 },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classifier() -> PatternPrivacyClassifier {
        PatternPrivacyClassifier::new().unwrap()
    }

    // Hard PII — must always trigger
    #[test]
    fn detects_ssn() {
        let result = classifier().classify("My SSN is 123-45-6789");
        assert!(result.is_sensitive);
        assert_eq!(result.confidence, 1.0);
    }

    #[test]
    fn detects_credit_card() {
        let result = classifier().classify("Card: 4111 1111 1111 1111");
        assert!(result.is_sensitive);
        assert_eq!(result.confidence, 1.0);
    }

    #[test]
    fn detects_password() {
        assert!(classifier().classify("my password is hunter2").is_sensitive);
    }

    #[test]
    fn detects_api_key() {
        assert!(
            classifier()
                .classify("set the api_key to sk-1234")
                .is_sensitive
        );
    }

    // Household content — must NOT trigger (previous false positives)
    #[test]
    fn allows_normal_household_content() {
        let result = classifier().classify("We need to buy groceries for dinner Saturday");
        assert!(!result.is_sensitive);
        assert_eq!(result.confidence, 0.0);
    }

    #[test]
    fn allows_doctor_mention() {
        assert!(
            !classifier()
                .classify("the doctor's office called about Saturday")
                .is_sensitive
        );
    }

    #[test]
    fn allows_email_address() {
        assert!(
            !classifier()
                .classify("email joe@plumber.com about the leak")
                .is_sensitive
        );
    }

    #[test]
    fn allows_phone_number() {
        assert!(
            !classifier()
                .classify("call the restaurant at 555-123-4567")
                .is_sensitive
        );
    }

    #[test]
    fn allows_medical_terms_in_context() {
        assert!(
            !classifier()
                .classify("Started new medication for anxiety")
                .is_sensitive
        );
    }

    #[test]
    fn configurable_with_custom_patterns() {
        let c = ConfigurablePrivacyClassifier::new(vec![
            r"\b\d{3}-\d{2}-\d{4}\b".into(), // SSN only
        ])
        .unwrap();
        assert!(c.classify("SSN: 123-45-6789").is_sensitive);
        // Health terms no longer trigger with SSN-only config
        assert!(!c.classify("saw the doctor today").is_sensitive);
    }

    #[test]
    fn configurable_rejects_bad_regex() {
        let result = ConfigurablePrivacyClassifier::new(vec!["[invalid".into()]);
        assert!(result.is_err());
    }

    #[test]
    fn configurable_empty_patterns_allows_everything() {
        let c = ConfigurablePrivacyClassifier::new(vec![]).unwrap();
        assert!(!c.classify("My SSN is 123-45-6789").is_sensitive);
    }

    // Format variants
    #[test]
    fn detects_credit_card_no_separators() {
        assert!(
            classifier()
                .classify("card 4111111111111111 on file")
                .is_sensitive
        );
    }

    #[test]
    fn detects_credit_card_with_dashes() {
        assert!(
            classifier()
                .classify("Card: 4111-1111-1111-1111")
                .is_sensitive
        );
    }

    #[test]
    fn detects_ssn_bare() {
        assert!(classifier().classify("123-45-6789").is_sensitive);
    }

    #[test]
    fn detects_auth_token_keyword() {
        assert!(
            classifier()
                .classify("set auth_token to abc123")
                .is_sensitive
        );
    }

    #[test]
    fn detects_secret_key_keyword() {
        assert!(
            classifier()
                .classify("the secret_key is sk-prod-xyz")
                .is_sensitive
        );
    }

    #[test]
    fn detects_pii_in_longer_document() {
        let content = "Meeting notes from Thursday.\n\
                        Discussed budget and timeline.\n\
                        SSN is 999-88-7777 for the insurance form.\n\
                        Action items: follow up with vendor.";
        assert!(classifier().classify(content).is_sensitive);
    }

    #[test]
    fn empty_string_is_not_sensitive() {
        assert!(!classifier().classify("").is_sensitive);
    }

    #[test]
    fn partial_ssn_not_sensitive() {
        assert!(
            !classifier()
                .classify("code 123-45 in the system")
                .is_sensitive
        );
    }
}
