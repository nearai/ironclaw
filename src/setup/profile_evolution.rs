//! Profile evolution prompt generation.
//!
//! Generates prompts for weekly re-analysis of the user's psychographic
//! profile based on recent conversation history. Used by the profile
//! evolution routine created during onboarding.

use crate::profile::PsychographicProfile;

/// Generate the LLM prompt for weekly profile evolution.
///
/// Takes the current profile and a summary of recent conversations,
/// and returns a prompt that asks the LLM to output an updated profile.
pub fn profile_evolution_prompt(
    current_profile: &PsychographicProfile,
    recent_messages_summary: &str,
) -> String {
    let profile_json = serde_json::to_string_pretty(current_profile)
        .unwrap_or_else(|_| "{\"error\": \"failed to serialize current profile\"}".to_string());

    format!(
        r#"You are updating a user's psychographic profile based on recent conversations.

CURRENT PROFILE:
```json
{profile_json}
```

RECENT CONVERSATION SUMMARY (last 7 days):
{recent_messages_summary}

{framework}

CONFIDENCE GATING:
- Only update a field when your confidence in the new value exceeds 0.6.
- If evidence is ambiguous or weak, leave the existing value unchanged.
- For personality trait scores: shift gradually (max ±10 per update). Only move above 70 or below 30 with strong evidence.

UPDATE RULES:
1. Compare recent conversations against the current profile across all 9 dimensions.
2. Add new items to arrays (interests, goals, challenges) if discovered.
3. Remove items from arrays only if explicitly contradicted.
4. Update the `updated_at` timestamp to the current ISO-8601 datetime.
5. Do NOT change `version` — it represents the schema version (1=original, 2=enriched), not a revision counter.

ANALYSIS METADATA:
Update these fields:
- message_count: approximate number of user messages in the summary period
- analysis_method: "evolution"
- update_type: "weekly"
- confidence_score: use this formula as a guide:
  confidence = 0.5 + (message_count / 100) * 0.4 + (topic_variety / max(message_count, 1)) * 0.1

LOW CONFIDENCE FLAG:
If the overall confidence_score is below 0.3, add this to the daily log:
"Profile confidence is low — consider a profile refresh conversation."

Output ONLY the updated JSON profile object with the same schema. No explanation, no markdown fences."#,
        framework = crate::profile::ANALYSIS_FRAMEWORK
    )
}

/// The routine prompt template used by the profile evolution cron job.
///
/// This is injected as the routine's action prompt. The agent will:
/// 1. Read `context/profile.json` via `memory_read`
/// 2. Search recent conversations via `memory_search`
/// 3. Call itself with the evolution prompt
/// 4. Write the updated profile back via `memory_write`
pub const PROFILE_EVOLUTION_ROUTINE_PROMPT: &str = r#"You are running a weekly profile evolution check.

Steps:
1. Read the current user profile from `context/profile.json` using the `memory_read` tool.
2. Search for recent conversation themes using `memory_search` with queries like "user preferences", "user goals", "user challenges", "user frustrations".
3. Analyze whether any profile fields should be updated based on what you've learned in the past week.
4. Only update fields where your confidence in the new value exceeds 0.6. Leave ambiguous fields unchanged.
5. If updates are needed, write the updated profile to `context/profile.json` using `memory_write`.
6. Also update `USER.md` with a refreshed markdown summary if the profile changed.
7. Update `analysis_metadata` with message_count, analysis_method="evolution", update_type="weekly", and recalculated confidence_score.
8. If overall confidence_score drops below 0.3, note in the daily log that a profile refresh conversation may help.
9. If no updates are needed, do nothing.

Be conservative — only update fields with clear evidence from recent interactions."#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_evolution_prompt_contains_profile() {
        let profile = PsychographicProfile::default();
        let prompt = profile_evolution_prompt(&profile, "User discussed fitness goals.");
        assert!(prompt.contains("\"version\": 2"));
        assert!(prompt.contains("fitness goals"));
    }

    #[test]
    fn test_profile_evolution_prompt_contains_instructions() {
        let profile = PsychographicProfile::default();
        let prompt = profile_evolution_prompt(&profile, "No notable changes.");
        assert!(prompt.contains("Do NOT change `version`"));
        assert!(prompt.contains("max ±10 per update"));
    }

    #[test]
    fn test_profile_evolution_prompt_includes_framework() {
        let profile = PsychographicProfile::default();
        let prompt = profile_evolution_prompt(&profile, "User likes cooking.");
        assert!(prompt.contains("COMMUNICATION STYLE"));
        assert!(prompt.contains("PERSONALITY TRAITS"));
        assert!(prompt.contains("CONFIDENCE GATING"));
        assert!(prompt.contains("confidence in the new value exceeds 0.6"));
    }

    #[test]
    fn test_routine_prompt_mentions_tools() {
        assert!(PROFILE_EVOLUTION_ROUTINE_PROMPT.contains("memory_read"));
        assert!(PROFILE_EVOLUTION_ROUTINE_PROMPT.contains("memory_write"));
        assert!(PROFILE_EVOLUTION_ROUTINE_PROMPT.contains("memory_search"));
    }
}
