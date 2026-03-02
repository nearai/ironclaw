//! Psychographic profile types for user onboarding.
//!
//! Adapted from NPA's psychographic profiling system. These types capture
//! personality traits, communication preferences, behavioral patterns, and
//! assistance preferences discovered during the "Getting to Know You"
//! onboarding conversation and refined through ongoing interactions.
//!
//! The profile is stored as JSON in `context/profile.json` and rendered
//! as markdown in `USER.md` for system prompt injection.

use serde::{Deserialize, Deserializer, Serialize};

// ---------------------------------------------------------------------------
// 9-dimension analysis framework (shared by onboarding + evolution prompts)
// ---------------------------------------------------------------------------

/// Structured analysis framework used by both onboarding profile generation
/// and weekly profile evolution to guide the LLM in psychographic analysis.
pub const ANALYSIS_FRAMEWORK: &str = r#"Analyze across these 9 dimensions:

1. COMMUNICATION STYLE
   - detail_level: detailed | concise | balanced | unknown
   - formality: casual | balanced | formal | unknown
   - tone: warm | neutral | professional
   - response_speed: quick | thoughtful | depends | unknown
   - learning_style: deep_dive | overview | hands_on | unknown
   - pace: fast | measured | variable | unknown
   Look for: message length, vocabulary complexity, emoji use, sentence structure,
   how quickly they respond, whether they prefer bullet points or prose.

2. PERSONALITY TRAITS (0-100 scale, 50 = average)
   - empathy, problem_solving, emotional_intelligence, adaptability, communication
   Scoring guidance: 40-60 is average. Only score above 70 or below 30 with
   strong evidence from multiple messages. A single empathetic statement is not
   enough for empathy=90.

3. SOCIAL & RELATIONSHIP PATTERNS
   - social_energy: extroverted | introverted | ambivert | unknown
   - friendship.style: few_close | wide_circle | mixed | unknown
   - friendship.support_style: listener | problem_solver | emotional_support | perspective_giver | adaptive | unknown
   - relationship_values: primary values, secondary values, deal_breakers
   Look for: how they talk about others, group vs solo preferences, how they
   describe helping friends/family (the "one step removed" technique).

4. DECISION MAKING & INTERACTION
   - communication.decision_making: intuitive | analytical | balanced | unknown
   - interaction_preferences.proactivity_style: proactive | reactive | collaborative
   - interaction_preferences.feedback_style: direct | gentle | detailed | minimal
   - interaction_preferences.decision_making: autonomous | guided | collaborative
   Look for: do they want options or recommendations? Do they analyze before
   deciding or go with gut feel?

5. BEHAVIORAL PATTERNS
   - frictions: things that frustrate or block them
   - desired_outcomes: what they're trying to achieve
   - time_wasters: activities they want to minimize
   - pain_points: recurring challenges
   - strengths: things they excel at
   - suggested_support: concrete ways the assistant can help
   Look for: complaints, wishes, repeated themes, "I always have to..." patterns.

6. CONTEXTUAL INFO
   - profession, interests, life_stage, challenges
   Only include what is directly stated or strongly implied.

7. ASSISTANCE PREFERENCES
   - proactivity: high | medium | low | unknown
   - formality: formal | casual | professional | unknown
   - interaction_style: direct | conversational | minimal | unknown
   - notification_preferences: frequent | moderate | minimal | unknown
   - focus_areas, routines, goals (arrays of strings)
   Look for: how they frame requests, whether they want hand-holding or autonomy.

8. USER COHORT
   - cohort: busy_professional | new_parent | student | elder | other
   - confidence: 0-100 (how sure you are of this classification)
   - indicators: specific evidence strings supporting the classification
   Only classify with confidence > 30 if there is direct evidence.

9. FRIENDSHIP QUALITIES (deep structure)
   - qualities.user_values: what they value in friendships
   - qualities.friends_appreciate: what friends like about them
   - qualities.consistency_pattern: consistent | adaptive | situational | null
   - qualities.primary_role: their main role in friendships (e.g., "the organizer")
   - qualities.secondary_roles: other roles they play
   - qualities.challenging_aspects: relationship difficulties they mention

GENERAL RULES:
- Be evidence-based: only include insights supported by message content.
- Use "unknown" or empty arrays when there is insufficient evidence.
- Prefer conservative scores over speculative ones.
- Look for patterns across multiple messages, not just individual statements.
"#;

