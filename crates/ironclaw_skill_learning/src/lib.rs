#![forbid(unsafe_code)]

//! Skill learning logic for IronClaw Reborn.
//!
//! Distills a reusable `SKILL.md` from a completed run's transcript. (Refinement
//! and library upkeep land here too in later increments.)
//!
//! This crate is **pure domain logic**: it does not depend on the LLM provider,
//! the runtime, or the filesystem. Inference is abstracted behind
//! [`SkillInferencePort`], and the produced document is validated with the same
//! parser the skill-install path uses ([`ironclaw_skills::parse_skill_md`]), so a
//! distilled skill is guaranteed installable. The composition layer supplies the
//! concrete inference adapter (over the runtime's non-run inference port) and the
//! scoped write; neither concern leaks into this crate.

use async_trait::async_trait;
use ironclaw_skills::{SkillParseError, parse_skill_md};

/// The extraction prompt (transcript -> `SKILL.md` or a `SKIP:` line). Kept next
/// to the parser whose output contract it must satisfy.
const SKILL_EXTRACTION_PROMPT: &str = include_str!("../prompts/skill_extraction.md");

/// The refinement prompt (existing + candidate `SKILL.md` -> improved `SKILL.md`
/// or a `KEEP` line). Drives the self-evolution step: each time a learned task
/// recurs, the skill folds in the new evidence and gets strictly better.
const SKILL_REFINEMENT_PROMPT: &str = include_str!("../prompts/skill_refinement.md");

/// Single-shot inference: system instructions + user content -> text.
///
/// Implemented by the composition layer over the runtime's non-run inference
/// port so this crate stays free of any runtime/LLM dependency.
#[async_trait]
pub trait SkillInferencePort: Send + Sync {
    async fn infer(&self, system: &str, user: &str) -> Result<String, SkillInferenceError>;
}

/// Opaque inference failure. The concrete adapter maps provider/runtime errors
/// into this so the logic crate never names them.
#[derive(Debug, thiserror::Error)]
#[error("skill inference failed: {0}")]
pub struct SkillInferenceError(pub String);

/// A skill distilled from a transcript, validated and ready to install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DistilledSkill {
    /// Stable skill name parsed from the `SKILL.md` frontmatter.
    pub name: String,
    /// The full `SKILL.md` document (frontmatter + body).
    pub skill_md: String,
}

/// Why a distillation attempt produced no skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoSkillReason {
    /// The model judged the run not worth distilling (carries the `SKIP:` reason).
    NotSkillWorthy(String),
}

/// Outcome of a distillation attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DistillOutcome {
    /// A validated, installable skill.
    Skill(DistilledSkill),
    /// The model declined to produce a skill.
    Skipped(NoSkillReason),
}

