//! Conversational onboarding engine.
//!
//! Drives a "Getting to Know You" conversation using the configured LLM
//! provider. Tracks 6 intents adapted from NPA's onboarding system and
//! generates a [`PsychographicProfile`] from the conversation transcript.

use std::collections::HashSet;
use std::sync::Arc;

use crate::llm::{ChatMessage, CompletionRequest, LlmProvider};
use crate::profile::PsychographicProfile;
use crate::setup::prompts::{input, print_info};
use crate::setup::wizard::SetupError;

/// Onboarding intents — topics the conversation should cover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OnboardingIntent {
    LearnName,
    SupportStyle,
    FriendshipValues,
    SupportExample,
    CommunicationPrefs,
    ReceivingHelp,
}

impl OnboardingIntent {
    const ALL: &'static [Self] = &[
        Self::LearnName,
        Self::SupportStyle,
        Self::FriendshipValues,
        Self::SupportExample,
        Self::CommunicationPrefs,
        Self::ReceivingHelp,
    ];

    #[cfg(test)]
    fn tag(&self) -> &'static str {
        match self {
            Self::LearnName => "learn_name",
            Self::SupportStyle => "support_style",
            Self::FriendshipValues => "friendship_values",
            Self::SupportExample => "support_example",
            Self::CommunicationPrefs => "communication_prefs",
            Self::ReceivingHelp => "receiving_help",
        }
    }

    fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "learn_name" => Some(Self::LearnName),
            "support_style" => Some(Self::SupportStyle),
            "friendship_values" => Some(Self::FriendshipValues),
            "support_example" => Some(Self::SupportExample),
            "communication_prefs" => Some(Self::CommunicationPrefs),
            "receiving_help" => Some(Self::ReceivingHelp),
            _ => None,
        }
    }
}

/// Drives the conversational onboarding flow.
pub struct OnboardingChat {
    llm: Arc<dyn LlmProvider>,
    messages: Vec<ChatMessage>,
    intents_completed: HashSet<OnboardingIntent>,
}

impl OnboardingChat {
    /// Create a new onboarding chat with the given LLM provider.
    pub fn new(llm: Arc<dyn LlmProvider>) -> Self {
        Self {
            llm,
            messages: vec![ChatMessage::system(ONBOARDING_SYSTEM_PROMPT)],
            intents_completed: HashSet::new(),
        }
    }

    /// Run the full onboarding conversation and return the generated profile.
    pub async fn run(&mut self) -> Result<PsychographicProfile, SetupError> {
        println!();
        print_info("Let's have a short conversation so I can personalize your experience.");
        print_info("Type \"skip\" at any time to finish early.\n");

        // Get the initial greeting from the LLM.
        let greeting = self.llm_turn().await?;
        self.extract_completed_intents(&greeting);
        let greeting_display = strip_intent_tags(&greeting);
        println!("{}\n", greeting_display);

        loop {
            // Read user input.
            let user_input = input("You").map_err(SetupError::Io)?;
            let trimmed = user_input.trim();

            if trimmed.eq_ignore_ascii_case("skip") {
                print_info("Skipping remaining questions...");
                break;
            }

            if trimmed.is_empty() {
                continue;
            }

            self.messages.push(ChatMessage::user(trimmed));

            // Get LLM response.
            let response = self.llm_turn().await?;

            // Parse intent completion tags.
            self.extract_completed_intents(&response);

            // Display the response (without intent tags).
            let display = strip_intent_tags(&response);
            println!("\n{}\n", display);

            // Check if all intents are complete.
            if self.all_intents_complete() {
                break;
            }
        }

        // Generate the psychographic profile from the conversation.
        self.generate_profile().await
    }

    /// Send the current message history to the LLM and get a response.
    async fn llm_turn(&mut self) -> Result<String, SetupError> {
        let request = CompletionRequest::new(self.messages.clone())
            .with_temperature(0.7)
            .with_max_tokens(500);

        let response = self
            .llm
            .complete(request)
            .await
            .map_err(|e| SetupError::Config(format!("LLM error during onboarding: {}", e)))?;

        let content = response.content.clone();
        self.messages.push(ChatMessage::assistant(&content));
        Ok(content)
    }