/// JSON schema reference for the psychographic profile.
///
/// Shared by onboarding (onboarding_chat.rs) and first-contact (workspace/mod.rs)
/// prompt generation to ensure the LLM always targets the same structure.
pub const PROFILE_JSON_SCHEMA: &str = r#"{
  "version": 2,
  "preferred_name": "<string>",
  "personality": {
    "empathy": <0-100>,
    "problem_solving": <0-100>,
    "emotional_intelligence": <0-100>,
    "adaptability": <0-100>,
    "communication": <0-100>
  },
  "communication": {
    "detail_level": "<detailed|concise|balanced|unknown>",
    "formality": "<casual|balanced|formal|unknown>",
    "tone": "<warm|neutral|professional>",
    "learning_style": "<deep_dive|overview|hands_on|unknown>",
    "social_energy": "<extroverted|introverted|ambivert|unknown>",
    "decision_making": "<intuitive|analytical|balanced|unknown>",
    "pace": "<fast|measured|variable|unknown>",
    "response_speed": "<quick|thoughtful|depends|unknown>"
  },
  "cohort": {
    "cohort": "<busy_professional|new_parent|student|elder|other>",
    "confidence": <0-100>,
    "indicators": ["<evidence string>"]
  },
  "behavior": {
    "frictions": ["<string>"],
    "desired_outcomes": ["<string>"],
    "time_wasters": ["<string>"],
    "pain_points": ["<string>"],
    "strengths": ["<string>"],
    "suggested_support": ["<string>"]
  },
  "friendship": {
    "style": "<few_close|wide_circle|mixed|unknown>",
    "values": ["<string>"],
    "support_style": "<listener|problem_solver|emotional_support|perspective_giver|adaptive|unknown>",
    "qualities": {
      "user_values": ["<string>"],
      "friends_appreciate": ["<string>"],
      "consistency_pattern": "<consistent|adaptive|situational|null>",
      "primary_role": "<string or null>",
      "secondary_roles": ["<string>"],
      "challenging_aspects": ["<string>"]
    }
  },
  "assistance": {
    "proactivity": "<high|medium|low|unknown>",
    "formality": "<formal|casual|professional|unknown>",
    "focus_areas": ["<string>"],
    "routines": ["<string>"],
    "goals": ["<string>"],
    "interaction_style": "<direct|conversational|minimal|unknown>",
    "notification_preferences": "<minimal|moderate|frequent|unknown>"
  },
  "context": {
    "profession": "<string or null>",
    "interests": ["<string>"],
    "life_stage": "<string or null>",
    "challenges": ["<string>"]
  },
  "relationship_values": {
    "primary": ["<string>"],
    "secondary": ["<string>"],
    "deal_breakers": ["<string>"]
  },
  "interaction_preferences": {
    "proactivity_style": "<proactive|reactive|collaborative>",
    "feedback_style": "<direct|gentle|detailed|minimal>",
    "decision_making": "<autonomous|guided|collaborative>"
  },
  "analysis_metadata": {
    "message_count": <number>,
    "confidence_score": <0.0-1.0>,
    "analysis_method": "<onboarding|evolution>",
    "update_type": "<initial|weekly>"
  },
  "confidence": <0.0-1.0>,
  "created_at": "<ISO-8601>",
  "updated_at": "<ISO-8601>"
}"#;

// ---------------------------------------------------------------------------
// Personality traits
// ---------------------------------------------------------------------------

/// Personality trait scores on a 0-100 scale.
///
/// Values are clamped to 0-100 during deserialization via [`deserialize_trait_score`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersonalityTraits {
    #[serde(deserialize_with = "deserialize_trait_score")]
    pub empathy: u8,
    #[serde(deserialize_with = "deserialize_trait_score")]
    pub problem_solving: u8,
    #[serde(deserialize_with = "deserialize_trait_score")]
    pub emotional_intelligence: u8,
    #[serde(deserialize_with = "deserialize_trait_score")]
    pub adaptability: u8,
    #[serde(deserialize_with = "deserialize_trait_score")]
    pub communication: u8,
}

/// Deserialize a trait score, clamping to the 0-100 range.
///
/// Accepts integer or floating-point JSON numbers. Values outside 0-100
/// are clamped. Non-finite or non-numeric values fall back to a default of 50.
fn deserialize_trait_score<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = f64::deserialize(deserializer).unwrap_or(50.0);
    if !raw.is_finite() {
        return Ok(50);
    }
    let clamped = raw.clamp(0.0, 100.0);
    Ok(clamped.round() as u8)
}

impl Default for PersonalityTraits {
    fn default() -> Self {
        Self {
            empathy: 50,
            problem_solving: 50,
            emotional_intelligence: 50,
            adaptability: 50,
            communication: 50,
        }
    }
}

// ---------------------------------------------------------------------------
// Communication preferences
// ---------------------------------------------------------------------------

/// How the user prefers to communicate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommunicationPreferences {
    /// "detailed" | "concise" | "balanced" | "unknown"
    pub detail_level: String,
    /// "casual" | "balanced" | "formal" | "unknown"
    pub formality: String,
    /// "warm" | "neutral" | "professional"
    pub tone: String,
    /// "deep_dive" | "overview" | "hands_on" | "unknown"
    pub learning_style: String,
    /// "extroverted" | "introverted" | "ambivert" | "unknown"
    pub social_energy: String,
    /// "intuitive" | "analytical" | "balanced" | "unknown"
    pub decision_making: String,
    /// "fast" | "measured" | "variable" | "unknown"
    pub pace: String,
    /// "quick" | "thoughtful" | "depends" | "unknown"
    #[serde(default = "default_unknown")]
    pub response_speed: String,
}

