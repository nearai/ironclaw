//! Provenance sidecar for machine-learned skills.
//!
//! Mirrors [`crate::install_metadata`]: a `.ironclaw-*.json` dotfile written
//! next to `SKILL.md` (skill discovery skips hidden entries, so it is never
//! treated as a bundle file). It records that a skill was authored by the
//! self-evolution learning sink, plus a baseline of what the machine last
//! wrote, so the sink can tell whether a human has since edited the skill — in
//! which case it must stop silently overwriting it and propose instead.
//!
//! The baseline is two parts because the registry's `content_hash` is
//! **body-only** (`compute_hash(prompt_content)`): a human who hand-tunes ANY
//! frontmatter field (activation keywords, description, requirements,
//! credentials, …) would otherwise be invisible. So we also hash the full
//! canonicalized manifest (activation lists sorted, so a pure reorder is not
//! treated as an edit).
//!
//! See `docs/plans/2026-06-19-skill-edit-preservation.md`.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::validation::normalize_line_endings;
use crate::{SkillManifest, SkillParseError, parse_skill_md};

/// Sidecar filename written next to `SKILL.md`. The leading `.` keeps skill
/// discovery (which skips hidden entries) from treating it as a bundle file.
pub const LEARNED_PROVENANCE_FILE_NAME: &str = ".ironclaw-learned.json";

/// Generous cap for the provenance sidecar — it may carry a stashed proposal.
pub const MAX_LEARNED_PROVENANCE_BYTES: usize = 64 * 1024;

/// Provenance sidecar for a machine-learned skill. Its mere PRESENCE marks the
/// skill as written by the learning sink — the sink is the only writer of this
/// dotfile — so the overwrite/divergence gate keys on presence plus the
/// body-hash + activation baseline below (which reveal whether a human has since
/// edited it). A skill's *declared* origin now lives in its SKILL.md frontmatter
/// (`SkillManifest::origin`) so it travels with the skill across export/sharing;
/// this sidecar is the host-private security ledger for the gate only.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedSkillProvenance {
    /// Body hash the machine last wrote, computed identically to
    /// `LoadedSkill::content_hash` (`compute_hash` over the post-frontmatter
    /// prompt content).
    pub last_machine_body_hash: String,
    /// SHA-256 of the full canonicalized frontmatter manifest the machine last
    /// wrote (activation lists sorted). Covers every human-editable frontmatter
    /// field, so an edit to `description`/`requires`/`credentials`/`origin`/
    /// `auto_activate` — not just the activation arrays — counts as a human edit.
    pub last_machine_manifest_hash: String,
    /// A distilled candidate stashed for human review when the live skill is
    /// human-owned (divergent) and must not be overwritten. `None` = no pending
    /// proposal. Phase 2 surfaces a diff/approve UI over this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposed_content: Option<String>,
    /// `true` when this is a freshly-learned skill installed under "hold for
    /// review" (the `require_review` switch): it is saved but NOT auto-activated
    /// and awaits the user's approval. Approving clears this; discarding removes
    /// the skill. `false` for skills that are live.
    #[serde(default)]
    pub pending_review: bool,
}

impl LearnedSkillProvenance {
    /// Build the baseline for `content` — the full `SKILL.md` text the machine
    /// is writing. Line endings are normalized first so the baseline matches
    /// what `update_skill` persists and what the loader re-hashes.
    pub fn for_machine_content(content: &str) -> Result<Self, SkillParseError> {
        let (body_hash, manifest_hash) = content_fingerprint(content)?;
        Ok(Self {
            last_machine_body_hash: body_hash,
            last_machine_manifest_hash: manifest_hash,
            proposed_content: None,
            pending_review: false,
        })
    }

    /// True iff `live_content` (the on-disk skill) still matches the machine's
    /// last-written baseline — i.e. no human edit since. A parse failure counts
    /// as divergence (fail safe: do not overwrite).
    pub fn matches_live_content(&self, live_content: &str) -> bool {
        match content_fingerprint(live_content) {
            Ok((body_hash, manifest_hash)) => {
                body_hash == self.last_machine_body_hash
                    && manifest_hash == self.last_machine_manifest_hash
            }
            Err(_) => false,
        }
    }

    pub fn to_pretty_json(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec_pretty(self)
    }

    pub fn from_sidecar_bytes(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }
}

/// Compute `(body_hash, activation_snapshot)` for a `SKILL.md` document the same
/// way the registry computes `content_hash` (body-only) plus a sorted snapshot
/// of activation metadata. Shared by baseline-write and divergence-check so the
/// two can never drift.
fn content_fingerprint(content: &str) -> Result<(String, String), SkillParseError> {
    let normalized = normalize_line_endings(content);
    let parsed = parse_skill_md(&normalized)?;
    let body_hash = hash_body(&parsed.prompt_content);
    let manifest_hash = hash_manifest(&parsed.manifest);
    Ok((body_hash, manifest_hash))
}

