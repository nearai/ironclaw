//! Built-in tools for structured collections.
//!
//! Two categories:
//!
//! **Management tools** (static, one instance each):
//! - `collections_list` — List all registered collections
//! - `collections_register` — Register a new collection schema
//! - `collections_alter` — Alter an existing collection schema (add/remove fields, enum values)
//! - `collections_drop` — Drop a collection and all its records
//!
//! **Per-collection tools** (dynamically generated per schema):
//! - `{collection}_add` — Insert a record with typed fields
//! - `{collection}_update` — Update fields on an existing record
//! - `{collection}_delete` — Delete a record by ID
//! - `{collection}_query` — Query records with filters
//! - `{collection}_summary` — Aggregate records (sum, count, avg, min, max)
//!
//! At startup, all existing collection schemas are loaded and per-collection
//! tools are generated. When `collections_register` is called mid-session,
//! it also registers the new per-collection tools dynamically.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::context::JobContext;
use crate::db::structured::{
    AggOp, Aggregation, AlterOperation, Alteration, CollectionSchema, FieldType, Filter,
};
use crate::db::Database;
use crate::tools::registry::ToolRegistry;
use crate::tools::tool::{Tool, ToolError, ToolOutput, ToolRateLimitConfig, require_str};

// ==================== Schema → JSON Schema conversion ====================

/// Convert a `FieldType` to its JSON Schema representation.
fn field_type_to_json_schema(field_type: &FieldType) -> serde_json::Value {
    match field_type {
        FieldType::Text => json!({ "type": "string" }),
        FieldType::Number => json!({ "type": "number" }),
        FieldType::Date => json!({ "type": "string", "format": "date", "description": "Date in YYYY-MM-DD format" }),
        FieldType::Time => json!({ "type": "string", "description": "Time in HH:MM or HH:MM:SS format" }),
        FieldType::DateTime => json!({ "type": "string", "format": "date-time", "description": "ISO 8601 datetime (e.g. 2026-02-22T08:00:00Z)" }),
        FieldType::Bool => json!({ "type": "boolean" }),
        FieldType::Enum { values } => json!({ "type": "string", "enum": values }),
    }
}

/// Generate tool instances for a collection schema.
pub fn generate_collection_tools(
    schema: &CollectionSchema,
    db: Arc<dyn Database>,
) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(CollectionAddTool::new(schema.clone(), Arc::clone(&db))),
        Arc::new(CollectionUpdateTool::new(schema.clone(), Arc::clone(&db))),
        Arc::new(CollectionDeleteTool::new(schema.clone(), Arc::clone(&db))),
        Arc::new(CollectionQueryTool::new(schema.clone(), Arc::clone(&db))),
        Arc::new(CollectionSummaryTool::new(schema.clone(), db)),
    ]
}

// ==================== Per-Collection Skill Generation ====================