fn default_unknown() -> String {
    "unknown".into()
}

fn default_moderate() -> String {
    "moderate".into()
}

impl Default for CommunicationPreferences {
    fn default() -> Self {
        Self {
            detail_level: "balanced".into(),
            formality: "balanced".into(),
            tone: "neutral".into(),
            learning_style: "unknown".into(),
            social_energy: "unknown".into(),
            decision_making: "unknown".into(),
            pace: "unknown".into(),
            response_speed: "unknown".into(),
        }
    }
}

// ---------------------------------------------------------------------------
// User cohort
// ---------------------------------------------------------------------------

/// User cohort classification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum UserCohort {
    BusyProfessional,
    NewParent,
    Student,
    Elder,
    #[default]
    Other,
}

impl std::fmt::Display for UserCohort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BusyProfessional => write!(f, "busy professional"),
            Self::NewParent => write!(f, "new parent"),
            Self::Student => write!(f, "student"),
            Self::Elder => write!(f, "elder"),
            Self::Other => write!(f, "general"),
        }
    }
}

/// Cohort classification with confidence and evidence.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CohortClassification {
    #[serde(default)]
    pub cohort: UserCohort,
    /// 0-100 confidence in this classification.
    #[serde(default)]
    pub confidence: u8,
    /// Evidence strings supporting the classification.
    #[serde(default)]
    pub indicators: Vec<String>,
}

/// Custom deserializer: accepts either a bare string (old format) or a struct (new format).
fn deserialize_cohort<'de, D>(deserializer: D) -> Result<CohortClassification, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum CohortOrString {
        Classification(CohortClassification),
        BareEnum(UserCohort),
    }

    match CohortOrString::deserialize(deserializer)? {
        CohortOrString::Classification(c) => Ok(c),
        CohortOrString::BareEnum(e) => Ok(CohortClassification {
            cohort: e,
            confidence: 0,
            indicators: Vec::new(),
        }),
    }
}

// ---------------------------------------------------------------------------
// Behavior patterns
// ---------------------------------------------------------------------------

/// Behavioral observations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct BehaviorPatterns {
    pub frictions: Vec<String>,
    pub desired_outcomes: Vec<String>,
    pub time_wasters: Vec<String>,
    pub pain_points: Vec<String>,
    pub strengths: Vec<String>,
    /// Concrete ways the assistant can help.
    #[serde(default)]
    pub suggested_support: Vec<String>,
}

// ---------------------------------------------------------------------------
// Friendship profile
// ---------------------------------------------------------------------------

/// Deep friendship qualities.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct FriendshipQualities {
    #[serde(default)]
    pub user_values: Vec<String>,
    #[serde(default)]
    pub friends_appreciate: Vec<String>,
    /// "consistent" | "adaptive" | "situational" | "unknown"
    #[serde(default)]
    pub consistency_pattern: Option<String>,
    /// Main role in friendships (e.g., "the organizer", "the listener").
    #[serde(default)]
    pub primary_role: Option<String>,
    #[serde(default)]
    pub secondary_roles: Vec<String>,
    #[serde(default)]
    pub challenging_aspects: Vec<String>,
}

/// Custom deserializer: accepts either a `Vec<String>` (old format) or `FriendshipQualities`.
fn deserialize_qualities<'de, D>(deserializer: D) -> Result<FriendshipQualities, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum QualitiesOrVec {
        Struct(FriendshipQualities),
        Vec(Vec<String>),
    }

    match QualitiesOrVec::deserialize(deserializer)? {
        QualitiesOrVec::Struct(q) => Ok(q),
        QualitiesOrVec::Vec(v) => Ok(FriendshipQualities {
            user_values: v,
            ..Default::default()
        }),
    }
}

/// Friendship and support profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FriendshipProfile {
    /// "few_close" | "wide_circle" | "mixed" | "unknown"
    pub style: String,
    pub values: Vec<String>,
    /// "listener" | "problem_solver" | "emotional_support" | "perspective_giver" | "adaptive" | "unknown"
    pub support_style: String,
    /// Deep friendship qualities structure.
    #[serde(default, deserialize_with = "deserialize_qualities")]
    pub qualities: FriendshipQualities,
}