/// SHA-256 over the full canonicalized manifest. The activation lists are sorted
/// so a pure reorder is not treated as a human edit; every other frontmatter
/// field is hashed as-is, so ANY edit (description, requires, credentials,
/// origin, auto_activate, version, …) — not just the four activation arrays —
/// shows up as divergence. Hashing the serialized manifest rather than
/// enumerating fields keeps coverage complete as new manifest fields are added.
fn hash_manifest(manifest: &SkillManifest) -> String {
    let mut canonical = manifest.clone();
    canonical.activation.keywords.sort();
    canonical.activation.exclude_keywords.sort();
    canonical.activation.patterns.sort();
    canonical.activation.tags.sort();
    // `SkillManifest` is a plain serde struct, so JSON serialization is
    // effectively infallible; fall back to the Debug repr (which also captures
    // every field) rather than silently hashing empty if it ever fails.
    let serialized =
        serde_json::to_vec(&canonical).unwrap_or_else(|_| format!("{canonical:?}").into_bytes());
    let mut hasher = Sha256::new();
    hasher.update(&serialized);
    format!("sha256:{:x}", hasher.finalize())
}

/// SHA-256 of the body, in the same `"sha256:<hex>"` shape as the registry's
/// `compute_hash`. Computed here (not via the `registry`-feature-gated re-export)
/// so this module is feature-independent; consistency is guaranteed because the
/// baseline write and the divergence check both go through this one function.
fn hash_body(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill_md(keywords: &str, body: &str) -> String {
        format!(
            "---\nname: test-skill\ndescription: A test skill\nactivation:\n  keywords: [{keywords}]\n  tags: [alpha]\n---\n\n# Test\n{body}\n"
        )
    }

    #[test]
    fn baseline_matches_identical_content() {
        let content = skill_md("foo, bar", "do the thing");
        let prov = LearnedSkillProvenance::for_machine_content(&content).unwrap();
        assert!(prov.matches_live_content(&content));
    }

    #[test]
    fn body_edit_is_detected_as_divergence() {
        let content = skill_md("foo, bar", "do the thing");
        let prov = LearnedSkillProvenance::for_machine_content(&content).unwrap();
        let edited = skill_md("foo, bar", "do a DIFFERENT thing the human prefers");
        assert!(!prov.matches_live_content(&edited));
    }

    #[test]
    fn frontmatter_keyword_edit_is_detected_even_though_body_unchanged() {
        // The high-value case body-only hashing would miss.
        let content = skill_md("foo, bar", "do the thing");
        let prov = LearnedSkillProvenance::for_machine_content(&content).unwrap();
        let retuned = skill_md("foo, bar, deploy, kubernetes", "do the thing");
        assert!(!prov.matches_live_content(&retuned));
    }

    #[test]
    fn keyword_reorder_is_not_a_divergence() {
        let content = skill_md("foo, bar", "do the thing");
        let prov = LearnedSkillProvenance::for_machine_content(&content).unwrap();
        let reordered = skill_md("bar, foo", "do the thing");
        assert!(prov.matches_live_content(&reordered));
    }

    #[test]
    fn line_ending_difference_is_not_a_divergence() {
        let content = skill_md("foo, bar", "line one\nline two");
        let prov = LearnedSkillProvenance::for_machine_content(&content).unwrap();
        let crlf = content.replace('\n', "\r\n");
        assert!(prov.matches_live_content(&crlf));
    }

    #[test]
    fn sidecar_json_round_trips() {
        let content = skill_md("foo, bar", "do the thing");
        let mut prov = LearnedSkillProvenance::for_machine_content(&content).unwrap();
        prov.proposed_content = Some(skill_md("foo, bar, baz", "an improved version"));
        let bytes = prov.to_pretty_json().unwrap();
        let restored = LearnedSkillProvenance::from_sidecar_bytes(&bytes).unwrap();
        assert_eq!(prov, restored);
    }

    #[test]
    fn unparseable_live_content_is_divergence_fail_safe() {
        // A live file the machine can no longer parse must NOT be silently
        // overwritten — err toward "human owns it".
        let content = skill_md("foo, bar", "do the thing");
        let prov = LearnedSkillProvenance::for_machine_content(&content).unwrap();
        assert!(!prov.matches_live_content("not a valid SKILL.md at all"));
    }

    #[test]
    fn tag_edit_is_detected() {
        // Cover a non-keyword activation field (skill_md hardcodes `tags: [alpha]`).
        let content = skill_md("foo, bar", "do the thing");
        let prov = LearnedSkillProvenance::for_machine_content(&content).unwrap();
        let retagged = content.replace("[alpha]", "[alpha, beta]");
        assert!(!prov.matches_live_content(&retagged));
    }

    #[test]
    fn non_activation_frontmatter_edit_is_detected() {
        // The widened coverage: a human edit to a frontmatter field OUTSIDE the
        // activation arrays (here `description`) must count as divergence so a
        // learned evolution does not silently clobber it.
        let content = skill_md("foo, bar", "do the thing");
        let prov = LearnedSkillProvenance::for_machine_content(&content).unwrap();
        let edited = content.replace("A test skill", "A human-tuned description");
        assert!(!prov.matches_live_content(&edited));
    }
}
