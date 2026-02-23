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
            r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Z|a-z]{2,}\b",
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