impl Default for FriendshipProfile {
    fn default() -> Self {
        Self {
            style: "unknown".into(),
            values: Vec::new(),
            support_style: "unknown".into(),
            qualities: FriendshipQualities::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Assistance preferences
// ---------------------------------------------------------------------------

/// How the user wants the assistant to behave.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AssistancePreferences {
    /// "high" | "medium" | "low" | "unknown"
    pub proactivity: String,
    /// "formal" | "casual" | "professional" | "unknown"
    pub formality: String,
    pub focus_areas: Vec<String>,
    pub routines: Vec<String>,
    pub goals: Vec<String>,
    /// "direct" | "conversational" | "minimal" | "unknown"
    pub interaction_style: String,
    /// "frequent" | "moderate" | "minimal" | "unknown"
    #[serde(default = "default_moderate")]
    pub notification_preferences: String,
}

impl Default for AssistancePreferences {
    fn default() -> Self {
        Self {
            proactivity: "medium".into(),
            formality: "unknown".into(),
            focus_areas: Vec::new(),
            routines: Vec::new(),
            goals: Vec::new(),
            interaction_style: "unknown".into(),
            notification_preferences: "moderate".into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Contextual info
// ---------------------------------------------------------------------------

/// Contextual information about the user.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ContextualInfo {
    pub profession: Option<String>,
    pub interests: Vec<String>,
    pub life_stage: Option<String>,
    pub challenges: Vec<String>,
}

// ---------------------------------------------------------------------------
// New types: relationship values, interaction preferences, analysis metadata
// ---------------------------------------------------------------------------

/// Core relationship values and deal-breakers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RelationshipValues {
    /// Most important values in relationships.
    #[serde(default)]
    pub primary: Vec<String>,
    /// Additional important values.
    #[serde(default)]
    pub secondary: Vec<String>,
    /// Unacceptable behaviors/traits.
    #[serde(default)]
    pub deal_breakers: Vec<String>,
}

/// How the user prefers to interact with the assistant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InteractionPreferences {
    /// "proactive" | "reactive" | "collaborative"
    pub proactivity_style: String,
    /// "direct" | "gentle" | "detailed" | "minimal"
    pub feedback_style: String,
    /// "autonomous" | "guided" | "collaborative"
    pub decision_making: String,
}

impl Default for InteractionPreferences {
    fn default() -> Self {
        Self {
            proactivity_style: "reactive".into(),
            feedback_style: "direct".into(),
            decision_making: "guided".into(),
        }
    }
}

/// Metadata about the most recent profile analysis.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct AnalysisMetadata {
    /// Number of user messages analyzed.
    #[serde(default)]
    pub message_count: u32,
    /// ISO-8601 timestamp of the analysis.
    #[serde(default)]
    pub analysis_date: Option<String>,
    /// Time range of messages analyzed (e.g., "30 days").
    #[serde(default)]
    pub time_range: Option<String>,
    /// LLM model used for analysis.
    #[serde(default)]
    pub model_used: Option<String>,
    /// Overall confidence score (0.0-1.0).
    #[serde(default)]
    pub confidence_score: f64,
    /// "onboarding" | "evolution" | "passive"
    #[serde(default)]
    pub analysis_method: Option<String>,
    /// "initial" | "weekly" | "event_driven"
    #[serde(default)]
    pub update_type: Option<String>,
}

// ---------------------------------------------------------------------------
// The full psychographic profile
// ---------------------------------------------------------------------------

/// The full psychographic profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PsychographicProfile {
    /// Schema version (1 = original, 2 = enriched with NPA patterns).
    pub version: u32,
    /// What the user likes to be called.
    pub preferred_name: String,
    pub personality: PersonalityTraits,
    pub communication: CommunicationPreferences,
    /// Cohort classification with confidence and evidence.
    #[serde(deserialize_with = "deserialize_cohort")]
    pub cohort: CohortClassification,
    pub behavior: BehaviorPatterns,
    pub friendship: FriendshipProfile,
    pub assistance: AssistancePreferences,
    pub context: ContextualInfo,
    /// Core relationship values.
    #[serde(default)]
    pub relationship_values: RelationshipValues,
    /// How the user prefers to interact with the assistant.
    #[serde(default)]
    pub interaction_preferences: InteractionPreferences,
    /// Metadata about the most recent analysis.
    #[serde(default)]
    pub analysis_metadata: AnalysisMetadata,
    /// Top-level confidence (0.0-1.0), convenience mirror of analysis_metadata.confidence_score.
    #[serde(default)]
    pub confidence: f64,
    /// ISO-8601 creation timestamp.
    pub created_at: String,
    /// ISO-8601 last update timestamp.
    pub updated_at: String,
}

impl Default for PsychographicProfile {
    fn default() -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            version: 2,
            preferred_name: String::new(),
            personality: PersonalityTraits::default(),
            communication: CommunicationPreferences::default(),
            cohort: CohortClassification::default(),
            behavior: BehaviorPatterns::default(),
            friendship: FriendshipProfile::default(),
            assistance: AssistancePreferences::default(),
            context: ContextualInfo::default(),
            relationship_values: RelationshipValues::default(),
            interaction_preferences: InteractionPreferences::default(),
            analysis_metadata: AnalysisMetadata::default(),
            confidence: 0.0,
            created_at: now.clone(),
            updated_at: now,
        }
    }
}

