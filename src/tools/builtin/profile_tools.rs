//! User profile management tools (view, edit, clear).

use std::sync::Arc;

use async_trait::async_trait;

use crate::context::JobContext;
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_str};
use crate::user_profile::engine::UserProfileEngine;
use crate::user_profile::types::{FactCategory, FactSource, ProfileFact};

/// Tool for viewing the current user profile.
pub struct ProfileViewTool {
    engine: Arc<dyn UserProfileEngine>,
}

impl ProfileViewTool {
    pub fn new(engine: Arc<dyn UserProfileEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait]
impl Tool for ProfileViewTool {
    fn name(&self) -> &str {
        "profile_view"
    }

    fn description(&self) -> &str {
        "View the current user profile — all learned facts about the user \
         including preferences, expertise, style, and context. Use this when \
         the user asks 'what do you know about me?'"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "category": {
                    "type": "string",
                    "enum": ["preference", "expertise", "style", "context"],
                    "description": "Optional: filter by category"
                }
            },
            "required": []
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        // TODO: use ctx.agent_id when multi-agent is supported
        let profile = self
            .engine
            .load_profile(&ctx.user_id, "default")
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to load profile: {e}")))?;

        let category_filter = params
            .get("category")
            .and_then(|v| v.as_str())
            .and_then(FactCategory::from_str_opt);

        let facts: Vec<&ProfileFact> = if let Some(ref cat) = category_filter {
            profile
                .facts
                .iter()
                .filter(|f| &f.category == cat)
                .collect()
        } else {
            profile.facts.iter().collect()
        };

        if facts.is_empty() {
            let msg = if category_filter.is_some() {
                "No profile facts in this category."
            } else {
                "No profile facts stored yet."
            };
            return Ok(ToolOutput::text(msg, start.elapsed()));
        }

        let mut output = format!("{} profile fact(s):\n\n", facts.len());
        for f in &facts {
            output.push_str(&format!(
                "- **{}**/`{}` = {} (confidence: {:.0}%, source: {})\n",
                f.category,
                f.key,
                f.value,
                f.confidence * 100.0,
                f.source.as_str(),
            ));
        }

        Ok(ToolOutput::text(output, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

/// Tool for manually adding or updating a profile fact.
pub struct ProfileEditTool {
    engine: Arc<dyn UserProfileEngine>,
}

impl ProfileEditTool {
    pub fn new(engine: Arc<dyn UserProfileEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait]
impl Tool for ProfileEditTool {
    fn name(&self) -> &str {
        "profile_edit"
    }

    fn description(&self) -> &str {
        "Add or update a user profile fact. Use when the user explicitly \
         states a preference, expertise, or context (e.g., 'I prefer Rust', \
         'my timezone is UTC')."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "category": {
                    "type": "string",
                    "enum": ["preference", "expertise", "style", "context"],
                    "description": "Fact category"
                },
                "key": {
                    "type": "string",
                    "description": "Fact key (alphanumeric + underscore, max 64 chars)"
                },
                "value": {
                    "type": "string",
                    "description": "Fact value (max 512 chars)"
                }
            },
            "required": ["category", "key", "value"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let category_str = require_str(&params, "category")?;
        let key = require_str(&params, "key")?;
        let value = require_str(&params, "value")?;

        let category = FactCategory::from_str_opt(category_str).ok_or_else(|| {
            ToolError::InvalidParameters(format!(
                "Invalid category '{category_str}'. Must be: preference, expertise, style, context"
            ))
        })?;

        // Validate key format
        if key.is_empty()
            || key.len() > 64
            || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(ToolError::InvalidParameters(
                "Key must be 1-64 alphanumeric/underscore characters".to_string(),
            ));
        }

        if value.is_empty() || value.len() > 512 {
            return Err(ToolError::InvalidParameters(
                "Value must be 1-512 characters".to_string(),
            ));
        }

        let fact = ProfileFact {
            category,
            key: key.to_string(),
            value: value.to_string(),
            confidence: 1.0, // explicit user input = max confidence
            source: FactSource::Explicit,
            updated_at: chrono::Utc::now(),
        };

        self.engine
            .store_fact(&ctx.user_id, "default", &fact)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to store fact: {e}")))?;

        // Don't include the value in the output — it may contain sensitive data
        // and ToolOutput is broadcast via SSE/logged.
        Ok(ToolOutput::text(
            format!(
                "Profile updated: {}/{} (value stored encrypted)",
                fact.category, key
            ),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

/// Tool for clearing profile facts.
pub struct ProfileClearTool {
    engine: Arc<dyn UserProfileEngine>,
}

impl ProfileClearTool {
    pub fn new(engine: Arc<dyn UserProfileEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait]
impl Tool for ProfileClearTool {
    fn name(&self) -> &str {
        "profile_clear"
    }

    fn description(&self) -> &str {
        "Remove a specific profile fact (provide key) or all facts in a category \
         (omit key). WARNING: omitting key deletes ALL facts in the category. \
         Use when the user explicitly asks to forget something."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "category": {
                    "type": "string",
                    "enum": ["preference", "expertise", "style", "context"],
                    "description": "Category of the fact to remove"
                },
                "key": {
                    "type": "string",
                    "description": "Specific fact key to remove"
                }
            },
            "required": ["category"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let category_str = require_str(&params, "category")?;
        let key = params.get("key").and_then(|v| v.as_str());

        let category = FactCategory::from_str_opt(category_str).ok_or_else(|| {
            ToolError::InvalidParameters(format!("Invalid category '{category_str}'"))
        })?;

        // If key is provided, remove specific fact; otherwise remove all in category
        if let Some(key) = key {
            let removed = self
                .engine
                .remove_fact(&ctx.user_id, "default", &category, key)
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("Failed to remove fact: {e}")))?;

            let msg = if removed {
                format!("Removed profile fact: {category}/{key}")
            } else {
                format!("No fact found: {category}/{key}")
            };
            Ok(ToolOutput::text(msg, start.elapsed()))
        } else {
            // Remove all facts in category by loading and deleting each
            let profile = self
                .engine
                .load_profile(&ctx.user_id, "default")
                .await
                .map_err(|e| ToolError::ExecutionFailed(format!("Failed to load profile: {e}")))?;

            let keys: Vec<String> = profile
                .facts
                .iter()
                .filter(|f| f.category == category)
                .map(|f| f.key.clone())
                .collect();

            let mut removed_count = 0;
            let mut error_count = 0;
            for k in &keys {
                match self
                    .engine
                    .remove_fact(&ctx.user_id, "default", &category, k)
                    .await
                {
                    Ok(true) => removed_count += 1,
                    Ok(false) => {} // already deleted by another process
                    Err(e) => {
                        tracing::warn!("Profile clear: failed to remove fact '{k}': {e}");
                        error_count += 1;
                    }
                }
            }

            let msg = if error_count > 0 {
                format!(
                    "Removed {removed_count} fact(s) from category '{category}' ({error_count} failed with error)."
                )
            } else {
                format!("Removed {removed_count} fact(s) from category '{category}'.")
            };
            Ok(ToolOutput::text(msg, start.elapsed()))
        }
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}
