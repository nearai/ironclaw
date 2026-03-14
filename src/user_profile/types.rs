use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Categories of user profile facts.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactCategory {
    /// User preferences (timezone, language, tool preferences).
    Preference,
    /// Technical expertise areas.
    Expertise,
    /// Communication style (verbosity, formality, language).
    Style,
    /// Contextual information (current project, role, team).
    Context,
}

impl FactCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Preference => "preference",
            Self::Expertise => "expertise",
            Self::Style => "style",
            Self::Context => "context",
        }
    }

    /// Parse from database string. Returns None for unknown categories.
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "preference" => Some(Self::Preference),
            "expertise" => Some(Self::Expertise),
            "style" => Some(Self::Style),
            "context" => Some(Self::Context),
            _ => None,
        }
    }
}

impl std::fmt::Display for FactCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How a fact was learned.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactSource {
    /// User explicitly stated it.
    Explicit,
    /// Inferred from behavior patterns.
    Inferred,
    /// User corrected a previous inference.
    Corrected,
}

impl FactSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::Inferred => "inferred",
            Self::Corrected => "corrected",
        }
    }
}

/// A single fact about a user, stored encrypted at rest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileFact {
    pub category: FactCategory,
    pub key: String,
    pub value: String, // plaintext (only in memory, encrypted at rest)
    pub confidence: f32,
    pub source: FactSource,
    pub updated_at: DateTime<Utc>,
}

/// Assembled user profile for system prompt injection.
#[derive(Debug, Clone, Default)]
pub struct UserProfile {
    pub facts: Vec<ProfileFact>,
}

impl UserProfile {
    /// Format profile for inclusion in the system prompt.
    /// Capped at `max_chars` to stay within token budget.
    pub fn format_for_prompt(&self, max_chars: usize) -> String {
        if self.facts.is_empty() {
            return String::new();
        }

        let mut sections: std::collections::BTreeMap<String, Vec<String>> =
            std::collections::BTreeMap::new();

        for fact in &self.facts {
            sections
                .entry(fact.category.as_str().to_string())
                .or_default()
                .push(format!("- {}: {}", fact.key, fact.value));
        }

        let mut output = String::from("## User Profile\n\n");
        for (category, entries) in &sections {
            // Capitalize first letter
            let title: String = category
                .chars()
                .enumerate()
                .map(|(i, c)| if i == 0 { c.to_ascii_uppercase() } else { c })
                .collect();
            output.push_str(&format!("### {title}\n"));
            for entry in entries {
                output.push_str(entry);
                output.push('\n');
            }
            output.push('\n');
        }

        // Truncate to budget (char boundary safe)
        if output.len() > max_chars {
            let mut end = max_chars;
            while end > 0 && !output.is_char_boundary(end) {
                end -= 1;
            }
            output.truncate(end);
            output.push_str("\n[profile truncated]");
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_format_for_prompt() {
        let profile = UserProfile {
            facts: vec![ProfileFact {
                category: FactCategory::Preference,
                key: "timezone".into(),
                value: "Europe/Rome".into(),
                confidence: 0.9,
                source: FactSource::Explicit,
                updated_at: chrono::Utc::now(),
            }],
        };
        let output = profile.format_for_prompt(1000);
        assert!(output.contains("timezone"));
        assert!(output.contains("Europe/Rome"));
        assert!(output.contains("### Preference"));
    }

    #[test]
    fn test_empty_profile_returns_empty_string() {
        let profile = UserProfile::default();
        assert!(profile.format_for_prompt(1000).is_empty());
    }

    #[test]
    fn test_profile_truncation_respects_char_boundaries() {
        let profile = UserProfile {
            facts: vec![ProfileFact {
                category: FactCategory::Context,
                key: "project".into(),
                value: "\u{041F}\u{0440}\u{043E}\u{0435}\u{043A}\u{0442}".into(), // "Проект"
                confidence: 0.8,
                source: FactSource::Inferred,
                updated_at: chrono::Utc::now(),
            }],
        };
        let output = profile.format_for_prompt(50);
        // Must not panic on multi-byte char boundary
        assert!(output.len() <= 70); // 50 + "[profile truncated]" overhead
    }

    #[test]
    fn test_fact_category_roundtrip() {
        for cat in [
            FactCategory::Preference,
            FactCategory::Expertise,
            FactCategory::Style,
            FactCategory::Context,
        ] {
            assert_eq!(FactCategory::from_str_opt(cat.as_str()), Some(cat));
        }
        assert_eq!(FactCategory::from_str_opt("unknown"), None);
    }
}