impl PsychographicProfile {
    /// Whether this profile contains meaningful user data beyond defaults.
    ///
    /// Used to decide whether to inject First Contact onboarding instructions
    /// or profile-based personalization into the system prompt.
    pub fn is_populated(&self) -> bool {
        !self.preferred_name.is_empty()
            || self.context.profession.is_some()
            || !self.assistance.goals.is_empty()
    }

    /// Render a concise markdown summary suitable for `USER.md`.
    pub fn to_user_md(&self) -> String {
        let mut sections = Vec::new();

        sections.push("# User Profile\n".to_string());

        if !self.preferred_name.is_empty() {
            sections.push(format!("**Name**: {}\n", self.preferred_name));
        }

        // Communication style
        let mut comm = format!(
            "**Communication**: {} tone, {} detail, {} formality, {} pace",
            self.communication.tone,
            self.communication.detail_level,
            self.communication.formality,
            self.communication.pace,
        );
        if self.communication.response_speed != "unknown" {
            comm.push_str(&format!(
                ", {} response speed",
                self.communication.response_speed
            ));
        }
        sections.push(comm);

        // Decision making
        if self.communication.decision_making != "unknown" {
            sections.push(format!(
                "**Decision style**: {}",
                self.communication.decision_making
            ));
        }

        // Social energy
        if self.communication.social_energy != "unknown" {
            sections.push(format!(
                "**Social energy**: {}",
                self.communication.social_energy
            ));
        }

        // Cohort
        if self.cohort.cohort != UserCohort::Other {
            let mut cohort_line = format!("**User type**: {}", self.cohort.cohort);
            if self.cohort.confidence > 0 {
                cohort_line.push_str(&format!(" ({}% confidence)", self.cohort.confidence));
            }
            sections.push(cohort_line);
        }

        // Profession
        if let Some(ref profession) = self.context.profession {
            sections.push(format!("**Profession**: {}", profession));
        }

        // Life stage
        if let Some(ref stage) = self.context.life_stage {
            sections.push(format!("**Life stage**: {}", stage));
        }

        // Interests
        if !self.context.interests.is_empty() {
            sections.push(format!(
                "**Interests**: {}",
                self.context.interests.join(", ")
            ));
        }

        // Goals
        if !self.assistance.goals.is_empty() {
            sections.push(format!("**Goals**: {}", self.assistance.goals.join(", ")));
        }

        // Focus areas
        if !self.assistance.focus_areas.is_empty() {
            sections.push(format!(
                "**Focus areas**: {}",
                self.assistance.focus_areas.join(", ")
            ));
        }

        // Strengths
        if !self.behavior.strengths.is_empty() {
            sections.push(format!(
                "**Strengths**: {}",
                self.behavior.strengths.join(", ")
            ));
        }

        // Pain points
        if !self.behavior.pain_points.is_empty() {
            sections.push(format!(
                "**Pain points**: {}",
                self.behavior.pain_points.join(", ")
            ));
        }

        // Relationship values
        if !self.relationship_values.primary.is_empty() {
            sections.push(format!(
                "**Core values**: {}",
                self.relationship_values.primary.join(", ")
            ));
        }

        // Assistance preferences
        let mut assist = format!(
            "\n## Assistance Preferences\n\n\
             - **Proactivity**: {}\n\
             - **Interaction style**: {}",
            self.assistance.proactivity, self.assistance.interaction_style,
        );
        if self.assistance.notification_preferences != "moderate" {
            assist.push_str(&format!(
                "\n- **Notifications**: {}",
                self.assistance.notification_preferences
            ));
        }
        sections.push(assist);

        // Interaction preferences
        if self.interaction_preferences.feedback_style != "direct" {
            sections.push(format!(
                "- **Feedback style**: {}",
                self.interaction_preferences.feedback_style
            ));
        }

        // Friendship/support style
        if self.friendship.support_style != "unknown" {
            sections.push(format!(
                "- **Support style**: {}",
                self.friendship.support_style
            ));
        }

        sections.join("\n")
    }