    /// Extract `<intent_complete>` tags from a response.
    fn extract_completed_intents(&mut self, response: &str) {
        // Look for <intent_complete>tag_name</intent_complete>
        let prefix = "<intent_complete>";
        let suffix = "</intent_complete>";

        let mut search_from = 0;
        while let Some(start) = response[search_from..].find(prefix) {
            let abs_start = search_from + start + prefix.len();
            if let Some(end) = response[abs_start..].find(suffix) {
                let tag = response[abs_start..abs_start + end].trim();
                if let Some(intent) = OnboardingIntent::from_tag(tag) {
                    self.intents_completed.insert(intent);
                }
                search_from = abs_start + end + suffix.len();
            } else {
                break;
            }
        }
    }

    /// Check if all onboarding intents have been covered.
    fn all_intents_complete(&self) -> bool {
        OnboardingIntent::ALL
            .iter()
            .all(|i| self.intents_completed.contains(i))
    }

    /// Generate a psychographic profile from the conversation transcript.
    async fn generate_profile(&self) -> Result<PsychographicProfile, SetupError> {
        // Build transcript of user messages only (for analysis).
        let user_messages: Vec<&str> = self
            .messages
            .iter()
            .filter(|m| m.role == crate::llm::Role::User)
            .map(|m| m.content.as_str())
            .collect();

        if user_messages.is_empty() {
            print_info("No conversation data — using default profile.");
            return Ok(PsychographicProfile::default());
        }

        let transcript = user_messages.join("\n\n");
        let prompt = build_profile_generation_prompt(&transcript);

        let messages = vec![ChatMessage::system(prompt)];
        let request = CompletionRequest::new(messages)
            .with_temperature(0.3)
            .with_max_tokens(1500);

        let response = self
            .llm
            .complete(request)
            .await
            .map_err(|e| SetupError::Config(format!("Profile generation failed: {}", e)))?;

        // Try to parse the JSON from the response.
        match parse_profile_json(&response.content) {
            Ok(profile) => Ok(profile),
            Err(first_err) => {
                tracing::debug!("First profile parse failed: {}", first_err);
                print_info("Refining profile analysis...");

                // Retry with a stricter prompt.
                let retry_prompt = format!(
                    "The previous response was not valid JSON. \
                     Please output ONLY a valid JSON object matching the PsychographicProfile schema. \
                     No markdown, no explanation, just the JSON.\n\n\
                     User messages:\n{}\n\n{}",
                    transcript,
                    crate::profile::PROFILE_JSON_SCHEMA
                );

                let retry_messages = vec![ChatMessage::system(retry_prompt)];
                let retry_request = CompletionRequest::new(retry_messages)
                    .with_temperature(0.1)
                    .with_max_tokens(1500);

                let retry_response = self.llm.complete(retry_request).await.map_err(|e| {
                    SetupError::Config(format!("Profile generation retry failed: {}", e))
                })?;

                // NOTE: The default profile returned here is NOT written to
                // workspace by this code path — callers are responsible for
                // persisting. First Contact will still fire on next turn
                // because has_rich_profile checks profile content, not existence.
                parse_profile_json(&retry_response.content).or_else(|e| {
                    tracing::warn!(
                        "Profile generation failed after retry, falling back to default: {}",
                        e
                    );
                    print_info(
                        "Could not generate profile from conversation — using defaults. \
                         Your profile will be built over time through regular conversation.",
                    );
                    Ok(PsychographicProfile::default())
                })
            }
        }
    }
}

/// Extract JSON from a response that may contain markdown code fences.
fn parse_profile_json(text: &str) -> Result<PsychographicProfile, String> {
    let cleaned = text.trim();

    // Try to extract from ```json ... ``` blocks.
    let json_str = if let Some(start) = cleaned.find("```json") {
        let after_fence = &cleaned[start + 7..];
        if let Some(end) = after_fence.find("```") {
            after_fence[..end].trim()
        } else {
            after_fence.trim()
        }
    } else if let Some(start) = cleaned.find("```") {
        let after_fence = &cleaned[start + 3..];
        if let Some(end) = after_fence.find("```") {
            after_fence[..end].trim()
        } else {
            after_fence.trim()
        }
    } else if cleaned.starts_with('{') {
        cleaned
    } else {
        // Try to find the first { and last }
        let start = cleaned.find('{').ok_or("No JSON object found")?;
        let end = cleaned.rfind('}').ok_or("No closing brace found")?;
        &cleaned[start..=end]
    };

    serde_json::from_str(json_str).map_err(|e| format!("JSON parse error: {}", e))
}

/// Strip `<intent_complete>...</intent_complete>` tags from display text.
fn strip_intent_tags(text: &str) -> String {
    let mut result = text.to_string();
    while let Some(start) = result.find("<intent_complete>") {
        if let Some(end) = result[start..].find("</intent_complete>") {
            let remove_end = start + end + "</intent_complete>".len();
            result = format!("{}{}", &result[..start], &result[remove_end..]);
        } else {
            break;
        }
    }
    result.trim().to_string()
}