/// Distillation failure.
#[derive(Debug, thiserror::Error)]
pub enum DistillError {
    /// The inference call itself failed.
    #[error(transparent)]
    Inference(#[from] SkillInferenceError),
    /// The model produced something that is not a valid `SKILL.md`.
    #[error("model produced an unparseable SKILL.md: {0}")]
    Unparseable(#[from] SkillParseError),
    /// The model returned an empty response.
    #[error("model produced an empty response")]
    EmptyResponse,
}

/// Distill a skill from a completed run's transcript.
///
/// Calls the inference port with the extraction prompt + transcript, then
/// validates the result with [`ironclaw_skills::parse_skill_md`]. Returns
/// [`DistillOutcome::Skipped`] when the model declines, or a validated
/// [`DistilledSkill`].
pub async fn distill_skill(
    transcript: &str,
    inference: &dyn SkillInferencePort,
) -> Result<DistillOutcome, DistillError> {
    let raw = inference.infer(SKILL_EXTRACTION_PROMPT, transcript).await?;
    parse_distillation(&raw)
}

/// Parse a raw model response into a [`DistillOutcome`].
///
/// Pure and unit-tested: tolerates an accidental ```` ``` ```` fence wrap and a
/// leading `SKIP:` decline, and validates any candidate document with the
/// install-path parser so only installable skills come back as
/// [`DistillOutcome::Skill`].
pub fn parse_distillation(raw: &str) -> Result<DistillOutcome, DistillError> {
    let cleaned = strip_code_fence(raw.trim());
    if cleaned.is_empty() {
        return Err(DistillError::EmptyResponse);
    }
    if let Some(rest) = cleaned.strip_prefix("SKIP") {
        let reason = rest.trim_start_matches([':', ' ', '-', '\t']).trim();
        return Ok(DistillOutcome::Skipped(NoSkillReason::NotSkillWorthy(
            reason.to_string(),
        )));
    }
    let parsed = parse_skill_md(cleaned)?;
    Ok(DistillOutcome::Skill(DistilledSkill {
        name: parsed.manifest.name,
        skill_md: cleaned.to_string(),
    }))
}

/// Outcome of a refinement attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefineOutcome {
    /// A refined, validated, installable skill. Its `name` is whatever the model
    /// emitted (the refinement prompt instructs it to preserve the existing
    /// name; the caller still retargets defensively before install).
    Refined(DistilledSkill),
    /// The existing skill already subsumes the candidate; keep it unchanged.
    KeepExisting,
}

/// Refine an existing skill with a freshly distilled candidate for the same kind
/// of task — the self-evolution step. The model folds the candidate's new
/// evidence (clearer steps, the union of real gotchas, a bumped version) into the
/// existing skill, or returns [`RefineOutcome::KeepExisting`] when the existing
/// already covers everything useful. The produced document is validated with
/// [`ironclaw_skills::parse_skill_md`], so only an installable skill comes back
/// as [`RefineOutcome::Refined`].
pub async fn refine_skill(
    existing_skill_md: &str,
    candidate_skill_md: &str,
    inference: &dyn SkillInferencePort,
) -> Result<RefineOutcome, DistillError> {
    let user = format!(
        "# Existing SKILL.md\n\n{existing_skill_md}\n\n\
         # Newly distilled candidate SKILL.md (same task)\n\n{candidate_skill_md}"
    );
    let raw = inference.infer(SKILL_REFINEMENT_PROMPT, &user).await?;
    parse_refinement(&raw)
}

/// Parse a raw refinement response into a [`RefineOutcome`].
///
/// Pure and unit-tested: tolerates an accidental ```` ``` ```` fence wrap and a
/// leading `KEEP` decline, and validates any candidate document with the
/// install-path parser so only installable skills come back as
/// [`RefineOutcome::Refined`].
pub fn parse_refinement(raw: &str) -> Result<RefineOutcome, DistillError> {
    let cleaned = strip_code_fence(raw.trim());
    if cleaned.is_empty() {
        return Err(DistillError::EmptyResponse);
    }
    // A real refined document begins with the `---` frontmatter opener, so a
    // leading `KEEP` (any case) is unambiguously the decline sentinel.
    if cleaned
        .lines()
        .next()
        .is_some_and(|line| line.trim().to_ascii_uppercase().starts_with("KEEP"))
    {
        return Ok(RefineOutcome::KeepExisting);
    }
    let parsed = parse_skill_md(cleaned)?;
    Ok(RefineOutcome::Refined(DistilledSkill {
        name: parsed.manifest.name,
        skill_md: cleaned.to_string(),
    }))
}

/// Strip a single wrapping ```` ``` ```` fence (with optional language tag) when
/// the model wraps its `SKILL.md` despite being told not to. Returns the input
/// unchanged when it is not fence-wrapped.
fn strip_code_fence(text: &str) -> &str {
    let trimmed = text.trim();
    let Some(after_open) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    // Drop the rest of the opening-fence line (an optional language tag).
    let Some(newline) = after_open.find('\n') else {
        return trimmed;
    };
    let body = &after_open[newline + 1..];
    match body.rfind("```") {
        Some(close) => body[..close].trim(),
        None => trimmed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_SKILL: &str = "---\n\
name: github-issue-triage\n\
version: 1\n\
description: Triage incoming GitHub issues\n\
activation:\n\
  keywords: [github, issue]\n\
---\n\
\n\
# GitHub Issue Triage\n\
\n\
## When this helps\n\
\n\
A new GitHub issue needs labels and a first response.\n";

    struct StubInference {
        response: String,
    }

    #[async_trait]
    impl SkillInferencePort for StubInference {
        async fn infer(&self, _system: &str, _user: &str) -> Result<String, SkillInferenceError> {
            Ok(self.response.clone())
        }
    }

    #[test]
    fn parses_a_valid_skill_and_extracts_the_name() {
        let outcome = parse_distillation(VALID_SKILL).expect("valid skill parses");
        match outcome {
            DistillOutcome::Skill(skill) => {
                assert_eq!(skill.name, "github-issue-triage");
                assert!(skill.skill_md.contains("## When this helps"));
            }
            other => panic!("expected a skill, got {other:?}"),
        }
    }

    #[test]
    fn strips_an_accidental_code_fence_wrap() {
        let wrapped = format!("```markdown\n{VALID_SKILL}```");
        let outcome = parse_distillation(&wrapped).expect("fenced skill parses");
        match outcome {
            DistillOutcome::Skill(skill) => {
                assert_eq!(skill.name, "github-issue-triage");
                assert!(!skill.skill_md.contains("```"), "fence must be stripped");
            }
            other => panic!("expected a skill, got {other:?}"),
        }
    }

    #[test]
    fn treats_a_skip_line_as_not_skill_worthy() {
        let outcome = parse_distillation("SKIP: trivial one-off question").expect("skip parses");
        assert_eq!(
            outcome,
            DistillOutcome::Skipped(NoSkillReason::NotSkillWorthy(
                "trivial one-off question".to_string()
            ))
        );
    }

    #[test]
    fn empty_response_is_an_error() {
        assert!(matches!(
            parse_distillation("   "),
            Err(DistillError::EmptyResponse)
        ));
    }

    #[test]
    fn non_skill_text_is_rejected_by_the_install_parser() {
        // Anything that isn't `SKIP` and isn't a valid SKILL.md must fail, so a
        // chatty model response never reaches the install path.
        assert!(matches!(
            parse_distillation("Sure! Here is a summary of what I did..."),
            Err(DistillError::Unparseable(_))
        ));
    }

    #[tokio::test]
    async fn distill_skill_runs_inference_then_validates() {
        let inference = StubInference {
            response: VALID_SKILL.to_string(),
        };
        let outcome = distill_skill("user: triage issue 42\nassistant: done", &inference)
            .await
            .expect("distillation succeeds");
        match outcome {
            DistillOutcome::Skill(skill) => assert_eq!(skill.name, "github-issue-triage"),
            other => panic!("expected a skill, got {other:?}"),
        }
    }

    const REFINED_SKILL: &str = "---\n\
name: github-issue-triage\n\
version: 2\n\
description: Triage incoming GitHub issues\n\
activation:\n\
  keywords: [github, issue, label]\n\
---\n\
\n\
# GitHub Issue Triage\n\
\n\
## Gotchas\n\
\n\
- Rate-limited after 30 calls; back off.\n";

    #[test]
    fn parse_refinement_accepts_a_refined_skill() {
        match parse_refinement(REFINED_SKILL).expect("refined skill parses") {
            RefineOutcome::Refined(skill) => {
                assert_eq!(skill.name, "github-issue-triage");
                assert!(skill.skill_md.contains("version: 2"));
                assert!(skill.skill_md.contains("Rate-limited"));
            }
            other => panic!("expected a refined skill, got {other:?}"),
        }
    }

    #[test]
    fn parse_refinement_treats_keep_as_keep_existing() {
        assert_eq!(
            parse_refinement("KEEP").expect("keep parses"),
            RefineOutcome::KeepExisting
        );
        assert_eq!(
            parse_refinement("keep — existing already covers it").expect("keep prose parses"),
            RefineOutcome::KeepExisting
        );
    }

    #[test]
    fn parse_refinement_rejects_chatty_non_skill_output() {
        assert!(matches!(
            parse_refinement("Here is the merged skill for you:"),
            Err(DistillError::Unparseable(_))
        ));
    }

    #[tokio::test]
    async fn refine_skill_runs_inference_then_validates() {
        let inference = StubInference {
            response: REFINED_SKILL.to_string(),
        };
        let outcome = refine_skill(VALID_SKILL, VALID_SKILL, &inference)
            .await
            .expect("refinement succeeds");
        match outcome {
            RefineOutcome::Refined(skill) => {
                assert_eq!(skill.name, "github-issue-triage");
                assert!(skill.skill_md.contains("version: 2"));
            }
            other => panic!("expected a refined skill, got {other:?}"),
        }
    }
}
