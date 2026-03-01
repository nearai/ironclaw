use std::sync::LazyLock;

use regex::Regex;

/// Classifies content as potentially sensitive for privacy purposes.
///
/// Used to guard writes to shared memory layers -- if content is flagged
/// as sensitive, it can be redirected to the private layer instead.
pub trait PrivacyClassifier: Send + Sync {
    /// Returns true if the content appears to contain private/sensitive information.
    fn is_sensitive(&self, content: &str) -> bool;
}

/// Pattern-based privacy classifier using regex matching.
///
/// Detects PII patterns (SSN, credit card, phone), health/medical terms,
/// and personal sentiment markers.
///
/// # Limitations
///
/// This classifier uses simple regex patterns and has known limitations:
///
/// - **Bypass**: Sensitive data can be obfuscated to evade detection —
///   base64 encoding, whitespace insertion, Unicode lookalikes, natural
///   language paraphrasing, or splitting across multiple writes.
/// - **False positives**: The email regex matches any email address, so
///   intentional shared content like "send to team@company.com" triggers
///   a redirect. Medical terms like "treatment" or "doctor" match in
///   non-medical contexts.
/// - **Append gap**: When used with `append_to_layer`, only the new content
///   being appended is classified, not the full document after concatenation.
///   Sensitive data split across multiple individually-benign appends will
///   not be detected.
///
/// The classifier is pluggable via the [`PrivacyClassifier`] trait.
/// Operators who find the false-positive rate unacceptable can implement
/// a custom classifier or disable auto-redirect by not configuring a
/// shared layer with `sensitivity = "shared"`.
pub struct PatternPrivacyClassifier {
    patterns: Vec<Regex>,
}

impl Default for PatternPrivacyClassifier {
    fn default() -> Self {
        Self::new()
    }
}

impl PatternPrivacyClassifier {
    pub fn new() -> Self {
        let pattern_strs = [
            // SSN
            r"\b\d{3}-\d{2}-\d{4}\b",
            // Credit card (basic)
            r"\b\d{4}[\s-]?\d{4}[\s-]?\d{4}[\s-]?\d{4}\b",
            // Email (as PII in context)
            r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b",
            // Phone numbers (US)
            r"\b\(?\d{3}\)?[\s.-]?\d{3}[\s.-]?\d{4}\b",
            // Health/medical terms
            r"(?i)\b(diagnosis|prescription|medication|therapy|doctor|medical|symptom|treatment|illness|disease|mental health|anxiety|depression)\b",
            // Highly personal markers
            r"(?i)\b(password|secret|confession|affair|divorce|pregnant|rehab|addiction)\b",
        ];
        let patterns = pattern_strs
            .iter()
            .map(|p| Regex::new(p).expect("hardcoded regex must compile"))
            .collect();
        Self { patterns }
    }
}

static GLOBAL_CLASSIFIER: LazyLock<PatternPrivacyClassifier> =
    LazyLock::new(PatternPrivacyClassifier::new);

pub fn global_classifier() -> &'static PatternPrivacyClassifier {
    &GLOBAL_CLASSIFIER
}

impl PrivacyClassifier for PatternPrivacyClassifier {
    fn is_sensitive(&self, content: &str) -> bool {
        self.patterns.iter().any(|p| p.is_match(content))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classifier() -> PatternPrivacyClassifier {
        PatternPrivacyClassifier::new()
    }

    #[test]
    fn detects_ssn() {
        assert!(classifier().is_sensitive("My SSN is 123-45-6789"));
    }

    #[test]
    fn detects_credit_card() {
        assert!(classifier().is_sensitive("Card: 4111 1111 1111 1111"));
    }

    #[test]
    fn detects_medical_terms() {
        assert!(classifier().is_sensitive("Started new medication for anxiety"));
    }

    #[test]
    fn detects_personal_markers() {
        assert!(classifier().is_sensitive("This is a secret I haven't told anyone"));
    }

    #[test]
    fn allows_normal_household_content() {
        assert!(!classifier().is_sensitive("We need to buy groceries for dinner Saturday"));
    }

    #[test]
    fn allows_normal_finance_content() {
        assert!(!classifier().is_sensitive("Electric bill was $120 this month"));
    }
}