/// System prompt for the onboarding conversation.
const ONBOARDING_SYSTEM_PROMPT: &str = r#"You are meeting your new user for the first time. Think of yourself as a billionaire's chief of staff — hyper-competent, professional, warm. Like a Slack DM with your closest, most capable colleague. Skip filler phrases ("Great question!", "I'd be happy to help!"). Be direct. Have opinions.

CONVERSATION GOALS:
Cover these 6 topics naturally. After each is adequately covered, output a hidden tag:

1. Learn their preferred name → <intent_complete>learn_name</intent_complete>
2. How they naturally support friends/family → <intent_complete>support_style</intent_complete>
3. What they value most in friendships → <intent_complete>friendship_values</intent_complete>
4. A specific example of supporting someone through a challenge → <intent_complete>support_example</intent_complete>
5. How they prefer to communicate → <intent_complete>communication_prefs</intent_complete>
6. How they prefer to receive help/support → <intent_complete>receiving_help</intent_complete>

ONE-STEP-REMOVED TECHNIQUE:
Ask about how they support friends and family to understand their own values. Instead of "What are your values?" ask "When a friend is going through something tough, what do you usually do?" Instead of "How do you handle conflict?" ask "When two friends come to you with a disagreement, how do you usually help?" This indirect approach reduces defensiveness and yields authentic insights about who the person really is.

QUESTION STYLE:
- Open-ended questions that invite storytelling, not yes/no answers
- Explore feelings and motivations, not just facts
- Connect to daily life and real experiences
- One question at a time — short, conversational, natural
- Use "tell me about..." or "what's it like when..." or "walk me through..." phrasing
- Reference what they've shared to show you're listening

AVOID:
- Yes/no questions or anything that sounds like a survey
- Numbered lists, formal language, academic tone
- Generic questions you'd ask anyone ("What are your hobbies?")
- Asking for files, images, or anything technical
- Trying to solve problems or give advice yet
- Gushing, filler phrases, or performative warmth

Start by introducing yourself briefly and asking what they like to be called. Keep messages short (2-3 sentences max). Match the user's energy and vocabulary.

After all topics are covered, thank them warmly and let them know this will help you communicate better."#;

/// Prompt for generating the psychographic profile from conversation transcript.
fn build_profile_generation_prompt(transcript: &str) -> String {
    let schema = format!(
        "Output a JSON object with this exact structure:\n{}",
        crate::profile::PROFILE_JSON_SCHEMA
    );
    format!(
        r#"Analyze this onboarding conversation and generate a psychographic profile as a JSON object.

{framework}

EVIDENCE-BASED ANALYSIS:
- Only include insights supported by the messages. If the conversation doesn't reveal enough about a dimension, use defaults/unknown.
- For personality trait scores: 40-60 is average range. Only score above 70 or below 30 with strong evidence. Default to 50 if unclear.
- For cohort classification: set confidence 0-100 reflecting how sure you are. Include specific indicators from the conversation.

CONFIDENCE SCORING:
Set the top-level `confidence` field (0.0-1.0) using this formula as a guide:
  confidence = 0.4 + (message_count / 50) * 0.4 + (topic_variety / max(message_count, 1)) * 0.2
Where message_count is the number of user messages and topic_variety is how many distinct topics they covered.

ANALYSIS METADATA:
Set these fields:
- message_count: number of user messages in the transcript
- analysis_method: "onboarding"
- update_type: "initial"
- confidence_score: same as the top-level confidence value

{schema}

User messages from onboarding conversation:
{transcript}

Output ONLY the JSON object, no other text."#,
        framework = crate::profile::ANALYSIS_FRAMEWORK,
        schema = schema,
        transcript = transcript,
    )
}

