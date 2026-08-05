//! Hot-swappable skill-activation strategies, following the memory-provider pattern: a named set,
//! a fail-closed binding, and a default that preserves existing behavior.
//!
//! Scoring reads only `activation.keywords`/`tags`/`patterns`, so `name` and `description`
//! contribute nothing. That is fine for curated skills and fatal for self-authored ones: on the
//! 31-task subset in nearai/benchmarks#287, **0 of 30** agent-authored skills carried an
//! `activation` block, so each scored 0 and could never be selected again.
//! [`ActivationStrategy::NameAndDescription`] ports Claude Code's contract, where name and
//! description alone make a skill selectable.

use crate::types::SkillManifest;

/// Host-defined capability profile this module implements. Providers declare it
/// in their extension manifest; see `docs/skills/activation.md`.
pub const PROFILE_ID: &str = "skill.activation.v1";

/// Sentinel binding that disables criteria-based activation entirely, leaving
/// only explicit `$name` mentions and `skill_activate` calls. Non-production only.
pub const ACTIVATION_DISABLED_SENTINEL: &str = "skill.activation.disabled";

/// Which activation strategy is bound to [`PROFILE_ID`].
/// `Copy` so it can live in [`crate::selector::SkillSelectionOptions`], which is
/// passed by value through the per-skill scoring loop. The third-party variant
/// therefore carries a `&'static str` id (interned by composition at bind time)
/// rather than an owned `String`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActivationStrategy {
    /// Score from `activation.keywords`/`tags`/`patterns` only.
    ///
    /// The historical behavior, preserved as the default so bindings that do not
    /// opt in are byte-identical to before. A skill with no `activation` block is
    /// unreachable under this strategy.
    #[default]
    CriteriaOnly,
    /// Score from activation metadata **when present**, and fall back to the
    /// skill's `name` and `description` when it is absent or yields nothing.
    ///
    /// This is the Claude-Code-equivalent contract: authoring a skill requires no
    /// activation metadata, so agent-authored skills stay selectable. Curated
    /// skills are unaffected — their explicit keywords still score first and
    /// still outrank a name/description match.
    NameAndDescription,
    /// No criteria-based activation; explicit mention or `skill_activate` only.
    Disabled,
    /// A third-party strategy extension. Permitted in production only with an
    /// admin override; the dispatch site fails closed if it cannot construct it.
    ThirdParty { extension_id: &'static str },
}

/// Points awarded for a whole-word hit in the skill `name`.
///
/// Below `KEYWORD_EXACT` (10) on purpose: an explicit curated keyword must always
/// outrank an incidental name collision, so adding this fallback can never
/// reorder two skills that both declare keywords.
pub const NAME_WORD_SCORE: u32 = 8;
/// Points for a whole-word hit in the skill `description`. Weakest signal.
pub const DESCRIPTION_WORD_SCORE: u32 = 2;
/// Cap on the fallback contribution, mirroring `MAX_KEYWORD_SCORE`'s role of
/// preventing a verbose description from dominating selection.
pub const MAX_FALLBACK_SCORE: u32 = 20;

impl ActivationStrategy {
    /// Behavior-preserving default: criteria only.
    pub fn criteria_only() -> Self {
        Self::CriteriaOnly
    }

    /// Whether this strategy may fall back to name/description scoring.
    pub fn allows_name_fallback(&self) -> bool {
        matches!(self, Self::NameAndDescription)
    }

    /// Whether criteria-based activation runs at all.
    pub fn criteria_enabled(&self) -> bool {
        !matches!(self, Self::Disabled)
    }

    /// Parse a binding id, as it appears in `[skills] activation = "..."`.
    pub fn parse(value: &str) -> Result<Self, ActivationBindingError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "criteria" | "criteria_only" => Ok(Self::CriteriaOnly),
            "name_and_description" | "name+description" => Ok(Self::NameAndDescription),
            s if s == ACTIVATION_DISABLED_SENTINEL || s == "disabled" => Ok(Self::Disabled),
            other => {
                if let Some(id) = other.strip_prefix("ext:") {
                    if id.is_empty() {
                        return Err(ActivationBindingError::UnknownStrategy(other.to_string()));
                    }
                    // Leak is bounded and intentional: bindings are resolved once
                    // at composition time, not per request.
                    Ok(Self::ThirdParty {
                        extension_id: Box::leak(id.to_string().into_boxed_str()),
                    })
                } else {
                    Err(ActivationBindingError::UnknownStrategy(other.to_string()))
                }
            }
        }
    }

    /// Stable id for diagnostics and config round-tripping.
    pub fn id(&self) -> String {
        match self {
            Self::CriteriaOnly => "criteria".to_string(),
            Self::NameAndDescription => "name_and_description".to_string(),
            Self::Disabled => ACTIVATION_DISABLED_SENTINEL.to_string(),
            Self::ThirdParty { extension_id } => format!("ext:{extension_id}"),
        }
    }
}

/// Binding failed to resolve. Callers fail closed rather than silently
/// downgrading to a different strategy.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActivationBindingError {
    #[error("unknown skill-activation strategy {0:?}")]
    UnknownStrategy(String),
    #[error(
        "skill-activation strategy {0:?} is not permitted in a production deployment \
         without an admin override"
    )]
    NotPermittedInProduction(String),
}