    /// Generate behavioral directives for `context/assistant-directives.md`.
    pub fn to_assistant_directives(&self) -> String {
        let proactivity_instruction = match self.assistance.proactivity.as_str() {
            "high" => "Proactively suggest actions, check in regularly, and anticipate needs.",
            "low" => "Wait for explicit requests. Minimize unsolicited suggestions.",
            _ => "Offer suggestions when relevant but don't overwhelm.",
        };

        let name = if self.preferred_name.is_empty() {
            "the user"
        } else {
            &self.preferred_name
        };

        let mut lines = vec![
            "# Assistant Directives\n".to_string(),
            format!("Based on {}'s profile:\n", name),
            format!(
                "- **Proactivity**: {} -- {}",
                self.assistance.proactivity, proactivity_instruction
            ),
            format!(
                "- **Communication**: {} tone, {} detail level",
                self.communication.tone, self.communication.detail_level
            ),
            format!(
                "- **Decision support**: {} style",
                self.communication.decision_making
            ),
        ];

        if self.communication.response_speed != "unknown" {
            lines.push(format!(
                "- **Response pacing**: {} (match this energy)",
                self.communication.response_speed
            ));
        }

        if self.interaction_preferences.feedback_style != "direct" {
            lines.push(format!(
                "- **Feedback style**: {}",
                self.interaction_preferences.feedback_style
            ));
        }

        if self.assistance.notification_preferences != "moderate"
            && self.assistance.notification_preferences != "unknown"
        {
            lines.push(format!(
                "- **Notification frequency**: {}",
                self.assistance.notification_preferences
            ));
        }

        if !self.assistance.focus_areas.is_empty() {
            lines.push(format!(
                "- **Focus areas**: {}",
                self.assistance.focus_areas.join(", ")
            ));
        }

        if !self.assistance.goals.is_empty() {
            lines.push(format!(
                "- **Goals to support**: {}",
                self.assistance.goals.join(", ")
            ));
        }

        if !self.behavior.pain_points.is_empty() {
            lines.push(format!(
                "- **Pain points to address**: {}",
                self.behavior.pain_points.join(", ")
            ));
        }

        lines.push(String::new());
        lines.push(
            "Start conservative with autonomy — ask before taking actions that affect \
             others or the outside world. Increase autonomy as trust grows."
                .to_string(),
        );

        lines.join("\n")
    }