// JSON schema is now shared via crate::profile::PROFILE_JSON_SCHEMA.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_intent_tags() {
        let input = "That's great! <intent_complete>learn_name</intent_complete> So tell me more.";
        let result = strip_intent_tags(input);
        assert_eq!(result, "That's great!  So tell me more.");
    }

    #[test]
    fn test_strip_multiple_intent_tags() {
        let input = "Hello <intent_complete>learn_name</intent_complete> world <intent_complete>support_style</intent_complete> end";
        let result = strip_intent_tags(input);
        assert_eq!(result, "Hello  world  end");
    }

    #[test]
    fn test_strip_no_tags() {
        let input = "No tags here";
        assert_eq!(strip_intent_tags(input), "No tags here");
    }

    #[test]
    fn test_extract_completed_intents() {
        let mut chat = OnboardingChat {
            llm: Arc::new(MockLlm),
            messages: Vec::new(),
            intents_completed: HashSet::new(),
        };

        chat.extract_completed_intents(
            "Great! <intent_complete>learn_name</intent_complete> Now, about your support style...",
        );
        assert!(
            chat.intents_completed
                .contains(&OnboardingIntent::LearnName)
        );
        assert_eq!(chat.intents_completed.len(), 1);
    }

    #[test]
    fn test_all_intents_complete() {
        let mut chat = OnboardingChat {
            llm: Arc::new(MockLlm),
            messages: Vec::new(),
            intents_completed: HashSet::new(),
        };

        assert!(!chat.all_intents_complete());

        for intent in OnboardingIntent::ALL {
            chat.intents_completed.insert(*intent);
        }
        assert!(chat.all_intents_complete());
    }

    #[test]
    fn test_parse_profile_json_from_code_fence() {
        let input = r#"```json
{
    "version": 1,
    "preferred_name": "Test",
    "personality": {"empathy": 50, "problem_solving": 50, "emotional_intelligence": 50, "adaptability": 50, "communication": 50},
    "communication": {"detail_level": "balanced", "formality": "balanced", "tone": "neutral", "learning_style": "unknown", "social_energy": "unknown", "decision_making": "unknown", "pace": "unknown"},
    "cohort": "other",
    "behavior": {"frictions": [], "desired_outcomes": [], "time_wasters": [], "pain_points": [], "strengths": []},
    "friendship": {"style": "unknown", "values": [], "support_style": "unknown", "qualities": []},
    "assistance": {"proactivity": "medium", "formality": "unknown", "focus_areas": [], "routines": [], "goals": [], "interaction_style": "unknown"},
    "context": {"profession": null, "interests": [], "life_stage": null, "challenges": []},
    "created_at": "2026-01-01T00:00:00Z",
    "updated_at": "2026-01-01T00:00:00Z"
}
```"#;
        let profile = parse_profile_json(input).expect("should parse");
        assert_eq!(profile.preferred_name, "Test");
    }

    #[test]
    fn test_parse_profile_json_raw() {
        let input = r#"{"version":1,"preferred_name":"Raw","personality":{"empathy":50,"problem_solving":50,"emotional_intelligence":50,"adaptability":50,"communication":50},"communication":{"detail_level":"balanced","formality":"balanced","tone":"neutral","learning_style":"unknown","social_energy":"unknown","decision_making":"unknown","pace":"unknown"},"cohort":"other","behavior":{"frictions":[],"desired_outcomes":[],"time_wasters":[],"pain_points":[],"strengths":[]},"friendship":{"style":"unknown","values":[],"support_style":"unknown","qualities":[]},"assistance":{"proactivity":"medium","formality":"unknown","focus_areas":[],"routines":[],"goals":[],"interaction_style":"unknown"},"context":{"profession":null,"interests":[],"life_stage":null,"challenges":[]},"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}"#;
        let profile = parse_profile_json(input).expect("should parse");
        assert_eq!(profile.preferred_name, "Raw");
    }

    #[test]
    fn test_intent_tag_roundtrip() {
        for intent in OnboardingIntent::ALL {
            let tag = intent.tag();
            let parsed = OnboardingIntent::from_tag(tag);
            assert_eq!(parsed, Some(*intent));
        }
    }

    /// Mock LLM provider for tests.
    struct MockLlm;

    #[async_trait::async_trait]
    impl LlmProvider for MockLlm {
        fn model_name(&self) -> &str {
            "mock-model"
        }
        fn cost_per_token(&self) -> (rust_decimal::Decimal, rust_decimal::Decimal) {
            (rust_decimal::Decimal::ZERO, rust_decimal::Decimal::ZERO)
        }
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<crate::llm::CompletionResponse, crate::error::LlmError> {
            Ok(crate::llm::CompletionResponse {
                content: "Hello!".to_string(),
                input_tokens: 0,
                output_tokens: 0,
                finish_reason: crate::llm::FinishReason::Stop,
            })
        }
        async fn complete_with_tools(
            &self,
            _request: crate::llm::ToolCompletionRequest,
        ) -> Result<crate::llm::ToolCompletionResponse, crate::error::LlmError> {
            unimplemented!()
        }
    }
}