/// Generate and write a per-collection SKILL.md so future conversations can
/// discover the collection's tools via skill activation keywords.
///
/// This is deterministic — no LLM call needed. The schema metadata (collection
/// name, description, field names, enum values) is rich enough to produce good
/// activation keywords. If keyword quality proves insufficient, this can be
/// replaced with an LLM-assisted generation step later without changing the
/// call site.
///
/// Errors are logged but not propagated — skill generation is best-effort and
/// should never block collection registration.
pub(crate) fn generate_collection_skill(schema: &CollectionSchema, skills_dir: &Path) {
    let name = &schema.collection;
    let raw_description = schema
        .description
        .as_deref()
        .unwrap_or("Structured data collection");
    // Sanitize for YAML: strip newlines and escape quotes to prevent injection.
    let description = raw_description
        .replace(['\n', '\r'], " ")
        .replace('"', "\\\"");

    // Build activation keywords from schema metadata
    let mut keywords: Vec<String> = Vec::new();

    // Collection name words (split on underscores)
    for word in name.split('_') {
        if word.len() > 2 {
            keywords.push(word.to_string());
        }
    }
    // Full name with spaces
    keywords.push(name.replace('_', " "));

    // Description words (skip short/common ones)
    let stopwords = [
        "a", "an", "the", "and", "or", "of", "for", "to", "in", "on", "with", "is", "are",
        "was", "were", "be", "been", "being", "has", "have", "had", "do", "does", "did", "this",
        "that", "it", "its", "my", "our", "your",
    ];
    for word in description.split_whitespace() {
        let w = word.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
        if w.len() > 3 && !stopwords.contains(&w.as_str()) && !keywords.contains(&w) {
            keywords.push(w);
        }
    }

    // Field names as keywords (e.g., "task", "priority", "assignee")
    for fname in schema.fields.keys() {
        for word in fname.split('_') {
            let w = word.to_lowercase();
            if w.len() > 2 && !stopwords.contains(&w.as_str()) && !keywords.contains(&w) {
                keywords.push(w);
            }
        }
    }

    // Enum values from fields
    for def in schema.fields.values() {
        if let FieldType::Enum { values } = &def.field_type {
            for v in values {
                let v_lower = v.to_lowercase().replace('_', " ");
                if !keywords.contains(&v_lower) {
                    keywords.push(v_lower);
                }
            }
        }
    }

    // Cap keywords at a reasonable number
    keywords.truncate(25);

    let keywords_yaml: String = keywords
        .iter()
        .map(|k| {
            // Quote keywords containing YAML-special characters.
            if k.contains(':') || k.contains('#') || k.contains('"') {
                let escaped = k.replace('"', "\\\"");
                format!("    - \"{escaped}\"")
            } else {
                format!("    - {k}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Build field documentation for the skill body
    let fields_doc: String = schema
        .fields
        .iter()
        .map(|(fname, fdef)| {
            let type_str = field_type_display(&fdef.field_type);
            let req = if fdef.required { ", required" } else { "" };
            format!("  - `{fname}` ({type_str}{req})")
        })
        .collect::<Vec<_>>()
        .join("\n");

    let human_name = name.replace('_', " ");
    let human_name_title = titlecase(&human_name);

    let skill_content = format!(
        r#"---
name: {name}
version: 0.1.0
description: "{description}"
activation:
  keywords:
{keywords_yaml}
  patterns:
    - "(?i)(needs? to|have to|got to|should|must|going to)"
    - "(?i)\\b(add|put|pick up|include|also need|another)\\b"
    - "(?i)(what('s| do| does| is)|show me|how many|on my (list|plate))"
  max_context_tokens: 800
  tools_prefix: {name}
---

# {human_name_title}

{description}

## Tools

Call these tools to manage records. ALWAYS call the tool — never just acknowledge in text.

- **{name}_add** — Add a record. Fields:
{fields_doc}
- **{name}_query** — Search and filter records (eq, neq, gt, gte, lt, lte, is_null, is_not_null).
- **{name}_summary** — Aggregations: sum, count, avg, min, max. Use group_by for breakdowns.
- **{name}_update** — Update a record by ID (partial update).
- **{name}_delete** — Delete a record by ID.

## Adding records

When the user mentions items to add, ALWAYS call {name}_add immediately:
- "Add X" → one {name}_add call
- "Add X and Y" → two {name}_add calls (one per item)
- "I also need X" / "and Y too" → one {name}_add call per item
- "[Person] needs to [task]" → one {name}_add call (extract person into assignee/person field)
- "I need to [task]" → one {name}_add call (assign to the user)
- ANY mention of something to track → call {name}_add. NEVER just respond in text.

## Querying

- "What's on my list?" / "Show me everything" → {name}_query with no filters
- "Show me just [value]" → {name}_query with filter
- "What does [person] have?" → {name}_query with person/assignee filter
- "How many?" / "Total?" / "Summary by [field]?" → {name}_summary
"#
    );

    let skill_dir = skills_dir.join(name);
    let skill_path = skill_dir.join("SKILL.md");

    if let Err(e) = std::fs::create_dir_all(&skill_dir) {
        tracing::warn!(
            "Failed to create skill directory {}: {e}",
            skill_dir.display()
        );
        return;
    }
    if let Err(e) = std::fs::write(&skill_path, skill_content) {
        tracing::warn!(
            "Failed to write collection skill {}: {e}",
            skill_path.display()
        );
        return;
    }
    tracing::info!(
        "Generated per-collection skill: {}",
        skill_path.display()
    );
}

/// Human-readable display for a field type.
fn field_type_display(ft: &FieldType) -> String {
    match ft {
        FieldType::Text => "text".to_string(),
        FieldType::Number => "number".to_string(),
        FieldType::Date => "date".to_string(),
        FieldType::Time => "time".to_string(),
        FieldType::DateTime => "datetime".to_string(),
        FieldType::Bool => "bool".to_string(),
        FieldType::Enum { values } => format!("enum: {}", values.join(", ")),
    }
}

/// Simple titlecase: capitalize first letter of each word.
fn titlecase(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().to_string() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Generate (or update) the collections-router SKILL.md.
///
/// The router skill matches broadly on any collection name keyword so the
/// agent discovers structured collections even when no per-collection skill
/// is active. It points the LLM at `collections_list` for schema discovery
/// rather than injecting full schemas into the prompt.
///
/// If `schemas` is empty the router file is removed.
pub(crate) fn generate_router_skill(schemas: &[CollectionSchema], skills_dir: &Path) {
    let router_dir = skills_dir.join("collections-router");
    let router_path = router_dir.join("SKILL.md");

    if schemas.is_empty() {
        // Nothing to route — clean up
        let _ = std::fs::remove_file(&router_path);
        let _ = std::fs::remove_dir(&router_dir);
        return;
    }

    // Build keywords from all collection name words
    let mut keywords: Vec<String> = Vec::new();
    for schema in schemas {
        for word in schema.collection.split('_') {
            if word.len() > 2 {
                let w = word.to_lowercase();
                if !keywords.contains(&w) {
                    keywords.push(w);
                }
            }
        }
        // Full name with spaces
        let human = schema.collection.replace('_', " ");
        if !keywords.contains(&human) {
            keywords.push(human);
        }
    }
    keywords.truncate(20);

    let keywords_yaml: String = keywords
        .iter()
        .map(|k| {
            // Quote keywords containing YAML-special characters.
            if k.contains(':') || k.contains('#') || k.contains('"') {
                let escaped = k.replace('"', "\\\"");
                format!("    - \"{escaped}\"")
            } else {
                format!("    - {k}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    let collection_list: String = schemas
        .iter()
        .map(|s| {
            let desc = s.description.as_deref().unwrap_or("structured data");
            format!("- **{}**: {}", s.collection, desc)
        })
        .collect::<Vec<_>>()
        .join("\n");

    let content = format!(
        r#"---
name: collections-router
version: 0.1.0
description: Routes to structured data collections
activation:
  keywords:
{keywords_yaml}
  max_context_tokens: 400
---

# Structured Collections

You have {count} collection(s):
{collection_list}

Call `collections_list` to discover schemas and available per-collection tools.
"#,
        count = schemas.len(),
    );

    if let Err(e) = std::fs::create_dir_all(&router_dir) {
        tracing::warn!(
            "Failed to create router skill directory {}: {e}",
            router_dir.display()
        );
        return;
    }
    if let Err(e) = std::fs::write(&router_path, content) {
        tracing::warn!(
            "Failed to write router skill {}: {e}",
            router_path.display()
        );
        return;
    }
    tracing::info!(
        "Generated collections-router skill for {} collections",
        schemas.len()
    );
}

// ==================== Shared Tool Refresh ====================

/// Unregister old per-collection tools, generate new ones from the schema,
/// register them, and regenerate skills. Used by both register and alter tools.
async fn refresh_collection_tools(
    schema: &CollectionSchema,
    db: &Arc<dyn Database>,
    registry: &Arc<ToolRegistry>,
    skills_dir: Option<&Path>,
    skill_registry: Option<&Arc<std::sync::RwLock<crate::skills::SkillRegistry>>>,
    user_id: &str,
) -> Vec<String> {
    // Unregister old per-collection tools (if they exist)
    let suffixes = ["_add", "_update", "_delete", "_query", "_summary"];
    for suffix in &suffixes {
        let tool_name = format!("{}{suffix}", schema.collection);
        registry.unregister(&tool_name).await;
    }

    // Generate and register new per-collection tools
    let tools = generate_collection_tools(schema, Arc::clone(db));
    let tool_names: Vec<String> = tools.iter().map(|t| t.name().to_string()).collect();
    for tool in tools {
        registry.register(tool).await;
    }

    // Generate per-collection skill for future session discovery (best-effort)
    if let Some(skills_dir) = skills_dir {
        generate_collection_skill(schema, skills_dir);

        if let Some(sr) = skill_registry {
            // Load the per-collection skill into the registry
            let skill_path = skills_dir.join(&schema.collection).join("SKILL.md");
            match crate::skills::load_and_validate_skill(
                &skill_path,
                crate::skills::SkillTrust::Trusted,
                crate::skills::SkillSource::User(skills_dir.join(&schema.collection)),
            )
            .await
            {
                Ok((name, skill)) => {
                    if let Ok(mut reg) = sr.write() {
                        let _ = reg.commit_remove(&name);
                        if let Err(e) = reg.commit_install(&name, skill) {
                            tracing::warn!("Failed to install per-collection skill: {e}");
                        } else {
                            tracing::info!("Loaded per-collection skill into registry: {name}");
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to load per-collection skill: {e}");
                }
            }

            // Update the router skill
            if let Ok(schemas) = db.list_collections(user_id).await {
                generate_router_skill(&schemas, skills_dir);
                let router_path = skills_dir.join("collections-router").join("SKILL.md");
                if router_path.exists() {
                    match crate::skills::load_and_validate_skill(
                        &router_path,
                        crate::skills::SkillTrust::Trusted,
                        crate::skills::SkillSource::User(skills_dir.join("collections-router")),
                    )
                    .await
                    {
                        Ok((rname, rskill)) => {
                            if let Ok(mut reg) = sr.write() {
                                let _ = reg.commit_remove(&rname);
                                let _ = reg.commit_install(&rname, rskill);
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to load router skill: {e}");
                        }
                    }
                }
            }
        }
    }

    tool_names
}

// ==================== Management Tools ====================

/// Tool to list all registered structured collections.
pub struct CollectionListTool {
    db: Arc<dyn Database>,
}

impl CollectionListTool {
    pub fn new(db: Arc<dyn Database>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl Tool for CollectionListTool {
    fn name(&self) -> &str {
        "collections_list"
    }

    fn description(&self) -> &str {
        "List all registered structured data collections. Returns collection names, \
         descriptions, and field definitions. Use this to discover what structured \
         data is available before querying."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
        })
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let schemas = self
            .db
            .list_collections(&ctx.user_id)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to list collections: {e}")))?;

        let collections: Vec<serde_json::Value> = schemas
            .iter()
            .map(|s| {
                let fields: serde_json::Value = s
                    .fields
                    .iter()
                    .map(|(name, def)| {
                        (
                            name.clone(),
                            json!({
                                "type": field_type_to_json_schema(&def.field_type),
                                "required": def.required,
                            }),
                        )
                    })
                    .collect::<serde_json::Map<String, serde_json::Value>>()
                    .into();
                json!({
                    "collection": s.collection,
                    "description": s.description,
                    "fields": fields,
                })
            })
            .collect();

        Ok(ToolOutput::success(
            json!({
                "collections": collections,
                "count": collections.len(),
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

/// Tool to register a new structured collection.
///
/// When called, this also dynamically registers per-collection tools
/// so the LLM can immediately start using them, and generates a
/// per-collection skill file for future session discovery.
pub struct CollectionRegisterTool {
    db: Arc<dyn Database>,
    registry: Arc<ToolRegistry>,
    skills_dir: Option<PathBuf>,
    skill_registry: Option<Arc<std::sync::RwLock<crate::skills::SkillRegistry>>>,
}

impl CollectionRegisterTool {
    pub fn new(db: Arc<dyn Database>, registry: Arc<ToolRegistry>) -> Self {
        Self {
            db,
            registry,
            skills_dir: None,
            skill_registry: None,
        }
    }

    pub fn with_skills_dir(mut self, dir: PathBuf) -> Self {
        self.skills_dir = Some(dir);
        self
    }

    pub fn with_skill_registry(
        mut self,
        sr: Arc<std::sync::RwLock<crate::skills::SkillRegistry>>,
    ) -> Self {
        self.skill_registry = Some(sr);
        self
    }
}

#[async_trait]
impl Tool for CollectionRegisterTool {
    fn name(&self) -> &str {
        "collections_register"
    }

    fn description(&self) -> &str {
        "Register a new structured data collection with a typed schema. \
         After registration, dedicated tools for adding, updating, deleting, \
         querying, and summarizing records become available immediately. \
         USE THIS for: todo lists, task boards, grocery lists, shopping lists, \
         inventories, schedules, logs, trackers, budgets, meal plans, chore charts, \
         contact lists, or ANY list/table the user wants to manage. \
         If the user says 'keep track of', 'set up a list', 'organize my', \
         'manage my', or 'I need a [something] list' — this is the right tool. \
         Design a RICH schema: ALWAYS include a required text field for the \
         main content (e.g. 'task', 'item', 'title', 'name' — whatever fits). \
         Add fields the user will want to filter or group by \
         (e.g. category, status, priority, assignee, date, due_date). \
         Use enum types for fields with a known set of values. \
         Mark key fields as required."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "collection": {
                    "type": "string",
                    "description": "Name for the collection (alphanumeric + underscores, e.g. 'nanny_shifts', 'grocery_items')"
                },
                "description": {
                    "type": "string",
                    "description": "Human-readable description of what this collection tracks"
                },
                "fields": {
                    "type": "object",
                    "description": "Field definitions. Each key is a field name, value is an object with 'type' (text/number/date/time/datetime/bool/enum), optional 'required' (boolean), optional 'default', and for enum type: 'values' (array of allowed strings).",
                    "additionalProperties": {
                        "type": "object",
                        "properties": {
                            "type": {
                                "type": "string",
                                "enum": ["text", "number", "date", "time", "datetime", "bool", "enum"]
                            },
                            "required": { "type": "boolean" },
                            "default": {},
                            "values": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Allowed values (only for enum type)"
                            }
                        },
                        "required": ["type"]
                    }
                }
            },
            "required": ["collection", "fields"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        // Parse the schema from parameters
        let schema: CollectionSchema = serde_json::from_value(params.clone()).map_err(|e| {
            ToolError::InvalidParameters(format!("Invalid collection schema: {e}"))
        })?;

        // Validate name
        CollectionSchema::validate_name(&schema.collection).map_err(|e| {
            ToolError::InvalidParameters(format!("Invalid collection name: {e}"))
        })?;

        // Validate field count limits
        if schema.fields.len() > 50 {
            return Err(ToolError::InvalidParameters(
                "Schema exceeds maximum of 50 fields".to_string(),
            ));
        }
        for (name, def) in &schema.fields {
            if let FieldType::Enum { values } = &def.field_type && values.len() > 100 {
                return Err(ToolError::InvalidParameters(format!(
                    "Enum field '{name}' exceeds maximum of 100 values"
                )));
            }
        }

        // Validate default values match their declared types
        schema.validate_defaults().map_err(|e| {
            ToolError::InvalidParameters(format!("Invalid default value: {e}"))
        })?;

        // Register in database
        self.db
            .register_collection(&ctx.user_id, &schema)
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to register collection: {e}"))
            })?;

        // Generate and register per-collection tools + skills
        let tool_names = refresh_collection_tools(
            &schema,
            &self.db,
            &self.registry,
            self.skills_dir.as_deref(),
            self.skill_registry.as_ref(),
            &ctx.user_id,
        )
        .await;

        Ok(ToolOutput::success(
            json!({
                "status": "registered",
                "collection": schema.collection,
                "tools_created": tool_names,
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(5, 50))
    }
}

/// Tool to drop a structured collection and all its records.
pub struct CollectionDropTool {
    db: Arc<dyn Database>,
    registry: Arc<ToolRegistry>,
    skills_dir: Option<PathBuf>,
    skill_registry: Option<Arc<std::sync::RwLock<crate::skills::SkillRegistry>>>,
}

impl CollectionDropTool {
    pub fn new(db: Arc<dyn Database>, registry: Arc<ToolRegistry>) -> Self {
        Self {
            db,
            registry,
            skills_dir: None,
            skill_registry: None,
        }
    }

    pub fn with_skills_dir(mut self, dir: PathBuf) -> Self {
        self.skills_dir = Some(dir);
        self
    }

    pub fn with_skill_registry(
        mut self,
        sr: Arc<std::sync::RwLock<crate::skills::SkillRegistry>>,
    ) -> Self {
        self.skill_registry = Some(sr);
        self
    }
}

#[async_trait]
impl Tool for CollectionDropTool {
    fn name(&self) -> &str {
        "collections_drop"
    }

    fn description(&self) -> &str {
        "Drop a structured data collection and permanently delete all its records. \
         This action cannot be undone. The associated tools for this collection \
         will also be removed."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "collection": {
                    "type": "string",
                    "description": "Name of the collection to drop"
                }
            },
            "required": ["collection"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let collection = require_str(&params, "collection")?;

        // Drop from database (cascades to records). Returns NotFound if
        // the collection doesn't exist — no need for a separate existence check.
        self.db
            .drop_collection(&ctx.user_id, collection)
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to drop collection: {e}"))
            })?;

        // Unregister per-collection tools
        let tool_suffixes = ["_add", "_update", "_delete", "_query", "_summary"];
        let mut removed = Vec::new();
        for suffix in &tool_suffixes {
            let tool_name = format!("{collection}{suffix}");
            if self.registry.unregister(&tool_name).await.is_some() {
                removed.push(tool_name);
            }
        }

        // Clean up per-collection skill (best-effort)
        if let Some(ref skills_dir) = self.skills_dir {
            let skill_dir = skills_dir.join(collection);
            let skill_path = skill_dir.join("SKILL.md");
            let _ = std::fs::remove_file(&skill_path);
            let _ = std::fs::remove_dir(&skill_dir);

            // Remove from in-memory registry
            if let Some(ref sr) = self.skill_registry
                && let Ok(mut reg) = sr.write()
            {
                let _ = reg.commit_remove(collection);
            }

            // Update the router skill
            if let Some(ref sr) = self.skill_registry
                && let Ok(schemas) = self.db.list_collections(&ctx.user_id).await
            {
                generate_router_skill(&schemas, skills_dir);
                let router_path = skills_dir.join("collections-router").join("SKILL.md");
                if router_path.exists() {
                    match crate::skills::load_and_validate_skill(
                        &router_path,
                        crate::skills::SkillTrust::Trusted,
                        crate::skills::SkillSource::User(
                            skills_dir.join("collections-router"),
                        ),
                    )
                    .await
                    {
                        Ok((rname, rskill)) => {
                            if let Ok(mut reg) = sr.write() {
                                let _ = reg.commit_remove(&rname);
                                let _ = reg.commit_install(&rname, rskill);
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to reload router skill: {e}");
                        }
                    }
                } else {
                    // No router needed (all collections dropped)
                    if let Ok(mut reg) = sr.write() {
                        let _ = reg.commit_remove("collections-router");
                    }
                }
            }
        }

        Ok(ToolOutput::success(
            json!({
                "status": "dropped",
                "collection": collection,
                "tools_removed": removed,
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(5, 50))
    }
}

// ==================== Alter Tool ====================

/// Tool to alter the schema of an existing collection.
///
/// Supports targeted mutations: add/remove fields, add/remove enum values.
/// Existing records are preserved — data is not migrated or deleted.
pub struct CollectionsAlterTool {
    db: Arc<dyn Database>,
    registry: Arc<ToolRegistry>,
    skills_dir: Option<PathBuf>,
    skill_registry: Option<Arc<std::sync::RwLock<crate::skills::SkillRegistry>>>,
}

impl CollectionsAlterTool {
    pub fn new(db: Arc<dyn Database>, registry: Arc<ToolRegistry>) -> Self {
        Self {
            db,
            registry,
            skills_dir: None,
            skill_registry: None,
        }
    }

    pub fn with_skills_dir(mut self, dir: PathBuf) -> Self {
        self.skills_dir = Some(dir);
        self
    }

    pub fn with_skill_registry(
        mut self,
        sr: Arc<std::sync::RwLock<crate::skills::SkillRegistry>>,
    ) -> Self {
        self.skill_registry = Some(sr);
        self
    }
}

#[async_trait]
impl Tool for CollectionsAlterTool {
    fn name(&self) -> &str {
        "collections_alter"
    }

    fn description(&self) -> &str {
        "Alter the schema of an existing collection. Add or remove fields, \
         add or remove enum values. Existing records are preserved — use this \
         instead of dropping and re-creating a collection."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "collection": {
                    "type": "string",
                    "description": "Name of the collection to alter"
                },
                "operation": {
                    "type": "string",
                    "enum": ["add_field", "remove_field", "add_enum_value", "remove_enum_value"],
                    "description": "The alteration operation to perform"
                },
                "field": {
                    "type": "string",
                    "description": "Field name to add, remove, or modify"
                },
                "field_type": {
                    "type": "string",
                    "enum": ["text", "number", "date", "time", "datetime", "bool", "enum"],
                    "description": "Type for the new field (add_field only)"
                },
                "required": {
                    "type": "boolean",
                    "description": "Whether the field is required (add_field only, default: false)"
                },
                "default": {
                    "description": "Default value for the field (add_field only)"
                },
                "values": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Enum values (add_field with enum type only)"
                },
                "value": {
                    "type": "string",
                    "description": "Single enum value to add or remove (add_enum_value / remove_enum_value)"
                }
            },
            "required": ["collection", "operation", "field"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let collection = require_str(&params, "collection")?;
        let op_str = require_str(&params, "operation")?;
        let field = require_str(&params, "field")?.to_string();

        let operation: AlterOperation = serde_json::from_value(json!(op_str)).map_err(|e| {
            ToolError::InvalidParameters(format!(
                "Invalid operation '{op_str}': {e}. Must be one of: add_field, remove_field, add_enum_value, remove_enum_value"
            ))
        })?;

        // Parse field_type for add_field
        let field_type = if let Some(ft_str) = params.get("field_type").and_then(|v| v.as_str()) {
            let ft = match ft_str {
                "text" => FieldType::Text,
                "number" => FieldType::Number,
                "date" => FieldType::Date,
                "time" => FieldType::Time,
                "datetime" => FieldType::DateTime,
                "bool" => FieldType::Bool,
                "enum" => {
                    let values: Vec<String> = params
                        .get("values")
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                        .unwrap_or_default();
                    FieldType::Enum { values }
                }
                other => {
                    return Err(ToolError::InvalidParameters(format!(
                        "Unknown field_type: {other}"
                    )));
                }
            };
            Some(ft)
        } else {
            None
        };

        let alteration = Alteration {
            operation: operation.clone(),
            field: field.clone(),
            field_type,
            required: params.get("required").and_then(|v| v.as_bool()),
            default: params.get("default").cloned(),
            value: params.get("value").and_then(|v| v.as_str()).map(String::from),
        };

        // Fetch current schema
        let current = self
            .db
            .get_collection_schema(&ctx.user_id, collection)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Collection not found: {e}")))?;

        // Apply mutation
        let new_schema = current.apply_alteration(&alteration).map_err(|e| {
            ToolError::InvalidParameters(format!("Invalid alteration: {e}"))
        })?;

        // Persist updated schema
        self.db
            .register_collection(&ctx.user_id, &new_schema)
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to update collection schema: {e}"))
            })?;

        // Regenerate tools + skills
        let tool_names = refresh_collection_tools(
            &new_schema,
            &self.db,
            &self.registry,
            self.skills_dir.as_deref(),
            self.skill_registry.as_ref(),
            &ctx.user_id,
        )
        .await;

        // Build human-readable description of the change
        let description = match alteration.operation {
            AlterOperation::AddField => format!("Added field '{field}' to {collection}"),
            AlterOperation::RemoveField => format!("Removed field '{field}' from {collection}"),
            AlterOperation::AddEnumValue => {
                let v = alteration.value.as_deref().unwrap_or("?");
                format!("Added enum value '{v}' to field '{field}' in {collection}")
            }
            AlterOperation::RemoveEnumValue => {
                let v = alteration.value.as_deref().unwrap_or("?");
                format!("Removed enum value '{v}' from field '{field}' in {collection}")
            }
        };

        Ok(ToolOutput::success(
            json!({
                "status": "altered",
                "collection": collection,
                "description": description,
                "tools_refreshed": tool_names,
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(5, 50))
    }
}

// ==================== Per-Collection Tools ====================

/// Tool for adding a record to a specific collection.
///
/// Parameters are derived from the collection's schema so the LLM
/// sees typed fields (not a generic "data" blob).
pub struct CollectionAddTool {
    tool_name: String,
    schema: CollectionSchema,
    db: Arc<dyn Database>,
}

impl CollectionAddTool {
    pub fn new(schema: CollectionSchema, db: Arc<dyn Database>) -> Self {
        let tool_name = format!("{}_add", schema.collection);
        Self {
            tool_name,
            schema,
            db,
        }
    }
}

#[async_trait]
impl Tool for CollectionAddTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        // Can't return dynamic string from &str, so use a static prefix.
        // The tool name already encodes the collection.
        "Add a new record to this collection. \
         Call this when the user wants to track, remember, save, create, or log something new. \
         Example triggers: 'I need to...', 'Add...', 'Don't forget...', 'Put X on my list', \
         '[person] needs to...'. Fields are validated against the schema."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();

        for (field_name, field_def) in &self.schema.fields {
            let mut prop = field_type_to_json_schema(&field_def.field_type);
            if let Some(ref default) = field_def.default
                && let Some(obj) = prop.as_object_mut()
            {
                obj.insert("default".to_string(), default.clone());
            }
            properties.insert(field_name.clone(), prop);
            if field_def.required {
                required.push(json!(field_name));
            }
        }

        json!({
            "type": "object",
            "properties": properties,
            "required": required,
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let id = self
            .db
            .insert_record(&ctx.user_id, &self.schema.collection, params)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to insert record: {e}")))?;

        Ok(ToolOutput::success(
            json!({
                "status": "created",
                "record_id": id.to_string(),
                "collection": self.schema.collection,
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(20, 200))
    }
}

/// Tool for updating a record in a specific collection.
pub struct CollectionUpdateTool {
    tool_name: String,
    schema: CollectionSchema,
    db: Arc<dyn Database>,
}

impl CollectionUpdateTool {
    pub fn new(schema: CollectionSchema, db: Arc<dyn Database>) -> Self {
        let tool_name = format!("{}_update", schema.collection);
        Self {
            tool_name,
            schema,
            db,
        }
    }
}

#[async_trait]
impl Tool for CollectionUpdateTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        "Update an existing record. Provide the record_id and only the fields you want to change."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let mut properties = serde_json::Map::new();

        // record_id is always required
        properties.insert(
            "record_id".to_string(),
            json!({
                "type": "string",
                "description": "The ID of the record to update"
            }),
        );

        // All collection fields are optional for updates
        for (field_name, field_def) in &self.schema.fields {
            properties.insert(field_name.clone(), field_type_to_json_schema(&field_def.field_type));
        }

        json!({
            "type": "object",
            "properties": properties,
            "required": ["record_id"],
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let record_id_str = require_str(&params, "record_id")?;
        let record_id = uuid::Uuid::parse_str(record_id_str).map_err(|e| {
            ToolError::InvalidParameters(format!("Invalid record_id: {e}"))
        })?;

        // Extract only the collection fields (not record_id) for the update
        let mut updates = serde_json::Map::new();
        if let Some(obj) = params.as_object() {
            for (key, value) in obj {
                if key != "record_id" {
                    updates.insert(key.clone(), value.clone());
                }
            }
        }

        if updates.is_empty() {
            return Err(ToolError::InvalidParameters(
                "No fields to update provided".to_string(),
            ));
        }

        self.db
            .update_record(
                &ctx.user_id,
                record_id,
                serde_json::Value::Object(updates),
            )
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to update record: {e}")))?;

        Ok(ToolOutput::success(
            json!({
                "status": "updated",
                "record_id": record_id_str,
                "collection": self.schema.collection,
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(20, 200))
    }
}

/// Tool for deleting a record from a specific collection.
pub struct CollectionDeleteTool {
    tool_name: String,
    collection_name: String,
    db: Arc<dyn Database>,
}

impl CollectionDeleteTool {
    pub fn new(schema: CollectionSchema, db: Arc<dyn Database>) -> Self {
        let tool_name = format!("{}_delete", schema.collection);
        Self {
            tool_name,
            collection_name: schema.collection,
            db,
        }
    }
}

#[async_trait]
impl Tool for CollectionDeleteTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        "Delete a record by its ID. This action cannot be undone."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "record_id": {
                    "type": "string",
                    "description": "The ID of the record to delete"
                }
            },
            "required": ["record_id"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let record_id_str = require_str(&params, "record_id")?;
        let record_id = uuid::Uuid::parse_str(record_id_str).map_err(|e| {
            ToolError::InvalidParameters(format!("Invalid record_id: {e}"))
        })?;

        self.db
            .delete_record(&ctx.user_id, record_id)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to delete record: {e}")))?;

        Ok(ToolOutput::success(
            json!({
                "status": "deleted",
                "record_id": record_id_str,
                "collection": self.collection_name,
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(20, 200))
    }
}

/// Tool for querying records from a specific collection.
pub struct CollectionQueryTool {
    tool_name: String,
    schema: CollectionSchema,
    db: Arc<dyn Database>,
}

impl CollectionQueryTool {
    pub fn new(schema: CollectionSchema, db: Arc<dyn Database>) -> Self {
        let tool_name = format!("{}_query", schema.collection);
        Self {
            tool_name,
            schema,
            db,
        }
    }
}

#[async_trait]
impl Tool for CollectionQueryTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        "Query records with optional filters, ordering, and limit. \
         Returns matching records sorted by the specified field or by creation date."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let field_names: Vec<&str> = self.schema.fields.keys().map(|s| s.as_str()).collect();

        json!({
            "type": "object",
            "properties": {
                "filters": {
                    "type": "array",
                    "description": "Optional filters to apply",
                    "items": {
                        "type": "object",
                        "properties": {
                            "field": {
                                "type": "string",
                                "enum": field_names,
                                "description": "Field to filter on"
                            },
                            "op": {
                                "type": "string",
                                "enum": ["eq", "neq", "gt", "gte", "lt", "lte", "is_null", "is_not_null"],
                                "description": "Filter operation"
                            },
                            "value": {
                                "description": "Value to compare against"
                            }
                        },
                        "required": ["field", "op"]
                    }
                },
                "order_by": {
                    "type": "string",
                    "enum": field_names,
                    "description": "Field to order results by (default: creation date descending)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results (default: 50, max: 200)",
                    "default": 50,
                    "minimum": 1,
                    "maximum": 200
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        // Parse filters — LLMs sometimes send "{}" (string) instead of [] (array).
        let filters: Vec<Filter> = match params.get("filters") {
            Some(v) if v.is_array() => serde_json::from_value(v.clone()).map_err(|e| {
                ToolError::InvalidParameters(format!("Invalid filters: {e}"))
            })?,
            Some(v) if v.is_string() => {
                // Try parsing stringified JSON; treat "{}" or empty as no filters.
                let s = v.as_str().unwrap_or("[]");
                if s == "{}" || s.trim().is_empty() {
                    Vec::new()
                } else {
                    serde_json::from_str(s).map_err(|e| {
                        ToolError::InvalidParameters(format!("Invalid filters string: {e}"))
                    })?
                }
            }
            _ => Vec::new(),
        };

        // Validate filter fields exist in the schema.
        for f in &filters {
            if !self.schema.fields.contains_key(&f.field) {
                return Err(ToolError::InvalidParameters(format!(
                    "Unknown filter field '{}'. Available fields: {}",
                    f.field,
                    self.schema.fields.keys().cloned().collect::<Vec<_>>().join(", ")
                )));
            }
        }

        let order_by = params.get("order_by").and_then(|v| v.as_str());
        // Validate order_by field exists in schema.
        if let Some(field) = order_by && !self.schema.fields.contains_key(field) {
            return Err(ToolError::InvalidParameters(format!(
                "Unknown order_by field '{field}'. Available fields: {}",
                self.schema.fields.keys().cloned().collect::<Vec<_>>().join(", ")
            )));
        }
        // LLMs sometimes send limit as string "50" instead of 50.
        let limit = params
            .get("limit")
            .and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok()))
            })
            .unwrap_or(50)
            .min(200) as usize;

        let records = self
            .db
            .query_records(
                &ctx.user_id,
                &self.schema.collection,
                &filters,
                order_by,
                limit,
            )
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to query records: {e}")))?;

        let results: Vec<serde_json::Value> = records
            .iter()
            .map(|r| {
                json!({
                    "id": r.id.to_string(),
                    "data": r.data,
                    "created_at": r.created_at.to_rfc3339(),
                    "updated_at": r.updated_at.to_rfc3339(),
                })
            })
            .collect();

        Ok(ToolOutput::success(
            json!({
                "collection": self.schema.collection,
                "results": results,
                "count": results.len(),
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

/// Tool for running aggregation queries on a specific collection.
pub struct CollectionSummaryTool {
    tool_name: String,
    schema: CollectionSchema,
    db: Arc<dyn Database>,
}

impl CollectionSummaryTool {
    pub fn new(schema: CollectionSchema, db: Arc<dyn Database>) -> Self {
        let tool_name = format!("{}_summary", schema.collection);
        Self {
            tool_name,
            schema,
            db,
        }
    }
}

#[async_trait]
impl Tool for CollectionSummaryTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        "Summarize records with aggregation operations like sum, count, average, \
         min, or max. Optionally group results by a field and filter before aggregating."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let field_names: Vec<&str> = self.schema.fields.keys().map(|s| s.as_str()).collect();
        let numeric_fields: Vec<&str> = self
            .schema
            .fields
            .iter()
            .filter(|(_, def)| matches!(def.field_type, FieldType::Number))
            .map(|(name, _)| name.as_str())
            .collect();

        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["sum", "count", "avg", "min", "max"],
                    "description": "The aggregation operation to perform"
                },
                "field": {
                    "type": "string",
                    "enum": if numeric_fields.is_empty() { field_names.clone() } else { numeric_fields },
                    "description": "The field to aggregate (required for sum/avg/min/max, optional for count)"
                },
                "group_by": {
                    "type": "string",
                    "enum": field_names,
                    "description": "Optional field to group results by"
                },
                "filters": {
                    "type": "array",
                    "description": "Optional filters to apply before aggregating",
                    "items": {
                        "type": "object",
                        "properties": {
                            "field": {
                                "type": "string",
                                "enum": field_names,
                                "description": "Field to filter on"
                            },
                            "op": {
                                "type": "string",
                                "enum": ["eq", "neq", "gt", "gte", "lt", "lte", "is_null", "is_not_null"],
                                "description": "Filter operation"
                            },
                            "value": {
                                "description": "Value to compare against"
                            }
                        },
                        "required": ["field", "op"]
                    }
                }
            },
            "required": ["operation"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let op_str = require_str(&params, "operation")?;
        let operation: AggOp = serde_json::from_value(json!(op_str)).map_err(|e| {
            ToolError::InvalidParameters(format!("Invalid operation: {e}"))
        })?;

        let field = params.get("field").and_then(|v| v.as_str()).map(String::from);
        let group_by = params
            .get("group_by")
            .and_then(|v| v.as_str())
            .map(String::from);

        let filters: Vec<Filter> = match params.get("filters") {
            Some(v) if v.is_array() => serde_json::from_value(v.clone()).map_err(|e| {
                ToolError::InvalidParameters(format!("Invalid filters: {e}"))
            })?,
            Some(v) if v.is_string() => {
                let s = v.as_str().unwrap_or("[]");
                if s == "{}" || s.trim().is_empty() {
                    Vec::new()
                } else {
                    serde_json::from_str(s).map_err(|e| {
                        ToolError::InvalidParameters(format!("Invalid filters string: {e}"))
                    })?
                }
            }
            _ => Vec::new(),
        };

        // Validate that sum/avg operations target numeric fields.
        if matches!(operation, AggOp::Sum | AggOp::Avg)
            && let Some(ref f) = field
            && let Some(def) = self.schema.fields.get(f)
            && !matches!(def.field_type, FieldType::Number)
        {
            return Err(ToolError::InvalidParameters(format!(
                "Cannot use {op_str} on non-numeric field '{f}' (type: {})",
                field_type_display(&def.field_type)
            )));
        }

        let aggregation = Aggregation {
            operation,
            field,
            group_by,
            filters,
        };

        let result = self
            .db
            .aggregate(&ctx.user_id, &self.schema.collection, &aggregation)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to aggregate: {e}")))?;

        Ok(ToolOutput::success(
            json!({
                "collection": self.schema.collection,
                "aggregation": result,
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_type_to_json_schema_text() {
        let schema = field_type_to_json_schema(&FieldType::Text);
        assert_eq!(schema["type"], "string");
    }

    #[test]
    fn field_type_to_json_schema_number() {
        let schema = field_type_to_json_schema(&FieldType::Number);
        assert_eq!(schema["type"], "number");
    }

    #[test]
    fn field_type_to_json_schema_bool() {
        let schema = field_type_to_json_schema(&FieldType::Bool);
        assert_eq!(schema["type"], "boolean");
    }

    #[test]
    fn field_type_to_json_schema_enum() {
        let schema = field_type_to_json_schema(&FieldType::Enum {
            values: vec!["a".to_string(), "b".to_string()],
        });
        assert_eq!(schema["type"], "string");
        assert_eq!(schema["enum"], json!(["a", "b"]));
    }

    #[test]
    fn field_type_to_json_schema_date() {
        let schema = field_type_to_json_schema(&FieldType::Date);
        assert_eq!(schema["type"], "string");
        assert_eq!(schema["format"], "date");
    }

    #[test]
    fn field_type_to_json_schema_datetime() {
        let schema = field_type_to_json_schema(&FieldType::DateTime);
        assert_eq!(schema["type"], "string");
        assert_eq!(schema["format"], "date-time");
    }

    #[test]
    fn field_type_to_json_schema_time() {
        let schema = field_type_to_json_schema(&FieldType::Time);
        assert_eq!(schema["type"], "string");
    }
}