    /// Generate a personalized `HEARTBEAT.md` checklist.
    pub fn to_heartbeat_md(&self) -> String {
        let name = if self.preferred_name.is_empty() {
            "the user".to_string()
        } else {
            self.preferred_name.clone()
        };

        let mut items = vec![
            format!("- [ ] Check if {} has any pending tasks or reminders", name),
            "- [ ] Review today's schedule and flag conflicts".to_string(),
            "- [ ] Check for messages that need follow-up".to_string(),
        ];

        for area in &self.assistance.focus_areas {
            items.push(format!("- [ ] Check on progress in: {}", area));
        }

        format!(
            "# Heartbeat Checklist\n\n\
             {}\n\n\
             Stay quiet during 23:00-08:00 unless urgent.\n\
             If nothing needs attention, reply HEARTBEAT_OK.",
            items.join("\n")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_profile_serialization_roundtrip() {
        let profile = PsychographicProfile::default();
        let json = serde_json::to_string_pretty(&profile).expect("serialize");
        let deserialized: PsychographicProfile = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(profile.version, deserialized.version);
        assert_eq!(profile.personality, deserialized.personality);
        assert_eq!(profile.communication, deserialized.communication);
        assert_eq!(profile.cohort, deserialized.cohort);
    }

    #[test]
    fn test_user_cohort_display() {
        assert_eq!(
            UserCohort::BusyProfessional.to_string(),
            "busy professional"
        );
        assert_eq!(UserCohort::Student.to_string(), "student");
        assert_eq!(UserCohort::Other.to_string(), "general");
    }

    #[test]
    fn test_to_user_md_includes_name() {
        let profile = PsychographicProfile {
            preferred_name: "Alice".into(),
            ..Default::default()
        };
        let md = profile.to_user_md();
        assert!(md.contains("**Name**: Alice"));
    }

    #[test]
    fn test_to_user_md_includes_goals() {
        let mut profile = PsychographicProfile::default();
        profile.assistance.goals = vec!["time management".into(), "fitness".into()];
        let md = profile.to_user_md();
        assert!(md.contains("time management, fitness"));
    }

    #[test]
    fn test_to_user_md_skips_unknown_fields() {
        let profile = PsychographicProfile::default();
        let md = profile.to_user_md();
        assert!(!md.contains("**User type**"));
        assert!(!md.contains("**Decision style**"));
    }

    #[test]
    fn test_to_assistant_directives_high_proactivity() {
        let mut profile = PsychographicProfile::default();
        profile.assistance.proactivity = "high".into();
        profile.preferred_name = "Bob".into();
        let directives = profile.to_assistant_directives();
        assert!(directives.contains("Proactively suggest actions"));
        assert!(directives.contains("Bob's profile"));
    }

    #[test]
    fn test_to_heartbeat_md_includes_focus_areas() {
        let profile = PsychographicProfile {
            preferred_name: "Carol".into(),
            assistance: AssistancePreferences {
                focus_areas: vec!["project Alpha".into()],
                ..Default::default()
            },
            ..Default::default()
        };
        let heartbeat = profile.to_heartbeat_md();
        assert!(heartbeat.contains("Check if Carol"));
        assert!(heartbeat.contains("project Alpha"));
    }

    #[test]
    fn test_personality_traits_default_is_midpoint() {
        let traits = PersonalityTraits::default();
        assert_eq!(traits.empathy, 50);
        assert_eq!(traits.problem_solving, 50);
    }

    #[test]
    fn test_personality_trait_score_clamped_to_100() {
        // Values > 100 (including > 255) are clamped to 100
        let json = r#"{"empathy":120,"problem_solving":100,"emotional_intelligence":50,"adaptability":300,"communication":0}"#;
        let traits: PersonalityTraits = serde_json::from_str(json).expect("should parse");
        assert_eq!(traits.empathy, 100);
        assert_eq!(traits.problem_solving, 100);
        assert_eq!(traits.emotional_intelligence, 50);
        assert_eq!(traits.adaptability, 100);
        assert_eq!(traits.communication, 0);
    }

    #[test]
    fn test_personality_trait_score_handles_floats_and_negatives() {
        // Floats are rounded, negatives clamped to 0
        let json = r#"{"empathy":75.6,"problem_solving":-10,"emotional_intelligence":50.4,"adaptability":99.5,"communication":0}"#;
        let traits: PersonalityTraits = serde_json::from_str(json).expect("should parse");
        assert_eq!(traits.empathy, 76);
        assert_eq!(traits.problem_solving, 0);
        assert_eq!(traits.emotional_intelligence, 50);
        assert_eq!(traits.adaptability, 100); // 99.5 rounds to 100
        assert_eq!(traits.communication, 0);
    }

    #[test]
    fn test_is_populated_default_is_false() {
        let profile = PsychographicProfile::default();
        assert!(!profile.is_populated());
    }

    #[test]
    fn test_is_populated_with_name() {
        let profile = PsychographicProfile {
            preferred_name: "Alice".into(),
            ..Default::default()
        };
        assert!(profile.is_populated());
    }

    #[test]
    fn test_backward_compat_old_cohort_format() {
        // Old format: cohort is a bare string
        let json = r#"{
            "version": 1,
            "preferred_name": "Test",
            "personality": {"empathy":50,"problem_solving":50,"emotional_intelligence":50,"adaptability":50,"communication":50},
            "communication": {"detail_level":"balanced","formality":"balanced","tone":"neutral","learning_style":"unknown","social_energy":"unknown","decision_making":"unknown","pace":"unknown"},
            "cohort": "busy_professional",
            "behavior": {"frictions":[],"desired_outcomes":[],"time_wasters":[],"pain_points":[],"strengths":[]},
            "friendship": {"style":"unknown","values":[],"support_style":"unknown","qualities":["reliable","loyal"]},
            "assistance": {"proactivity":"medium","formality":"unknown","focus_areas":[],"routines":[],"goals":[],"interaction_style":"unknown"},
            "context": {"profession":null,"interests":[],"life_stage":null,"challenges":[]},
            "created_at": "2026-02-22T00:00:00Z",
            "updated_at": "2026-02-22T00:00:00Z"
        }"#;

        let profile: PsychographicProfile =
            serde_json::from_str(json).expect("should parse old format");
        assert_eq!(profile.cohort.cohort, UserCohort::BusyProfessional);
        assert_eq!(profile.cohort.confidence, 0);
        assert!(profile.cohort.indicators.is_empty());
        // Old qualities Vec<String> should map to user_values
        assert_eq!(
            profile.friendship.qualities.user_values,
            vec!["reliable", "loyal"]
        );
        // New fields should have defaults
        assert_eq!(profile.confidence, 0.0);
        assert!(profile.relationship_values.primary.is_empty());
        assert_eq!(profile.interaction_preferences.feedback_style, "direct");
    }

    #[test]
    fn test_new_format_with_rich_cohort() {
        let json = r#"{
            "version": 2,
            "preferred_name": "Jay",
            "personality": {"empathy":75,"problem_solving":85,"emotional_intelligence":70,"adaptability":80,"communication":72},
            "communication": {"detail_level":"concise","formality":"casual","tone":"warm","learning_style":"hands_on","social_energy":"ambivert","decision_making":"analytical","pace":"fast","response_speed":"quick"},
            "cohort": {"cohort": "busy_professional", "confidence": 85, "indicators": ["mentions deadlines", "talks about team"]},
            "behavior": {"frictions":["context switching"],"desired_outcomes":["more focus time"],"time_wasters":["meetings"],"pain_points":["email overload"],"strengths":["technical depth"],"suggested_support":["automate email triage"]},
            "friendship": {"style":"few_close","values":["authenticity","loyalty"],"support_style":"problem_solver","qualities":{"user_values":["reliability"],"friends_appreciate":["direct advice"],"consistency_pattern":"consistent","primary_role":"the fixer","secondary_roles":["connector"],"challenging_aspects":["impatience"]}},
            "assistance": {"proactivity":"high","formality":"casual","focus_areas":["engineering","health"],"routines":["morning planning"],"goals":["ship product","exercise regularly"],"interaction_style":"direct","notification_preferences":"minimal"},
            "context": {"profession":"software engineer","interests":["AI","fitness","cooking"],"life_stage":"mid-career","challenges":["work-life balance"]},
            "relationship_values": {"primary":["honesty","respect"],"secondary":["humor"],"deal_breakers":["dishonesty"]},
            "interaction_preferences": {"proactivity_style":"proactive","feedback_style":"direct","decision_making":"autonomous"},
            "analysis_metadata": {"message_count":42,"confidence_score":0.85,"analysis_method":"onboarding","update_type":"initial"},
            "confidence": 0.85,
            "created_at": "2026-02-22T00:00:00Z",
            "updated_at": "2026-02-22T00:00:00Z"
        }"#;

        let profile: PsychographicProfile =
            serde_json::from_str(json).expect("should parse new format");
        assert_eq!(profile.preferred_name, "Jay");
        assert_eq!(profile.personality.empathy, 75);
        assert_eq!(profile.cohort.cohort, UserCohort::BusyProfessional);
        assert_eq!(profile.cohort.confidence, 85);
        assert_eq!(profile.communication.response_speed, "quick");
        assert_eq!(profile.assistance.notification_preferences, "minimal");
        assert_eq!(
            profile.behavior.suggested_support,
            vec!["automate email triage"]
        );
        assert_eq!(
            profile.friendship.qualities.primary_role,
            Some("the fixer".into())
        );
        assert_eq!(
            profile.relationship_values.primary,
            vec!["honesty", "respect"]
        );
        assert_eq!(
            profile.interaction_preferences.proactivity_style,
            "proactive"
        );
        assert_eq!(profile.analysis_metadata.message_count, 42);
        assert!((profile.confidence - 0.85).abs() < f64::EPSILON);
    }