/// Fallback score from a skill's `name`/`description`, used only when the bound
/// strategy allows it and the criteria pass produced nothing.
///
/// Whole-word matching only: a substring rule would make short names like `pdf`
/// match `pdfs`, `pdf-forms` and any prose mentioning them, which is exactly the
/// over-selection that makes injecting an unrelated skill bank harmful.
pub fn fallback_score(manifest: &SkillManifest, message_lower: &str) -> u32 {
    let words: Vec<&str> = message_lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2)
        .collect();
    if words.is_empty() {
        return 0;
    }
    let mut score = 0u32;
    for token in tokenize(&manifest.name) {
        if words.iter().any(|w| *w == token) {
            score += NAME_WORD_SCORE;
        }
    }
    for token in tokenize(&manifest.description) {
        if words.iter().any(|w| *w == token) {
            score += DESCRIPTION_WORD_SCORE;
        }
    }
    score.min(MAX_FALLBACK_SCORE)
}

/// Lowercase alphanumeric tokens of length > 2, splitting kebab/snake/space.
/// Stop-words are dropped so a description's filler can't accumulate score.
fn tokenize(text: &str) -> Vec<String> {
    const STOP: &[&str] = &[
        "the", "and", "for", "with", "from", "this", "that", "use", "using", "when", "into",
        "your", "you", "are", "was", "not", "but", "all", "any", "its", "how", "why", "via",
    ];
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2 && !STOP.contains(w))
        .map(|w| w.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a manifest through the REAL parser, with no `activation:` block —
    /// exactly the shape agents write for themselves (measured 0/30 with one).
    fn manifest(name: &str, description: &str) -> SkillManifest {
        let md = format!(
            "---\nname: {name}\nversion: \"1.0.0\"\ndescription: {description}\n---\n\n# {name}\n"
        );
        let parsed = crate::parse_skill_md(&md).expect("fixture parses");
        assert!(
            parsed.manifest.activation.keywords.is_empty(),
            "fixture must have NO activation keywords"
        );
        parsed.manifest
    }

    #[test]
    fn default_is_behavior_preserving() {
        assert_eq!(
            ActivationStrategy::default(),
            ActivationStrategy::CriteriaOnly
        );
        assert!(!ActivationStrategy::default().allows_name_fallback());
        assert!(ActivationStrategy::default().criteria_enabled());
    }

    #[test]
    fn parses_known_bindings_and_rejects_unknown() {
        for (s, want) in [
            ("", ActivationStrategy::CriteriaOnly),
            ("criteria", ActivationStrategy::CriteriaOnly),
            (
                "name_and_description",
                ActivationStrategy::NameAndDescription,
            ),
            (" Name+Description ", ActivationStrategy::NameAndDescription),
            ("disabled", ActivationStrategy::Disabled),
        ] {
            assert_eq!(ActivationStrategy::parse(s).expect("parses"), want, "{s:?}");
        }
        assert_eq!(
            ActivationStrategy::parse("ext:acme.skills").expect("parses"),
            ActivationStrategy::ThirdParty {
                extension_id: "acme.skills"
            }
        );
        assert!(matches!(
            ActivationStrategy::parse("nonsense"),
            Err(ActivationBindingError::UnknownStrategy(_))
        ));
        // fail closed, not silently to a default
        assert!(ActivationStrategy::parse("ext:").is_err());
    }

    #[test]
    fn binding_ids_round_trip() {
        for s in [
            ActivationStrategy::CriteriaOnly,
            ActivationStrategy::NameAndDescription,
            ActivationStrategy::Disabled,
            ActivationStrategy::ThirdParty {
                extension_id: "acme",
            },
        ] {
            assert_eq!(ActivationStrategy::parse(&s.id()).expect("round trip"), s);
        }
    }

    /// The case that motivates the module: an agent-authored skill with no
    /// `activation` block. Measured 0/30 such skills had one, so under
    /// `CriteriaOnly` they are permanently unreachable.
    #[test]
    fn name_fallback_selects_a_skill_with_no_activation_metadata() {
        let m = manifest(
            "hp-filter-detrending",
            "Detrend an economic time series with the HP filter and correlate",
        );
        let msg = "detrend the gdp series using an hp filter then report the correlation";
        assert!(
            fallback_score(&m, msg) > 0,
            "must be selectable from name/description"
        );
        // and it stays zero when genuinely irrelevant
        assert_eq!(
            fallback_score(&m, "fill in the pdf court form checkboxes"),
            0
        );
    }

    #[test]
    fn name_hit_outranked_by_an_explicit_curated_keyword() {
        // NAME_WORD_SCORE must stay below the selector's exact-keyword award so
        // adding the fallback can never reorder two skills that both declare
        // keywords.
        const { assert!(NAME_WORD_SCORE < 10, "curated keywords must win") };
        const { assert!(DESCRIPTION_WORD_SCORE < NAME_WORD_SCORE) };
    }

    #[test]
    fn fallback_is_capped_so_a_verbose_description_cannot_dominate() {
        let m = manifest(
            "pdf",
            &"pdf form field annotate flatten redact verify checkbox extract merge split"
                .repeat(20),
        );
        let msg = "pdf form field annotate flatten redact verify checkbox extract merge split";
        assert_eq!(fallback_score(&m, msg), MAX_FALLBACK_SCORE);
    }

    #[test]
    fn whole_word_only_so_short_names_do_not_over_match() {
        let m = manifest("pdf", "PDF tooling");
        // substring matching would fire on "pdfs"/"pdf-forms"; whole-word must not
        assert_eq!(fallback_score(&m, "convert the pdfs in the folder"), 0);
        assert!(fallback_score(&m, "edit the pdf") > 0);
    }

    #[test]
    fn stop_words_do_not_accumulate_score() {
        let m = manifest("the-and-for", "the and for with from this that");
        assert_eq!(fallback_score(&m, "the and for with from this that"), 0);
    }
}