    #[test]
    fn test_profile_from_llm_json_old_format() {
        // Original test: old format with bare cohort enum and Vec qualities
        let json = r#"{
            "version": 1,
            "preferred_name": "Jay",
            "personality": {
                "empathy": 75,
                "problem_solving": 85,
                "emotional_intelligence": 70,
                "adaptability": 80,
                "communication": 72
            },
            "communication": {
                "detail_level": "concise",
                "formality": "casual",
                "tone": "warm",
                "learning_style": "hands_on",
                "social_energy": "ambivert",
                "decision_making": "analytical",
                "pace": "fast"
            },
            "cohort": "busy_professional",
            "behavior": {
                "frictions": ["context switching"],
                "desired_outcomes": ["more focus time"],
                "time_wasters": ["meetings"],
                "pain_points": ["email overload"],
                "strengths": ["technical depth"]
            },
            "friendship": {
                "style": "few_close",
                "values": ["authenticity", "loyalty"],
                "support_style": "problem_solver",
                "qualities": ["reliable"]
            },
            "assistance": {
                "proactivity": "high",
                "formality": "casual",
                "focus_areas": ["engineering", "health"],
                "routines": ["morning planning"],
                "goals": ["ship product", "exercise regularly"],
                "interaction_style": "direct"
            },
            "context": {
                "profession": "software engineer",
                "interests": ["AI", "fitness", "cooking"],
                "life_stage": "mid-career",
                "challenges": ["work-life balance"]
            },
            "created_at": "2026-02-22T00:00:00Z",
            "updated_at": "2026-02-22T00:00:00Z"
        }"#;

        let profile: PsychographicProfile =
            serde_json::from_str(json).expect("should parse old LLM output");
        assert_eq!(profile.preferred_name, "Jay");
        assert_eq!(profile.personality.empathy, 75);
        assert_eq!(profile.cohort.cohort, UserCohort::BusyProfessional);
        assert_eq!(profile.assistance.proactivity, "high");
        // New fields get defaults
        assert_eq!(profile.communication.response_speed, "unknown");
        assert_eq!(profile.confidence, 0.0);
    }

    #[test]
    fn test_analysis_framework_contains_all_dimensions() {
        assert!(ANALYSIS_FRAMEWORK.contains("COMMUNICATION STYLE"));
        assert!(ANALYSIS_FRAMEWORK.contains("PERSONALITY TRAITS"));
        assert!(ANALYSIS_FRAMEWORK.contains("SOCIAL & RELATIONSHIP"));
        assert!(ANALYSIS_FRAMEWORK.contains("DECISION MAKING"));
        assert!(ANALYSIS_FRAMEWORK.contains("BEHAVIORAL PATTERNS"));
        assert!(ANALYSIS_FRAMEWORK.contains("CONTEXTUAL INFO"));
        assert!(ANALYSIS_FRAMEWORK.contains("ASSISTANCE PREFERENCES"));
        assert!(ANALYSIS_FRAMEWORK.contains("USER COHORT"));
        assert!(ANALYSIS_FRAMEWORK.contains("FRIENDSHIP QUALITIES"));
    }
}
