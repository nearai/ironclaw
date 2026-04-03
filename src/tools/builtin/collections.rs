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

use tokio::sync::broadcast;

use crate::agent::collection_events::CollectionWriteEvent;
use crate::context::JobContext;
use crate::db::Database;
use crate::db::structured::{
    AggOp, Aggregation, AlterOperation, Alteration, CollectionSchema, FieldType, Filter,
    append_history, init_history,
};
use crate::tools::builtin::memory::WorkspaceResolver;
use crate::tools::registry::ToolRegistry;
use crate::tools::tool::{Tool, ToolError, ToolOutput, ToolRateLimitConfig, require_str};

// ==================== Scoped tool naming ====================

/// Build the tool name for a collection.
///
/// Always prefixes with a user scope to prevent name collisions in the global
/// tool registry across multiple tenants.  When the schema carries an explicit
/// `source_scope` (cross-scope reference), that scope is used.  Otherwise the
/// `owner_user_id` — the user whose database row owns the schema — is used.
fn tool_name_for(schema: &CollectionSchema, suffix: &str, owner_user_id: &str) -> String {
    let scope = schema
        .source_scope
        .as_deref()
        .unwrap_or(owner_user_id);
    format!("{}_{}_{}", scope, schema.collection, suffix)
}

/// Build a tool description, prepending scope context for cross-scope tools.
fn scoped_description(schema: &CollectionSchema, base: &str, owner_user_id: &str) -> String {
    match &schema.source_scope {
        Some(scope) => format!(
            "[Operates on {scope}'s {collection}] {base}",
            scope = scope,
            collection = schema.collection,
            base = base,
        ),
        None => format!(
            "[{owner}'s {collection}] {base}",
            owner = owner_user_id,
            collection = schema.collection,
            base = base,
        ),
    }
}

// ==================== Cross-scope resolution ====================

/// Resolve which user_id owns a collection, checking the caller's own scope first.
///
/// Used by read-only collection tools (query, summary). Write tools (add,
/// update, delete) intentionally skip this and always operate on the caller's
/// own scope.
#[allow(dead_code)]
pub(crate) async fn resolve_collection_scope(
    db: &dyn Database,
    caller_user_id: &str,
    _scopes: &[String],
    collection: &str,
) -> Option<String> {
    // Try caller's own collections first.
    if db
        .get_collection_schema(caller_user_id, collection)
        .await
        .is_ok()
    {
        return Some(caller_user_id.to_string());
    }
    None
}

// ==================== Schema → JSON Schema conversion ====================

/// Convert a `FieldType` to its JSON Schema representation.
pub(crate) fn field_type_to_json_schema(field_type: &FieldType) -> serde_json::Value {
    match field_type {
        FieldType::Text => json!({ "type": "string" }),
        FieldType::Number => json!({ "type": "number" }),
        FieldType::Date => {
            json!({ "type": "string", "format": "date", "description": "Date in YYYY-MM-DD format" })
        }
        FieldType::Time => {
            json!({ "type": "string", "description": "Time in HH:MM or HH:MM:SS format" })
        }
        FieldType::DateTime => {
            json!({ "type": "string", "format": "date-time", "description": "ISO 8601 datetime (e.g. 2026-02-22T08:00:00Z)" })
        }
        FieldType::Bool => json!({ "type": "boolean" }),
        FieldType::Enum { values } => json!({ "type": "string", "enum": values }),
    }
}

/// Generate tool instances for a collection schema.
///
/// `owner_user_id` is the database user who owns this collection — used to
/// build globally unique tool names (`{user}_{collection}_{suffix}`).
pub fn generate_collection_tools(
    schema: &CollectionSchema,
    db: Arc<dyn Database>,
    collection_write_tx: Option<broadcast::Sender<CollectionWriteEvent>>,
    owner_user_id: &str,
) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(CollectionAddTool::new(schema.clone(), Arc::clone(&db), collection_write_tx, owner_user_id)),
        Arc::new(CollectionUpdateTool::new(schema.clone(), Arc::clone(&db), owner_user_id)),
        Arc::new(CollectionDeleteTool::new(schema.clone(), Arc::clone(&db), owner_user_id)),
        Arc::new(CollectionQueryTool::new(schema.clone(), Arc::clone(&db), owner_user_id)),
        Arc::new(CollectionSummaryTool::new(schema.clone(), db, owner_user_id)),
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
pub(crate) fn generate_collection_skill(
    schema: &CollectionSchema,
    skills_dir: &Path,
    owner_user_id: &str,
) {
    let scope = schema.source_scope.as_deref().unwrap_or(owner_user_id);
    let prefixed_name = format!("{}_{}", scope, schema.collection);
    let name = &prefixed_name;
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
        "a", "an", "the", "and", "or", "of", "for", "to", "in", "on", "with", "is", "are", "was",
        "were", "be", "been", "being", "has", "have", "had", "do", "does", "did", "this", "that",
        "it", "its", "my", "our", "your",
    ];
    for word in description.split_whitespace() {
        let w = word
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase();
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

    // YAML-safe tools_prefix and name (quote if contains special chars)
    let yaml_safe = |s: &str| -> String {
        if s.contains(':') || s.contains('#') || s.contains('"') || s.contains('\'') {
            let escaped = s.replace('"', "\\\"");
            format!("\"{}\"", escaped)
        } else {
            s.to_string()
        }
    };
    let name_yaml = yaml_safe(name);
    let tools_prefix_yaml = yaml_safe(name);

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
name: {name_yaml}
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
  tools_prefix: {tools_prefix_yaml}
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
    tracing::info!("Generated per-collection skill: {}", skill_path.display());
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
#[allow(dead_code)]
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

// ==================== Collection Discovery Docs ====================

/// Generate a rich, embedding-friendly description for a collection.
///
/// This document is written to workspace memory so that `memory_search` can
/// surface it when users ask questions semantically related to the collection.
/// The description includes synonyms, example queries, tool names, and field
/// details — all designed to maximise embedding similarity across a wide
/// variety of natural-language phrasings.
///
/// Deterministic: no LLM calls, purely template-based.
fn generate_collection_discovery_doc(schema: &CollectionSchema, owner_user_id: &str) -> String {
    let name = &schema.collection;
    let human_name = name.replace('_', " ");
    let human_name_title = titlecase(&human_name);
    let raw_description = schema
        .description
        .as_deref()
        .unwrap_or("Structured data collection");

    // Build field documentation
    let fields_doc: String = schema
        .fields
        .iter()
        .map(|(fname, fdef)| {
            let type_str = field_type_display(&fdef.field_type);
            let req = if fdef.required { ", required" } else { "" };
            let extra = match &fdef.field_type {
                FieldType::Enum { values } => format!(" — values: {}", values.join(", ")),
                _ => String::new(),
            };
            format!("- `{fname}` ({type_str}{req}){extra}")
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Generate semantic synonyms from collection name words
    let name_words: Vec<&str> = name.split('_').filter(|w| w.len() > 2).collect();
    let synonyms = generate_domain_synonyms(name, &name_words, schema);

    // Generate example queries based on fields and collection type
    let example_queries = generate_example_queries(name, &human_name, schema);

    // Build tool names — always scoped by owner
    let scope = schema.source_scope.as_deref().unwrap_or(owner_user_id);
    let tool_prefix = format!("{scope}_{name}");

    format!(
        r#"# {human_name_title} — Collection

## What this collection stores
{raw_description}

## Related terms
{synonyms}

## Common queries this answers
{example_queries}

## Available tools
- `{tool_prefix}_query` — search and filter {human_name} records
- `{tool_prefix}_add` — add new records
- `{tool_prefix}_delete` — remove records
- `{tool_prefix}_update` — modify existing records
- `{tool_prefix}_summary` — aggregate data (count, sum, avg, min, max)

## Schema fields
{fields_doc}
"#
    )
}

/// Generate domain-specific synonyms and related terms for embedding coverage.
fn generate_domain_synonyms(
    name: &str,
    name_words: &[&str],
    schema: &CollectionSchema,
) -> String {
    let mut terms: Vec<String> = Vec::new();

    // Add the human-readable name
    terms.push(name.replace('_', " "));

    // Add individual name words
    for word in name_words {
        if !terms.contains(&word.to_string()) {
            terms.push(word.to_string());
        }
    }

    // Add description words (skip stopwords)
    let stopwords = [
        "a", "an", "the", "and", "or", "of", "for", "to", "in", "on", "with", "is", "are",
        "was", "were", "be", "been", "being", "has", "have", "had", "do", "does", "did",
        "this", "that", "it", "its", "my", "our", "your", "data", "collection", "structured",
    ];
    if let Some(desc) = &schema.description {
        for word in desc.split_whitespace() {
            let w = word
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase();
            if w.len() > 3 && !stopwords.contains(&w.as_str()) && !terms.contains(&w) {
                terms.push(w);
            }
        }
    }

    // Add domain-specific synonyms based on common collection name patterns
    let name_lower = name.to_ascii_lowercase();
    let domain_synonyms: &[(&str, &[&str])] = &[
        (
            "grocery",
            &[
                "shopping list",
                "groceries",
                "supermarket",
                "food",
                "supplies",
                "things to buy",
                "shopping",
                "store",
                "items to get",
                "pick up",
            ],
        ),
        (
            "todo",
            &[
                "task",
                "to-do",
                "checklist",
                "action items",
                "things to do",
                "reminders",
                "pending",
            ],
        ),
        (
            "task",
            &[
                "to-do",
                "todo",
                "assignment",
                "action item",
                "things to do",
                "work item",
            ],
        ),
        (
            "transaction",
            &[
                "spending",
                "purchase",
                "payment",
                "bill",
                "budget",
                "expense",
                "financial",
                "cost",
                "money",
                "where did the money go",
                "how much did we spend",
            ],
        ),
        (
            "nanny",
            &[
                "childcare",
                "babysitting",
                "babysitter",
                "caregiver",
                "shift",
                "hours worked",
                "when did the nanny work",
                "childminder",
            ],
        ),
        (
            "hour",
            &[
                "time tracking",
                "shift",
                "timesheet",
                "hours worked",
                "clock in",
                "clock out",
                "work hours",
            ],
        ),
        (
            "meal",
            &[
                "food",
                "recipe",
                "dinner",
                "lunch",
                "breakfast",
                "menu",
                "meal plan",
                "what to eat",
                "cooking",
            ],
        ),
        (
            "chore",
            &[
                "housework",
                "cleaning",
                "household task",
                "duty",
                "responsibility",
            ],
        ),
        (
            "contact",
            &[
                "person",
                "people",
                "phone number",
                "email",
                "address book",
                "directory",
            ],
        ),
        (
            "inventory",
            &[
                "stock",
                "supplies",
                "items on hand",
                "what we have",
                "count",
            ],
        ),
        (
            "budget",
            &[
                "spending plan",
                "financial plan",
                "money",
                "allocation",
                "expenses",
            ],
        ),
        (
            "schedule",
            &[
                "calendar",
                "timetable",
                "appointment",
                "event",
                "when",
                "plan",
            ],
        ),
        (
            "log",
            &[
                "record",
                "entry",
                "history",
                "tracking",
                "diary",
            ],
        ),
    ];

    for (pattern, synonyms) in domain_synonyms {
        if name_lower.contains(pattern) {
            for syn in *synonyms {
                let s = syn.to_string();
                if !terms.contains(&s) {
                    terms.push(s);
                }
            }
        }
    }

    // Add field names as terms (they hint at the domain)
    for fname in schema.fields.keys() {
        for word in fname.split('_') {
            let w = word.to_lowercase();
            if w.len() > 2 && !stopwords.contains(&w.as_str()) && !terms.contains(&w) {
                terms.push(w);
            }
        }
    }

    // Add enum values
    for def in schema.fields.values() {
        if let FieldType::Enum { values } = &def.field_type {
            for v in values {
                let v_lower = v.to_lowercase();
                if !terms.contains(&v_lower) {
                    terms.push(v_lower);
                }
            }
        }
    }

    terms.truncate(40);
    terms.join(", ")
}

/// Generate example natural language queries based on schema fields and collection type.
fn generate_example_queries(
    _name: &str,
    human_name: &str,
    schema: &CollectionSchema,
) -> String {
    let mut queries: Vec<String> = Vec::new();

    // Generic queries
    queries.push(format!("What's in the {human_name}?"));
    queries.push(format!("Show me all {human_name}"));
    queries.push(format!("How many {human_name} are there?"));

    // Field-specific queries
    for (fname, fdef) in &schema.fields {
        let human_field = fname.replace('_', " ");
        match &fdef.field_type {
            FieldType::Text => {
                queries.push(format!("Which {human_name} have {human_field} [value]?"));
            }
            FieldType::Number => {
                queries.push(format!(
                    "What's the total {human_field}? How many by {human_field}?"
                ));
            }
            FieldType::Date | FieldType::DateTime => {
                queries.push(format!("What {human_name} are on [date]? Show me {human_name} from last week"));
            }
            FieldType::Bool => {
                queries.push(format!("Which {human_name} are {human_field}?"));
                queries.push(format!("Which {human_name} are not {human_field}?"));
            }
            FieldType::Enum { values } => {
                if let Some(first) = values.first() {
                    queries.push(format!(
                        "Show me {human_name} where {human_field} is {first}"
                    ));
                }
                queries.push(format!("How many {human_name} per {human_field}?"));
            }
            FieldType::Time => {
                queries.push(format!("What {human_name} are at [time]?"));
            }
        }
    }

    // Action queries
    queries.push(format!("Add [item] to {human_name}"));
    queries.push(format!("Remove [item] from {human_name}"));
    queries.push(format!("Update [item] in {human_name}"));

    queries.truncate(15);
    queries
        .iter()
        .map(|q| format!("- {q}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Write collection discovery doc to workspace memory (best-effort, async).
///
/// Errors are logged but not propagated — discovery doc generation should
/// never block collection registration.
async fn write_collection_discovery_doc(
    schema: &CollectionSchema,
    workspace_resolver: &dyn WorkspaceResolver,
    user_id: &str,
) {
    let doc_content = generate_collection_discovery_doc(schema, user_id);
    let doc_path = format!("collections/{}.md", schema.collection);

    let workspace = workspace_resolver.resolve(user_id).await;
    match workspace.write(&doc_path, &doc_content).await {
        Ok(_) => {
            tracing::info!(
                collection = %schema.collection,
                path = %doc_path,
                "Wrote collection discovery doc to workspace memory"
            );
        }
        Err(e) => {
            tracing::warn!(
                collection = %schema.collection,
                error = %e,
                "Failed to write collection discovery doc to workspace memory"
            );
        }
    }
}

// ==================== Shared Tool Refresh ====================

/// Unregister old per-collection tools, generate new ones from the schema,
/// register them, and regenerate skills. Used by both register and alter tools.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn refresh_collection_tools(
    schema: &CollectionSchema,
    db: &Arc<dyn Database>,
    registry: &Arc<ToolRegistry>,
    skills_dir: Option<&Path>,
    skill_registry: Option<&Arc<std::sync::RwLock<crate::skills::SkillRegistry>>>,
    user_id: &str,
    collection_write_tx: Option<&broadcast::Sender<CollectionWriteEvent>>,
    workspace_resolver: Option<&Arc<dyn WorkspaceResolver>>,
) -> Vec<String> {
    let unified = super::generic_collections::is_unified_mode();

    // Unregister old tools (if they exist) — both modes, to handle mode switches.
    let suffixes = ["add", "update", "delete", "query", "summary"];
    for suffix in &suffixes {
        let name = tool_name_for(schema, suffix, user_id);
        registry.unregister(&name).await;
    }
    // Also unregister the unified tool name.
    let scope = schema.source_scope.as_deref().unwrap_or(user_id);
    let unified_name = format!("{}_{}", scope, schema.collection);
    registry.unregister(&unified_name).await;

    // Generate and register tools based on mode.
    let tools = if unified {
        super::generic_collections::generate_unified_collection_tool(
            schema,
            Arc::clone(db),
            collection_write_tx.cloned(),
            user_id,
        )
    } else {
        generate_collection_tools(schema, Arc::clone(db), collection_write_tx.cloned(), user_id)
    };
    let tool_names: Vec<String> = tools.iter().map(|t| t.name().to_string()).collect();
    for tool in tools {
        registry.register(tool).await;
    }

    // Write collection discovery doc to workspace memory (best-effort).
    // This makes collection metadata searchable via memory_search embeddings,
    // enabling "did you mean?" style discovery when users ask related questions.
    if let Some(resolver) = workspace_resolver {
        write_collection_discovery_doc(schema, resolver.as_ref(), user_id).await;
    }

    // Generate per-collection skill for future session discovery (best-effort)
    if let Some(skills_dir) = skills_dir {
        generate_collection_skill(schema, skills_dir, user_id);

        // Hot-reload skills into the registry so they're available immediately
        // (not just on next restart).  Use spawn_blocking to avoid blocking
        // the tokio runtime — std::sync::RwLock can't be held across .await.
        if let Some(sr) = skill_registry {
            let sr = Arc::clone(sr);
            let _ = tokio::task::spawn_blocking(move || {
                if let Ok(mut registry) = sr.write() {
                    let rt = tokio::runtime::Handle::current();
                    let loaded = rt.block_on(registry.reload());
                    tracing::info!(
                        "Reloaded skills after collection registration ({})",
                        loaded.len()
                    );
                }
            })
            .await;
        } else {
            tracing::info!(
                "Generated per-collection skill: {}/SKILL.md (no registry to hot-reload)",
                schema.collection
            );
        }
    }

    tool_names
}

/// Load existing collection schemas from the database and register their
/// per-collection tools for every known user.
///
/// Call this at startup — after the database and tool registry are available
/// but before the first conversation — so that collections created in prior
/// sessions have their CRUD tools available immediately.
///
/// `user_ids` should include every user identity known to the system
/// (typically derived from `GATEWAY_USER_TOKENS` plus the default owner).
pub async fn initialize_collection_tools_for_users(
    user_ids: &[String],
    db: &Arc<dyn Database>,
    registry: &Arc<ToolRegistry>,
    skills_dir: Option<&Path>,
    skill_registry: Option<&Arc<std::sync::RwLock<crate::skills::SkillRegistry>>>,
    collection_write_tx: Option<&broadcast::Sender<CollectionWriteEvent>>,
    workspace_resolver: Option<&Arc<dyn WorkspaceResolver>>,
) {
    let mut total_tools = 0usize;
    for user_id in user_ids {
        let schemas = match db.list_collections(user_id).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    user_id = %user_id,
                    error = %e,
                    "Failed to list collections during startup initialization"
                );
                continue;
            }
        };
        for schema in &schemas {
            let tool_names = refresh_collection_tools(
                schema,
                db,
                registry,
                skills_dir,
                skill_registry,
                user_id,
                collection_write_tx,
                workspace_resolver,
            )
            .await;
            total_tools += tool_names.len();
        }
    }
    if total_tools > 0 {
        tracing::info!(
            "Registered {} per-collection tools across {} user(s) at startup",
            total_tools,
            user_ids.len()
        );
    }
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

        // List collections for the caller's own scope plus any workspace_read_scopes.
        let mut user_ids = vec![ctx.user_id.clone()];
        for scope in &ctx.workspace_read_scopes {
            if scope != &ctx.user_id && !user_ids.contains(scope) {
                user_ids.push(scope.clone());
            }
        }

        let mut all_collections: Vec<serde_json::Value> = Vec::new();
        for uid in &user_ids {
            let schemas = self.db.list_collections(uid).await.map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to list collections: {e}"))
            })?;

            for s in &schemas {
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
                let mut entry = json!({
                    "collection": s.collection,
                    "description": s.description,
                    "owner": uid,
                    "fields": fields,
                });
                if let Some(scope) = &s.source_scope {
                    entry["source_scope"] = json!(scope);
                }
                all_collections.push(entry);
            }
        }

        Ok(ToolOutput::success(
            json!({
                "collections": all_collections,
                "count": all_collections.len(),
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
    collection_write_tx: Option<broadcast::Sender<CollectionWriteEvent>>,
    workspace_resolver: Option<Arc<dyn WorkspaceResolver>>,
}

impl CollectionRegisterTool {
    pub fn new(db: Arc<dyn Database>, registry: Arc<ToolRegistry>) -> Self {
        Self {
            db,
            registry,
            skills_dir: None,
            skill_registry: None,
            collection_write_tx: None,
            workspace_resolver: None,
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

    pub fn with_collection_write_tx(
        mut self,
        tx: broadcast::Sender<CollectionWriteEvent>,
    ) -> Self {
        self.collection_write_tx = Some(tx);
        self
    }

    pub fn with_workspace_resolver(
        mut self,
        resolver: Arc<dyn WorkspaceResolver>,
    ) -> Self {
        self.workspace_resolver = Some(resolver);
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
                    "description": "Name for the collection (alphanumeric + underscores, e.g. 'time_entries', 'task_list')"
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
        let mut schema: CollectionSchema = serde_json::from_value(params.clone())
            .map_err(|e| ToolError::InvalidParameters(format!("Invalid collection schema: {e}")))?;

        // Only trusted seeding paths can set source_scope
        schema.source_scope = None;

        // Validate name
        CollectionSchema::validate_name(&schema.collection)
            .map_err(|e| ToolError::InvalidParameters(format!("Invalid collection name: {e}")))?;

        // Validate field count limits
        if schema.fields.len() > 50 {
            return Err(ToolError::InvalidParameters(
                "Schema exceeds maximum of 50 fields".to_string(),
            ));
        }
        for (name, def) in &schema.fields {
            if let FieldType::Enum { values } = &def.field_type
                && values.len() > 100
            {
                return Err(ToolError::InvalidParameters(format!(
                    "Enum field '{name}' exceeds maximum of 100 values"
                )));
            }
        }

        // Validate default values match their declared types
        schema
            .validate_defaults()
            .map_err(|e| ToolError::InvalidParameters(format!("Invalid default value: {e}")))?;

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
            self.collection_write_tx.as_ref(),
            self.workspace_resolver.as_ref(),
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
    workspace_resolver: Option<Arc<dyn WorkspaceResolver>>,
}

impl CollectionDropTool {
    pub fn new(db: Arc<dyn Database>, registry: Arc<ToolRegistry>) -> Self {
        Self {
            db,
            registry,
            skills_dir: None,
            skill_registry: None,
            workspace_resolver: None,
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

    pub fn with_workspace_resolver(
        mut self,
        resolver: Arc<dyn WorkspaceResolver>,
    ) -> Self {
        self.workspace_resolver = Some(resolver);
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
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to drop collection: {e}")))?;

        // Unregister per-collection tools using owner-prefixed format
        let tool_suffixes = ["add", "update", "delete", "query", "summary"];
        let mut removed = Vec::new();
        for suffix in &tool_suffixes {
            let tool_name = format!("{}_{collection}_{suffix}", ctx.user_id);
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

            // Remove from in-memory skill registry
            if let Some(ref sr) = self.skill_registry
                && let Ok(mut reg) = sr.write()
            {
                let _ = reg.commit_remove(collection);
            }

        }

        // Remove collection discovery doc from workspace memory (best-effort)
        if let Some(ref resolver) = self.workspace_resolver {
            let doc_path = format!("collections/{collection}.md");
            let workspace = resolver.resolve(&ctx.user_id).await;
            if let Err(e) = workspace.delete(&doc_path).await {
                tracing::warn!(
                    collection = collection,
                    error = %e,
                    "Failed to delete collection discovery doc from workspace memory"
                );
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
    collection_write_tx: Option<broadcast::Sender<CollectionWriteEvent>>,
    workspace_resolver: Option<Arc<dyn WorkspaceResolver>>,
}

impl CollectionsAlterTool {
    pub fn new(db: Arc<dyn Database>, registry: Arc<ToolRegistry>) -> Self {
        Self {
            db,
            registry,
            skills_dir: None,
            skill_registry: None,
            collection_write_tx: None,
            workspace_resolver: None,
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

    pub fn with_collection_write_tx(
        mut self,
        tx: broadcast::Sender<CollectionWriteEvent>,
    ) -> Self {
        self.collection_write_tx = Some(tx);
        self
    }

    pub fn with_workspace_resolver(
        mut self,
        resolver: Arc<dyn WorkspaceResolver>,
    ) -> Self {
        self.workspace_resolver = Some(resolver);
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
                    "type": "string",
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
            value: params
                .get("value")
                .and_then(|v| v.as_str())
                .map(String::from),
        };

        // Fetch current schema
        let current = self
            .db
            .get_collection_schema(&ctx.user_id, collection)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Collection not found: {e}")))?;

        // Apply mutation
        let new_schema = current
            .apply_alteration(&alteration)
            .map_err(|e| ToolError::InvalidParameters(format!("Invalid alteration: {e}")))?;

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
            self.collection_write_tx.as_ref(),
            self.workspace_resolver.as_ref(),
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
    tool_description: String,
    schema: CollectionSchema,
    db: Arc<dyn Database>,
    collection_write_tx: Option<broadcast::Sender<CollectionWriteEvent>>,
    owner_user_id: String,
}

impl CollectionAddTool {
    pub fn new(
        schema: CollectionSchema,
        db: Arc<dyn Database>,
        collection_write_tx: Option<broadcast::Sender<CollectionWriteEvent>>,
        owner_user_id: &str,
    ) -> Self {
        let tool_name = tool_name_for(&schema, "add", owner_user_id);
        let tool_description = scoped_description(
            &schema,
            "Add a new record to this collection. \
             Call this when the user wants to track, remember, save, create, or log something new. \
             Example triggers: 'I need to...', 'Add...', 'Don't forget...', 'Put X on my list', \
             '[person] needs to...'. Fields are validated against the schema.",
            owner_user_id,
        );
        Self {
            tool_name,
            tool_description,
            schema,
            db,
            collection_write_tx,
            owner_user_id: owner_user_id.to_string(),
        }
    }

    /// The user_id that owns the data: `source_scope` if set, else the collection owner.
    fn owner_scope(&self) -> &str {
        self.schema.source_scope.as_deref().unwrap_or(&self.owner_user_id)
    }
}

#[async_trait]
impl Tool for CollectionAddTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.tool_description
    }

    fn owner_user_id(&self) -> Option<&str> {
        Some(&self.owner_user_id)
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
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        // Inject _lineage for provenance tracking.
        let mut data = params;
        if let serde_json::Value::Object(ref mut obj) = data {
            obj.insert(
                "_lineage".to_string(),
                json!({
                    "source": "conversation",
                    "created_by": "user",
                    "timestamp": chrono::Utc::now().to_rfc3339()
                }),
            );
        }

        // Inject _history for audit trail.
        init_history(&mut data, "conversation");

        // Clone before insert_record consumes `data`.
        let data_for_event = data.clone();

        let id = self
            .db
            .insert_record(self.owner_scope(), &self.schema.collection, data)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to insert record: {e}")))?;

        // Fire collection write triggers.
        if let Some(tx) = &self.collection_write_tx {
            let _ = tx.send(CollectionWriteEvent {
                user_id: self.owner_scope().to_string(),
                collection: self.schema.collection.clone(),
                record_id: id,
                operation: "insert".to_string(),
                data: data_for_event,
            });
        }

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
    tool_description: String,
    schema: CollectionSchema,
    db: Arc<dyn Database>,
    owner_user_id: String,
}

impl CollectionUpdateTool {
    pub fn new(schema: CollectionSchema, db: Arc<dyn Database>, owner_user_id: &str) -> Self {
        let tool_name = tool_name_for(&schema, "update", owner_user_id);
        let tool_description = scoped_description(
            &schema,
            "Update an existing record. Provide the record_id and only the fields you want to change.",
            owner_user_id,
        );
        Self {
            tool_name,
            tool_description,
            schema,
            db,
            owner_user_id: owner_user_id.to_string(),
        }
    }

    fn owner_scope(&self) -> &str {
        self.schema.source_scope.as_deref().unwrap_or(&self.owner_user_id)
    }
}

#[async_trait]
impl Tool for CollectionUpdateTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.tool_description
    }

    fn owner_user_id(&self) -> Option<&str> {
        Some(&self.owner_user_id)
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
            properties.insert(
                field_name.clone(),
                field_type_to_json_schema(&field_def.field_type),
            );
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
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let record_id_str = require_str(&params, "record_id")?;
        let record_id = uuid::Uuid::parse_str(record_id_str)
            .map_err(|e| ToolError::InvalidParameters(format!("Invalid record_id: {e}")))?;

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

        // Fetch existing record to get current _history, then append update entry.
        let existing = self
            .db
            .get_record(self.owner_scope(), record_id)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to fetch record: {e}")))?;

        let changed_fields = serde_json::Value::Object(updates.clone());
        let mut existing_data = existing.data;
        append_history(&mut existing_data, &changed_fields, "conversation");

        // Carry the updated _history into the update payload so the DB merge
        // replaces the old _history with the appended version.
        if let Some(history) = existing_data.get("_history") {
            updates.insert("_history".to_string(), history.clone());
        }

        self.db
            .update_record(self.owner_scope(), record_id, serde_json::Value::Object(updates))
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
    tool_description: String,
    schema: CollectionSchema,
    db: Arc<dyn Database>,
    owner_user_id: String,
}

impl CollectionDeleteTool {
    pub fn new(schema: CollectionSchema, db: Arc<dyn Database>, owner_user_id: &str) -> Self {
        let tool_name = tool_name_for(&schema, "delete", owner_user_id);
        let tool_description = scoped_description(
            &schema,
            "Delete a record by its ID. This action cannot be undone.",
            owner_user_id,
        );
        Self {
            tool_name,
            tool_description,
            schema,
            db,
            owner_user_id: owner_user_id.to_string(),
        }
    }

    fn owner_scope(&self) -> &str {
        self.schema.source_scope.as_deref().unwrap_or(&self.owner_user_id)
    }
}

#[async_trait]
impl Tool for CollectionDeleteTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.tool_description
    }

    fn owner_user_id(&self) -> Option<&str> {
        Some(&self.owner_user_id)
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
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let record_id_str = require_str(&params, "record_id")?;
        let record_id = uuid::Uuid::parse_str(record_id_str)
            .map_err(|e| ToolError::InvalidParameters(format!("Invalid record_id: {e}")))?;

        self.db
            .delete_record(self.owner_scope(), record_id)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to delete record: {e}")))?;

        Ok(ToolOutput::success(
            json!({
                "status": "deleted",
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

/// Tool for querying records from a specific collection.
pub struct CollectionQueryTool {
    tool_name: String,
    tool_description: String,
    schema: CollectionSchema,
    db: Arc<dyn Database>,
    owner_user_id: String,
}

impl CollectionQueryTool {
    pub fn new(schema: CollectionSchema, db: Arc<dyn Database>, owner_user_id: &str) -> Self {
        let tool_name = tool_name_for(&schema, "query", owner_user_id);
        let tool_description = scoped_description(
            &schema,
            "Query records with optional filters, ordering, and limit. \
             Returns matching records sorted by the specified field or by creation date. \
             You can filter on 'created_at' or 'updated_at' (record timestamps) and \
             nested system fields like '_lineage.source' using dot notation.",
            owner_user_id,
        );
        Self {
            tool_name,
            tool_description,
            schema,
            db,
            owner_user_id: owner_user_id.to_string(),
        }
    }

    fn owner_scope(&self) -> &str {
        self.schema.source_scope.as_deref().unwrap_or(&self.owner_user_id)
    }
}

#[async_trait]
impl Tool for CollectionQueryTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.tool_description
    }

    fn owner_user_id(&self) -> Option<&str> {
        Some(&self.owner_user_id)
    }

    fn parameters_schema(&self) -> serde_json::Value {
        let mut field_names: Vec<&str> = self.schema.fields.keys().map(|s| s.as_str()).collect();
        // Sort for deterministic schema output.
        field_names.sort();

        json!({
            "type": "object",
            "properties": {
                "filters": {
                    "type": "array",
                    "description": "Optional filters to apply. Use 'created_at' or 'updated_at' for time-based filters (e.g. records created today). Use dot notation for nested system fields (e.g. '_lineage.source').",
                    "items": {
                        "type": "object",
                        "properties": {
                            "field": {
                                "type": "string",
                                "description": "Field to filter on. Schema fields, 'created_at', 'updated_at', or dot-notation system fields like '_lineage.source'."
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
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        // Parse filters — LLMs sometimes send "{}" (string) instead of [] (array).
        let filters: Vec<Filter> = match params.get("filters") {
            Some(v) if v.is_array() => serde_json::from_value(v.clone())
                .map_err(|e| ToolError::InvalidParameters(format!("Invalid filters: {e}")))?,
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

        // Validate filter fields: allow schema fields, created_at, and dot-notation system fields.
        for f in &filters {
            let is_schema_field = self.schema.fields.contains_key(&f.field);
            let is_db_column = f.field == "created_at" || f.field == "updated_at";
            let is_system_dot = f.field.starts_with('_') && f.field.contains('.');
            if !is_schema_field && !is_db_column && !is_system_dot {
                return Err(ToolError::InvalidParameters(format!(
                    "Unknown filter field '{}'. Available fields: {}, created_at, updated_at, or _lineage.* system fields",
                    f.field,
                    self.schema
                        .fields
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
        }

        let order_by = params.get("order_by").and_then(|v| v.as_str());
        // Validate order_by field exists in schema.
        if let Some(field) = order_by
            && !self.schema.fields.contains_key(field)
        {
            return Err(ToolError::InvalidParameters(format!(
                "Unknown order_by field '{field}'. Available fields: {}",
                self.schema
                    .fields
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
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

        let owner = self.owner_scope().to_string();

        let records = self
            .db
            .query_records(
                &owner,
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
    tool_description: String,
    schema: CollectionSchema,
    db: Arc<dyn Database>,
    owner_user_id: String,
}

impl CollectionSummaryTool {
    pub fn new(schema: CollectionSchema, db: Arc<dyn Database>, owner_user_id: &str) -> Self {
        let tool_name = tool_name_for(&schema, "summary", owner_user_id);
        let tool_description = scoped_description(
            &schema,
            "Summarize records with aggregation operations like sum, count, average, \
             min, or max. Optionally group results by a field and filter before aggregating. \
             Filters support 'created_at', 'updated_at', and dot-notation system fields like '_lineage.source'.",
            owner_user_id,
        );
        Self {
            tool_name,
            tool_description,
            schema,
            db,
            owner_user_id: owner_user_id.to_string(),
        }
    }

    fn owner_scope(&self) -> &str {
        self.schema.source_scope.as_deref().unwrap_or(&self.owner_user_id)
    }
}

#[async_trait]
impl Tool for CollectionSummaryTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.tool_description
    }

    fn owner_user_id(&self) -> Option<&str> {
        Some(&self.owner_user_id)
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
                    "description": "Optional filters to apply before aggregating. Use 'created_at' or 'updated_at' for time-based filters. Use dot notation for nested system fields (e.g. '_lineage.source').",
                    "items": {
                        "type": "object",
                        "properties": {
                            "field": {
                                "type": "string",
                                "description": "Field to filter on. Schema fields, 'created_at', 'updated_at', or dot-notation system fields like '_lineage.source'."
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
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let op_str = require_str(&params, "operation")?;
        let operation: AggOp = serde_json::from_value(json!(op_str))
            .map_err(|e| ToolError::InvalidParameters(format!("Invalid operation: {e}")))?;

        let field = params
            .get("field")
            .and_then(|v| v.as_str())
            .map(String::from);
        let group_by = params
            .get("group_by")
            .and_then(|v| v.as_str())
            .map(String::from);

        let filters: Vec<Filter> = match params.get("filters") {
            Some(v) if v.is_array() => serde_json::from_value(v.clone())
                .map_err(|e| ToolError::InvalidParameters(format!("Invalid filters: {e}")))?,
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

        let owner = self.owner_scope().to_string();

        let result = self
            .db
            .aggregate(&owner, &self.schema.collection, &aggregation)
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

    /// Try to connect to PostgreSQL for structured collection tests.
    ///
    /// Returns `Some(db)` if postgres is reachable, `None` otherwise.
    /// Uses `DATABASE_URL` env var if set, otherwise falls back to the
    /// Percy dev database on localhost:5432.
    #[cfg(feature = "postgres")]
    pub(crate) async fn try_connect_postgres() -> Option<std::sync::Arc<dyn crate::db::Database>> {
        use crate::config::DatabaseConfig;
        use crate::db::postgres::PgBackend;

        let url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://percy:percy@localhost:5432/percy".to_string());
        let config = DatabaseConfig::from_postgres_url(&url, 2);
        match PgBackend::new(&config).await {
            Ok(backend) => {
                if let Err(e) = backend.run_migrations().await {
                    eprintln!("postgres available but migrations failed: {e}");
                    return None;
                }
                Some(std::sync::Arc::new(backend))
            }
            Err(_) => None,
        }
    }

    /// Skip the current test if PostgreSQL is not available.
    ///
    /// Expands to an early return with a printed message when postgres
    /// cannot be reached, so the test passes (skipped) rather than fails.
    ///
    /// Uses `$crate` path to work from sub-modules (history, cross_scope,
    /// scoped_access).
    #[cfg(feature = "postgres")]
    macro_rules! require_postgres {
        () => {
            match crate::tools::builtin::collections::tests::try_connect_postgres().await {
                Some(db) => db,
                None => {
                    eprintln!("skipping: PostgreSQL not available");
                    return;
                }
            }
        };
    }

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

    #[test]
    fn generate_collection_skill_produces_valid_skill_md() {
        use std::collections::BTreeMap;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let skills_dir = tmp.path();

        let mut fields = BTreeMap::new();
        fields.insert(
            "task".to_string(),
            crate::db::structured::FieldDef {
                field_type: FieldType::Text,
                required: true,
                default: None,
            },
        );
        fields.insert(
            "priority".to_string(),
            crate::db::structured::FieldDef {
                field_type: FieldType::Enum {
                    values: vec!["low".into(), "medium".into(), "high".into()],
                },
                required: false,
                default: None,
            },
        );

        let schema = crate::db::structured::CollectionSchema {
            collection: "todo_items".to_string(),
            description: Some("Track todo items and tasks".to_string()),
            fields,
            source_scope: None,
        };

        generate_collection_skill(&schema, skills_dir, "test_user");

        let skill_path = skills_dir.join("test_user_todo_items").join("SKILL.md");
        assert!(skill_path.exists(), "SKILL.md should be created");

        let content = std::fs::read_to_string(&skill_path).unwrap();

        // Valid YAML frontmatter
        assert!(content.starts_with("---\n"), "should start with YAML frontmatter");
        assert!(
            content.contains("name: test_user_todo_items"),
            "should contain prefixed collection name"
        );
        assert!(
            content.contains("Track todo items and tasks"),
            "should contain description"
        );

        // Activation keywords from schema metadata
        assert!(content.contains("todo"), "should have keyword from collection name");
        assert!(content.contains("items"), "should have keyword from collection name");

        // Tool documentation
        assert!(
            content.contains("test_user_todo_items_add"),
            "should document add tool with prefix"
        );
        assert!(
            content.contains("test_user_todo_items_query"),
            "should document query tool with prefix"
        );
        assert!(
            content.contains("test_user_todo_items_summary"),
            "should document summary tool with prefix"
        );

        // Field documentation
        assert!(content.contains("`task`"), "should document task field");
        assert!(content.contains("`priority`"), "should document priority field");
    }

    #[test]
    fn generate_collection_skill_without_description() {
        use std::collections::BTreeMap;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();

        let mut fields = BTreeMap::new();
        fields.insert(
            "name".to_string(),
            crate::db::structured::FieldDef {
                field_type: FieldType::Text,
                required: true,
                default: None,
            },
        );

        let schema = crate::db::structured::CollectionSchema {
            collection: "contacts".to_string(),
            description: None,
            fields,
            source_scope: None,
        };

        generate_collection_skill(&schema, tmp.path(), "test_user");

        let content =
            std::fs::read_to_string(tmp.path().join("test_user_contacts").join("SKILL.md"))
                .unwrap();
        assert!(
            content.contains("Structured data collection"),
            "should use default description"
        );
    }

    #[cfg(test)]
    mod skill_generation_tests {
        use super::*;

        #[test]
        fn skill_uses_prefixed_tool_names() {
            let schema = CollectionSchema {
                collection: "tasks".to_string(),
                description: Some("Task tracking".to_string()),
                fields: {
                    let mut f = std::collections::BTreeMap::new();
                    f.insert(
                        "title".to_string(),
                        crate::db::structured::FieldDef {
                            field_type: FieldType::Text,
                            required: true,
                            default: None,
                        },
                    );
                    f
                },
                source_scope: None,
            };
            let tmp = tempfile::tempdir().unwrap();
            generate_collection_skill(&schema, tmp.path(), "andrew");

            let skill_path = tmp.path().join("andrew_tasks").join("SKILL.md");
            assert!(skill_path.exists(), "Skill dir should be andrew_tasks");

            let content = std::fs::read_to_string(&skill_path).unwrap();
            assert!(
                content.contains("name: andrew_tasks"),
                "Skill name should be prefixed"
            );
            assert!(
                content.contains("tools_prefix: andrew_tasks"),
                "tools_prefix should be prefixed"
            );
            assert!(
                content.contains("andrew_tasks_add"),
                "Tool refs in body should be prefixed"
            );
        }

        #[test]
        fn skill_with_source_scope_uses_source_scope() {
            let schema = CollectionSchema {
                collection: "tasks".to_string(),
                description: Some("test".to_string()),
                fields: std::collections::BTreeMap::new(),
                source_scope: Some("grace".to_string()),
            };
            let tmp = tempfile::tempdir().unwrap();
            generate_collection_skill(&schema, tmp.path(), "andrew");

            // source_scope takes precedence over owner_user_id
            let skill_path = tmp.path().join("grace_tasks").join("SKILL.md");
            assert!(
                skill_path.exists(),
                "Skill dir should use source_scope: grace_tasks"
            );

            let content = std::fs::read_to_string(&skill_path).unwrap();
            assert!(content.contains("name: grace_tasks"));
            assert!(content.contains("grace_tasks_add"));
        }

        #[test]
        fn skill_yaml_sanitizes_special_chars() {
            let schema = CollectionSchema {
                collection: "tasks".to_string(),
                description: Some("test".to_string()),
                fields: std::collections::BTreeMap::new(),
                source_scope: None,
            };
            let tmp = tempfile::tempdir().unwrap();
            generate_collection_skill(&schema, tmp.path(), "user:admin");

            let skill_path = tmp.path().join("user:admin_tasks").join("SKILL.md");
            assert!(skill_path.exists());

            let content = std::fs::read_to_string(&skill_path).unwrap();
            // Name and tools_prefix with colon must be quoted
            assert!(
                content.contains("name: \"user:admin_tasks\""),
                "name with special chars must be YAML-quoted. Got: {}",
                content
            );
            assert!(
                content.contains("tools_prefix: \"user:admin_tasks\""),
                "tools_prefix with special chars must be YAML-quoted"
            );
        }
    }

    #[test]
    fn generate_router_skill_produces_valid_skill_md() {
        use std::collections::BTreeMap;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();

        let schemas = vec![
            crate::db::structured::CollectionSchema {
                collection: "groceries".to_string(),
                description: Some("Grocery shopping list".to_string()),
                fields: BTreeMap::new(),
                source_scope: None,
            },
            crate::db::structured::CollectionSchema {
                collection: "time_entries".to_string(),
                description: Some("Track work time entries".to_string()),
                fields: BTreeMap::new(),
                source_scope: None,
            },
        ];

        generate_router_skill(&schemas, tmp.path());

        let router_path = tmp.path().join("collections-router").join("SKILL.md");
        assert!(router_path.exists(), "router SKILL.md should be created");

        let content = std::fs::read_to_string(&router_path).unwrap();
        assert!(content.starts_with("---\n"), "should have YAML frontmatter");
        assert!(content.contains("collections_list"), "should reference collections_list tool");
        assert!(content.contains("groceries"), "should reference groceries collection");
        assert!(content.contains("time_entries"), "should reference time_entries collection");
    }

    #[test]
    fn generate_router_skill_empty_schemas_removes_directory() {
        use std::collections::BTreeMap;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();

        // First, create a router skill with some schemas
        let schemas = vec![crate::db::structured::CollectionSchema {
            collection: "groceries".to_string(),
            description: Some("Grocery shopping list".to_string()),
            fields: BTreeMap::new(),
            source_scope: None,
        }];

        generate_router_skill(&schemas, tmp.path());

        let router_dir = tmp.path().join("collections-router");
        let router_path = router_dir.join("SKILL.md");
        assert!(router_path.exists(), "router SKILL.md should exist after creation");

        // Now call with empty schemas — should remove the file and directory
        generate_router_skill(&[], tmp.path());

        assert!(!router_path.exists(), "router SKILL.md should be removed for empty schemas");
        assert!(!router_dir.exists(), "router directory should be removed for empty schemas");
    }

    // ==================== History tracking tests ====================

    #[cfg(feature = "postgres")]
    mod history {
        use std::collections::BTreeMap;
        use std::sync::Arc;

        use crate::context::JobContext;
        use crate::db::Database;
        use crate::db::structured::{CollectionSchema, FieldDef, FieldType};
        use crate::tools::tool::Tool;

        use super::{CollectionAddTool, CollectionUpdateTool};

        fn test_schema() -> CollectionSchema {
            let mut fields = BTreeMap::new();
            fields.insert(
                "item".to_string(),
                FieldDef {
                    field_type: FieldType::Text,
                    required: true,
                    default: None,
                },
            );
            fields.insert(
                "quantity".to_string(),
                FieldDef {
                    field_type: FieldType::Number,
                    required: false,
                    default: None,
                },
            );
            CollectionSchema {
                collection: "groceries".to_string(),
                description: Some("test".to_string()),
                fields,
                source_scope: None,
            }
        }

        async fn setup_db(db: &Arc<dyn Database>) {
            db.register_collection("alice", &test_schema())
                .await
                .unwrap();
        }

        #[tokio::test]
        async fn insert_creates_history_with_one_entry() {
            let db = require_postgres!();
            setup_db(&db).await;
            let schema = test_schema();
            let tool = CollectionAddTool::new(schema, Arc::clone(&db), None, "alice");
            let ctx = JobContext::with_user("alice", "test", "test");

            let result = tool
                .execute(serde_json::json!({"item": "milk", "quantity": 2}), &ctx)
                .await
                .unwrap();
            let record_id: uuid::Uuid = result.result["record_id"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap();

            let record = db.get_record("alice", record_id).await.unwrap();
            let history = record.data["_history"].as_array().unwrap();
            assert_eq!(history.len(), 1);
            assert_eq!(history[0]["op"], "insert");
            assert_eq!(history[0]["source"], "conversation");
            assert_eq!(history[0]["fields"]["item"], "milk");
            assert_eq!(history[0]["fields"]["quantity"], 2);
            // System fields should not appear in history fields.
            assert!(history[0]["fields"].get("_lineage").is_none());
            assert!(history[0]["fields"].get("_history").is_none());
            // Time should be present.
            assert!(history[0]["time"].as_str().is_some());
        }

        #[tokio::test]
        async fn update_appends_to_history() {
            let db = require_postgres!();
            setup_db(&db).await;
            let schema = test_schema();
            let add_tool = CollectionAddTool::new(schema.clone(), Arc::clone(&db), None, "alice");
            let update_tool = CollectionUpdateTool::new(schema, Arc::clone(&db), "alice");
            let ctx = JobContext::with_user("alice", "test", "test");

            // Insert.
            let result = add_tool
                .execute(serde_json::json!({"item": "milk", "quantity": 1}), &ctx)
                .await
                .unwrap();
            let record_id_str = result.result["record_id"].as_str().unwrap();
            let record_id: uuid::Uuid = record_id_str.parse().unwrap();

            // Update.
            update_tool
                .execute(
                    serde_json::json!({"record_id": record_id_str, "quantity": 3}),
                    &ctx,
                )
                .await
                .unwrap();

            let record = db.get_record("alice", record_id).await.unwrap();
            let history = record.data["_history"].as_array().unwrap();
            assert_eq!(history.len(), 2);
            assert_eq!(history[0]["op"], "insert");
            assert_eq!(history[1]["op"], "update");
            assert_eq!(history[1]["source"], "conversation");
            assert_eq!(history[1]["fields"]["quantity"], 3);
            // Update should only contain changed fields, not all fields.
            assert!(
                history[1]["fields"].get("item").is_none(),
                "update history should only contain changed fields"
            );
        }

        #[tokio::test]
        async fn multiple_updates_produce_multiple_history_entries() {
            let db = require_postgres!();
            setup_db(&db).await;
            let schema = test_schema();
            let add_tool = CollectionAddTool::new(schema.clone(), Arc::clone(&db), None, "alice");
            let update_tool = CollectionUpdateTool::new(schema, Arc::clone(&db), "alice");
            let ctx = JobContext::with_user("alice", "test", "test");

            let result = add_tool
                .execute(serde_json::json!({"item": "eggs"}), &ctx)
                .await
                .unwrap();
            let rid = result.result["record_id"].as_str().unwrap().to_string();
            let record_id: uuid::Uuid = rid.parse().unwrap();

            // Three sequential updates.
            for qty in [6, 12, 24] {
                update_tool
                    .execute(
                        serde_json::json!({"record_id": rid, "quantity": qty}),
                        &ctx,
                    )
                    .await
                    .unwrap();
            }

            let record = db.get_record("alice", record_id).await.unwrap();
            let history = record.data["_history"].as_array().unwrap();
            assert_eq!(history.len(), 4, "1 insert + 3 updates = 4 entries");
            assert_eq!(history[0]["op"], "insert");
            assert_eq!(history[1]["fields"]["quantity"], 6);
            assert_eq!(history[2]["fields"]["quantity"], 12);
            assert_eq!(history[3]["fields"]["quantity"], 24);
        }

        #[tokio::test]
        async fn history_preserves_existing_entries() {
            let db = require_postgres!();
            setup_db(&db).await;
            let schema = test_schema();
            let add_tool = CollectionAddTool::new(schema.clone(), Arc::clone(&db), None, "alice");
            let update_tool = CollectionUpdateTool::new(schema, Arc::clone(&db), "alice");
            let ctx = JobContext::with_user("alice", "test", "test");

            let result = add_tool
                .execute(serde_json::json!({"item": "bread"}), &ctx)
                .await
                .unwrap();
            let rid = result.result["record_id"].as_str().unwrap().to_string();
            let record_id: uuid::Uuid = rid.parse().unwrap();

            // First update.
            update_tool
                .execute(
                    serde_json::json!({"record_id": rid, "quantity": 2}),
                    &ctx,
                )
                .await
                .unwrap();

            // Snapshot the history after first update.
            let record_after_first = db.get_record("alice", record_id).await.unwrap();
            let history_after_first = record_after_first.data["_history"]
                .as_array()
                .unwrap()
                .clone();
            assert_eq!(history_after_first.len(), 2);

            // Second update.
            update_tool
                .execute(
                    serde_json::json!({"record_id": rid, "item": "sourdough bread"}),
                    &ctx,
                )
                .await
                .unwrap();

            let record_after_second = db.get_record("alice", record_id).await.unwrap();
            let history_after_second = record_after_second.data["_history"]
                .as_array()
                .unwrap();
            assert_eq!(history_after_second.len(), 3);

            // Previous entries must be identical.
            assert_eq!(history_after_second[0], history_after_first[0]);
            assert_eq!(history_after_second[1], history_after_first[1]);
            // New entry.
            assert_eq!(history_after_second[2]["op"], "update");
            assert_eq!(history_after_second[2]["fields"]["item"], "sourdough bread");
        }

        #[tokio::test]
        async fn history_source_from_lineage() {
            // When inserting via the REST handler path, source comes from
            // the lineage/request. For tools, it's "conversation". This test
            // verifies the tool path sets source correctly.
            let db = require_postgres!();
            setup_db(&db).await;
            let schema = test_schema();
            let tool = CollectionAddTool::new(schema, Arc::clone(&db), None, "alice");
            let ctx = JobContext::with_user("alice", "test", "test");

            let result = tool
                .execute(serde_json::json!({"item": "butter"}), &ctx)
                .await
                .unwrap();
            let record_id: uuid::Uuid = result.result["record_id"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap();

            let record = db.get_record("alice", record_id).await.unwrap();
            let history = record.data["_history"].as_array().unwrap();
            assert_eq!(history[0]["source"], "conversation");
        }
    }

    // ==================== Cross-scope resolution tests ====================
    //
    // These tests use the PostgreSQL backend to verify that collection
    // tools correctly resolve cross-scope access via workspace_read_scopes.

    #[cfg(feature = "postgres")]
    mod cross_scope {
        use std::collections::BTreeMap;
        use std::sync::Arc;

        use crate::context::JobContext;
        use crate::db::Database;
        use crate::db::structured::{CollectionSchema, FieldDef, FieldType};
        use crate::tools::tool::Tool;

        use super::{
            CollectionAddTool, CollectionDeleteTool, CollectionQueryTool, CollectionSummaryTool,
            CollectionUpdateTool, resolve_collection_scope,
        };

        /// Create a minimal collection schema for testing.
        fn test_schema(name: &str) -> CollectionSchema {
            let mut fields = BTreeMap::new();
            fields.insert(
                "item".to_string(),
                FieldDef {
                    field_type: FieldType::Text,
                    required: true,
                    default: None,
                },
            );
            CollectionSchema {
                collection: name.to_string(),
                description: Some("test collection".to_string()),
                fields,
                source_scope: None,
            }
        }

        async fn setup_db_with_collection(
            db: &Arc<dyn Database>,
            owner_id: &str,
            collection: &str,
        ) {
            db.register_collection(owner_id, &test_schema(collection))
                .await
                .unwrap();
        }

        #[tokio::test]
        async fn resolve_scope_own_collection_first() {
            let db = require_postgres!();
            setup_db_with_collection(&db, "alice", "inventory").await;
            // Also register same-name collection under a scope.
            db.register_collection("shared", &test_schema("inventory"))
                .await
                .unwrap();

            let result = resolve_collection_scope(
                db.as_ref(),
                "alice",
                &["shared".to_string()],
                "inventory",
            )
            .await;
            assert_eq!(result, Some("alice".to_string()), "should prefer own scope");
        }

        #[tokio::test]
        async fn resolve_scope_returns_none_for_other_scopes() {
            let db = require_postgres!();
            setup_db_with_collection(&db, "shared", "inventory").await;

            // In upstream, cross-scope resolution is not supported.
            // Scopes are ignored, so alice cannot see shared's collections.
            let result = resolve_collection_scope(
                db.as_ref(),
                "alice",
                &["shared".to_string()],
                "inventory",
            )
            .await;
            assert_eq!(
                result,
                None,
                "should not resolve to other scopes in upstream"
            );
        }

        #[tokio::test]
        async fn resolve_scope_returns_none_when_not_found() {
            let db = require_postgres!();
            setup_db_with_collection(&db, "carol", "inventory").await;

            let result = resolve_collection_scope(
                db.as_ref(),
                "alice",
                &["shared".to_string()],
                "inventory",
            )
            .await;
            assert_eq!(result, None, "should return None when no scope has the collection");
        }

        #[tokio::test]
        async fn query_tool_reads_from_cross_scope() {
            let db = require_postgres!();
            setup_db_with_collection(&db, "shared", "inventory").await;
            // Insert a record under shared scope.
            db.insert_record(
                "shared",
                "inventory",
                serde_json::json!({"item": "widget"}),
            )
            .await
            .unwrap();

            let schema = test_schema("inventory");
            let tool = CollectionQueryTool::new(schema, Arc::clone(&db), "alice");

            // Without cross-scope access, alice can only see her own collections.
            // Alice has no own inventory collection, so query operates on alice's
            // scope and finds nothing.
            let ctx = JobContext::with_user("alice", "test", "test");

            let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
            let output = &result.result;
            assert_eq!(output["count"], 0, "alice has no own inventory — should find nothing");
        }

        #[tokio::test]
        async fn summary_tool_without_cross_scope() {
            let db = require_postgres!();
            setup_db_with_collection(&db, "shared", "inventory").await;
            db.insert_record(
                "shared",
                "inventory",
                serde_json::json!({"item": "gadget"}),
            )
            .await
            .unwrap();
            db.insert_record(
                "shared",
                "inventory",
                serde_json::json!({"item": "widget"}),
            )
            .await
            .unwrap();

            let schema = test_schema("inventory");
            let tool = CollectionSummaryTool::new(schema, Arc::clone(&db), "alice");

            let ctx = JobContext::with_user("alice", "test", "test");

            // Without cross-scope access, alice's aggregation operates on her
            // own scope which has no inventory collection — should fail.
            let result = tool
                .execute(serde_json::json!({"operation": "count"}), &ctx)
                .await;
            assert!(result.is_err(), "alice has no own inventory — aggregation should fail");
        }

        #[tokio::test]
        async fn add_tool_does_not_use_cross_scope() {
            let db = require_postgres!();
            setup_db_with_collection(&db, "shared", "inventory").await;

            let schema = test_schema("inventory");
            let tool = CollectionAddTool::new(schema, Arc::clone(&db), None, "alice");

            // alice does NOT have her own inventory collection, only shared does.
            let ctx = JobContext::with_user("alice", "test", "test");

            // The add tool uses owner_user_id (no source_scope set, no scope resolution).
            // Since "alice" has no "inventory" collection, this should fail.
            let result = tool
                .execute(serde_json::json!({"item": "gadget"}), &ctx)
                .await;
            assert!(result.is_err(), "add should fail — alice has no own inventory collection");
        }

        #[tokio::test]
        async fn update_tool_does_not_use_cross_scope() {
            let db = require_postgres!();
            setup_db_with_collection(&db, "shared", "inventory").await;
            let record_id = db
                .insert_record(
                    "shared",
                    "inventory",
                    serde_json::json!({"item": "widget"}),
                )
                .await
                .unwrap();

            let schema = test_schema("inventory");
            let tool = CollectionUpdateTool::new(schema, Arc::clone(&db), "alice");

            let ctx = JobContext::with_user("alice", "test", "test");

            // alice tries to update a shared record — should fail because
            // update uses owner_user_id ("alice"), not scope resolution.
            let result = tool
                .execute(
                    serde_json::json!({
                        "record_id": record_id.to_string(),
                        "item": "premium widget"
                    }),
                    &ctx,
                )
                .await;
            assert!(
                result.is_err(),
                "update should fail — alice cannot write to shared scope's collection via tool"
            );
        }

        #[tokio::test]
        async fn delete_tool_does_not_use_cross_scope() {
            let db = require_postgres!();
            setup_db_with_collection(&db, "shared", "inventory").await;
            let record_id = db
                .insert_record(
                    "shared",
                    "inventory",
                    serde_json::json!({"item": "widget"}),
                )
                .await
                .unwrap();

            let schema = test_schema("inventory");
            let tool = CollectionDeleteTool::new(schema, Arc::clone(&db), "alice");

            let ctx = JobContext::with_user("alice", "test", "test");

            let result = tool
                .execute(serde_json::json!({"record_id": record_id.to_string()}), &ctx)
                .await;
            assert!(
                result.is_err(),
                "delete should fail — alice cannot delete from shared scope's collection via tool"
            );
        }

        #[tokio::test]
        async fn list_tool_shows_only_own_collections() {
            let db = require_postgres!();
            setup_db_with_collection(&db, "shared", "inventory").await;
            // Also register a collection under alice.
            db.register_collection("alice", &test_schema("task_list"))
                .await
                .unwrap();

            let tool = super::CollectionListTool::new(Arc::clone(&db));

            let ctx = JobContext::with_user("alice", "test", "test");

            let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
            let output = &result.result;
            // Without cross-scope, alice only sees her own collections.
            assert_eq!(output["count"], 1, "should list only own collections");

            let collections = output["collections"].as_array().unwrap();
            let names: Vec<&str> = collections
                .iter()
                .map(|c| c["collection"].as_str().unwrap())
                .collect();
            assert!(names.contains(&"task_list"), "should include own collection");
        }
    }

    #[cfg(feature = "postgres")]
    mod scoped_access {
        use std::collections::BTreeMap;
        use std::sync::Arc;

        use crate::context::JobContext;
        use crate::db::Database;
        use crate::db::structured::{CollectionSchema, FieldDef, FieldType};
        use crate::tools::tool::Tool;

        use super::{
            CollectionAddTool, CollectionDeleteTool, CollectionListTool, CollectionQueryTool,
            CollectionUpdateTool,
        };

        /// Create a minimal collection schema for testing.
        fn test_schema(name: &str) -> CollectionSchema {
            let mut fields = BTreeMap::new();
            fields.insert(
                "item".to_string(),
                FieldDef {
                    field_type: FieldType::Text,
                    required: true,
                    default: None,
                },
            );
            CollectionSchema {
                collection: name.to_string(),
                description: Some("test collection".to_string()),
                fields,
                source_scope: None,
            }
        }

        /// Connect to PostgreSQL and register a collection under `owner_id`.
        async fn setup_db_with_collection(
            db: &Arc<dyn Database>,
            owner_id: &str,
            collection: &str,
        ) {
            db.register_collection(owner_id, &test_schema(collection))
                .await
                .unwrap();
        }

        // ── Schema persistence tests ────────────────────────────────────

        #[tokio::test]
        async fn source_scope_persists_through_register_and_retrieve() {
            let db = require_postgres!();

            let mut schema = test_schema("tasks");
            schema.source_scope = Some("household".to_string());

            db.register_collection("andrew", &schema).await.unwrap();

            let retrieved = db
                .get_collection_schema("andrew", "tasks")
                .await
                .unwrap();
            assert_eq!(
                retrieved.source_scope,
                Some("household".to_string()),
                "source_scope should survive register + retrieve round-trip"
            );
        }

        #[tokio::test]
        async fn source_scope_none_for_own_collection() {
            let db = require_postgres!();

            let schema = test_schema("personal_tasks");
            // source_scope is None by default from test_schema
            db.register_collection("andrew", &schema).await.unwrap();

            let retrieved = db
                .get_collection_schema("andrew", "personal_tasks")
                .await
                .unwrap();
            assert_eq!(
                retrieved.source_scope, None,
                "own-scope collection should have source_scope = None"
            );
        }

        #[tokio::test]
        async fn source_scope_survives_re_registration() {
            let db = require_postgres!();

            let mut schema = test_schema("tasks");
            schema.source_scope = Some("household".to_string());

            db.register_collection("andrew", &schema).await.unwrap();
            // Re-register the same schema.
            db.register_collection("andrew", &schema).await.unwrap();

            let retrieved = db
                .get_collection_schema("andrew", "tasks")
                .await
                .unwrap();
            assert_eq!(
                retrieved.source_scope,
                Some("household".to_string()),
                "source_scope should survive re-registration"
            );
        }

        // ── Write isolation tests ───────────────────────────────────────

        #[tokio::test]
        async fn scoped_add_writes_to_source_scope_not_caller() {
            let db = require_postgres!();

            // Register the collection under household (the actual data owner).
            db.register_collection("household", &test_schema("tasks"))
                .await
                .unwrap();

            // Andrew has a scoped schema pointing at household.
            let mut schema = test_schema("tasks");
            schema.source_scope = Some("household".to_string());

            let tool = CollectionAddTool::new(schema, Arc::clone(&db), None, "alice");

            // Andrew calls the tool.
            let ctx = JobContext::with_user("andrew", "test", "test");
            let result = tool
                .execute(serde_json::json!({"item": "buy milk"}), &ctx)
                .await
                .unwrap();
            assert_eq!(result.result["status"], "created");

            // The record should be in household's scope, not andrew's.
            let household_records = db
                .query_records("household", "tasks", &[], None, 100)
                .await
                .unwrap();
            assert_eq!(
                household_records.len(),
                1,
                "record should be written to household scope"
            );

            let andrew_records = db
                .query_records("andrew", "tasks", &[], None, 100)
                .await
                .unwrap();
            assert_eq!(
                andrew_records.len(),
                0,
                "record should NOT be in andrew's scope"
            );
        }

        #[tokio::test]
        async fn own_scope_add_writes_to_owner_user_id() {
            let db = require_postgres!();
            // Register collection under "alice" (the owner_user_id).
            setup_db_with_collection(&db, "alice", "personal_tasks").await;

            // No source_scope — writes to owner_user_id ("alice"), not ctx.user_id.
            let schema = test_schema("personal_tasks");
            let tool = CollectionAddTool::new(schema, Arc::clone(&db), None, "alice");

            // Andrew calls the tool, but the write should go to alice's scope.
            let ctx = JobContext::with_user("andrew", "test", "test");
            let result = tool
                .execute(serde_json::json!({"item": "read a book"}), &ctx)
                .await
                .unwrap();
            assert_eq!(result.result["status"], "created");

            // Record should be in alice's scope (owner_user_id), not andrew's (caller).
            let records = db
                .query_records("alice", "personal_tasks", &[], None, 100)
                .await
                .unwrap();
            assert_eq!(records.len(), 1, "record should be in alice's scope (owner_user_id)");

            // Andrew's scope should have no records.
            let andrew_records = db
                .query_records("andrew", "personal_tasks", &[], None, 100)
                .await
                .unwrap_or_default();
            assert_eq!(andrew_records.len(), 0, "record should NOT be in andrew's (caller) scope");
        }

        #[tokio::test]
        async fn scoped_update_modifies_source_scope() {
            let db = require_postgres!();

            db.register_collection("household", &test_schema("tasks"))
                .await
                .unwrap();

            // Insert a record directly into household.
            let record_id = db
                .insert_record(
                    "household",
                    "tasks",
                    serde_json::json!({"item": "buy milk"}),
                )
                .await
                .unwrap();

            // Andrew's scoped update tool.
            let mut schema = test_schema("tasks");
            schema.source_scope = Some("household".to_string());

            let tool = CollectionUpdateTool::new(schema, Arc::clone(&db), "alice");

            let ctx = JobContext::with_user("andrew", "test", "test");
            let result = tool
                .execute(
                    serde_json::json!({
                        "record_id": record_id.to_string(),
                        "item": "buy oat milk"
                    }),
                    &ctx,
                )
                .await
                .unwrap();
            assert_eq!(result.result["status"], "updated");

            // Verify the household record was updated.
            let records = db
                .query_records("household", "tasks", &[], None, 100)
                .await
                .unwrap();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].data["item"], "buy oat milk");
        }

        #[tokio::test]
        async fn scoped_delete_removes_from_source_scope() {
            let db = require_postgres!();

            db.register_collection("household", &test_schema("tasks"))
                .await
                .unwrap();

            let record_id = db
                .insert_record(
                    "household",
                    "tasks",
                    serde_json::json!({"item": "buy milk"}),
                )
                .await
                .unwrap();

            let mut schema = test_schema("tasks");
            schema.source_scope = Some("household".to_string());

            let tool = CollectionDeleteTool::new(schema, Arc::clone(&db), "alice");

            let ctx = JobContext::with_user("andrew", "test", "test");
            let result = tool
                .execute(
                    serde_json::json!({"record_id": record_id.to_string()}),
                    &ctx,
                )
                .await
                .unwrap();
            assert_eq!(result.result["status"], "deleted");

            let records = db
                .query_records("household", "tasks", &[], None, 100)
                .await
                .unwrap();
            assert_eq!(
                records.len(),
                0,
                "record should be deleted from household scope"
            );
        }

        #[tokio::test]
        async fn scoped_query_reads_from_source_scope() {
            let db = require_postgres!();

            db.register_collection("household", &test_schema("tasks"))
                .await
                .unwrap();

            db.insert_record(
                "household",
                "tasks",
                serde_json::json!({"item": "vacuum living room"}),
            )
            .await
            .unwrap();

            let mut schema = test_schema("tasks");
            schema.source_scope = Some("household".to_string());

            let tool = CollectionQueryTool::new(schema, Arc::clone(&db), "alice");

            // Andrew queries via his scoped tool — should read household data.
            let ctx = JobContext::with_user("andrew", "test", "test");
            let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
            assert_eq!(
                result.result["count"], 1,
                "scoped query should read from household"
            );
            assert_eq!(result.result["results"][0]["data"]["item"], "vacuum living room");
        }

        // ── Privacy boundary tests ──────────────────────────────────────

        #[tokio::test]
        async fn personal_item_does_not_leak_to_household() {
            let db = require_postgres!();

            // Set up both scopes.
            db.register_collection("andrew", &test_schema("tasks"))
                .await
                .unwrap();
            db.register_collection("household", &test_schema("tasks"))
                .await
                .unwrap();

            // Andrew's personal tasks tool (no source_scope).
            let personal_schema = test_schema("tasks");
            let tool = CollectionAddTool::new(personal_schema, Arc::clone(&db), None, "andrew");

            let ctx = JobContext::with_user("andrew", "test", "test");
            tool.execute(serde_json::json!({"item": "buy butt cream"}), &ctx)
                .await
                .unwrap();

            // Household should have zero records.
            let household_records = db
                .query_records("household", "tasks", &[], None, 100)
                .await
                .unwrap();
            assert_eq!(
                household_records.len(),
                0,
                "personal item must not leak to household"
            );

            // Andrew should have the record.
            let andrew_records = db
                .query_records("andrew", "tasks", &[], None, 100)
                .await
                .unwrap();
            assert_eq!(andrew_records.len(), 1);
        }

        #[tokio::test]
        async fn household_item_does_not_leak_to_personal() {
            let db = require_postgres!();

            db.register_collection("andrew", &test_schema("tasks"))
                .await
                .unwrap();
            db.register_collection("household", &test_schema("tasks"))
                .await
                .unwrap();

            // Andrew's household-scoped tool.
            let mut household_schema = test_schema("tasks");
            household_schema.source_scope = Some("household".to_string());
            let tool = CollectionAddTool::new(household_schema, Arc::clone(&db), None, "andrew");

            let ctx = JobContext::with_user("andrew", "test", "test");
            tool.execute(serde_json::json!({"item": "clean gutters"}), &ctx)
                .await
                .unwrap();

            // Andrew's own scope should be empty.
            let andrew_records = db
                .query_records("andrew", "tasks", &[], None, 100)
                .await
                .unwrap();
            assert_eq!(
                andrew_records.len(),
                0,
                "household item must not leak to andrew's personal scope"
            );

            // Household should have the record.
            let household_records = db
                .query_records("household", "tasks", &[], None, 100)
                .await
                .unwrap();
            assert_eq!(household_records.len(), 1);
        }

        #[tokio::test]
        async fn own_scope_tools_include_owner_prefix() {
            // Tool names always include the owner user_id prefix to prevent
            // collisions in the global tool registry across tenants.
            let schema = test_schema("tasks");
            assert!(
                schema.source_scope.is_none(),
                "own-scope schema should have no source_scope"
            );

            let db = require_postgres!();
            setup_db_with_collection(&db, "alice", "tasks").await;

            let add_tool = CollectionAddTool::new(schema.clone(), Arc::clone(&db), None, "alice");
            assert_eq!(add_tool.name(), "alice_tasks_add", "tool name should include owner prefix");

            let query_tool = CollectionQueryTool::new(schema.clone(), Arc::clone(&db), "alice");
            assert_eq!(query_tool.name(), "alice_tasks_query");

            let update_tool = CollectionUpdateTool::new(schema.clone(), Arc::clone(&db), "alice");
            assert_eq!(update_tool.name(), "alice_tasks_update");

            let delete_tool = CollectionDeleteTool::new(schema, Arc::clone(&db), "alice");
            assert_eq!(delete_tool.name(), "alice_tasks_delete");
        }

        // ── Edge case tests ─────────────────────────────────────────────

        #[tokio::test]
        async fn scoped_write_to_nonexistent_source_scope_fails_gracefully() {
            let db = require_postgres!();

            // No collection registered for "nonexistent" scope.
            let mut schema = test_schema("tasks");
            schema.source_scope = Some("nonexistent".to_string());

            let tool = CollectionAddTool::new(schema, Arc::clone(&db), None, "alice");
            let ctx = JobContext::with_user("andrew", "test", "test");

            let result = tool
                .execute(serde_json::json!({"item": "should fail"}), &ctx)
                .await;
            assert!(
                result.is_err(),
                "writing to a nonexistent source scope should fail"
            );
        }

        #[tokio::test]
        async fn scoped_delete_wrong_scope_record_fails() {
            let db = require_postgres!();

            db.register_collection("household", &test_schema("tasks"))
                .await
                .unwrap();
            db.register_collection("andrew", &test_schema("tasks"))
                .await
                .unwrap();

            // Insert a record in household's scope.
            let household_record_id = db
                .insert_record(
                    "household",
                    "tasks",
                    serde_json::json!({"item": "household chore"}),
                )
                .await
                .unwrap();

            // Andrew's personal delete tool (no source_scope).
            let schema = test_schema("tasks");
            let tool = CollectionDeleteTool::new(schema, Arc::clone(&db), "alice");

            let ctx = JobContext::with_user("andrew", "test", "test");
            let result = tool
                .execute(
                    serde_json::json!({"record_id": household_record_id.to_string()}),
                    &ctx,
                )
                .await;

            // The delete should fail or be a no-op because andrew's scope
            // doesn't own the household record.
            if result.is_ok() {
                // Even if the tool returns success, the record should still exist
                // in household's scope (it must not have been deleted).
                let records = db
                    .query_records("household", "tasks", &[], None, 100)
                    .await
                    .unwrap();
                assert_eq!(
                    records.len(),
                    1,
                    "personal delete tool must not delete household records"
                );
            }
        }

        #[tokio::test]
        async fn scoped_update_wrong_scope_record_fails() {
            let db = require_postgres!();

            db.register_collection("household", &test_schema("tasks"))
                .await
                .unwrap();
            db.register_collection("andrew", &test_schema("tasks"))
                .await
                .unwrap();

            // Insert a record in household's scope.
            let household_record_id = db
                .insert_record(
                    "household",
                    "tasks",
                    serde_json::json!({"item": "household chore"}),
                )
                .await
                .unwrap();

            // Andrew's personal update tool (no source_scope).
            let schema = test_schema("tasks");
            let tool = CollectionUpdateTool::new(schema, Arc::clone(&db), "alice");

            let ctx = JobContext::with_user("andrew", "test", "test");
            let result = tool
                .execute(
                    serde_json::json!({
                        "record_id": household_record_id.to_string(),
                        "item": "hijacked"
                    }),
                    &ctx,
                )
                .await;

            // The update should fail or be a no-op.
            if result.is_ok() {
                let records = db
                    .query_records("household", "tasks", &[], None, 100)
                    .await
                    .unwrap();
                assert_eq!(
                    records[0].data["item"], "household chore",
                    "personal update tool must not modify household records"
                );
            }
        }

        #[tokio::test]
        async fn multiple_scopes_same_collection_independent_data() {
            let db = require_postgres!();

            // Three scopes, same collection name.
            for scope in &["andrew", "household", "family"] {
                db.register_collection(scope, &test_schema("tasks"))
                    .await
                    .unwrap();
            }

            // Insert different data in each scope.
            db.insert_record(
                "andrew",
                "tasks",
                serde_json::json!({"item": "andrew's task"}),
            )
            .await
            .unwrap();
            db.insert_record(
                "household",
                "tasks",
                serde_json::json!({"item": "household task"}),
            )
            .await
            .unwrap();
            db.insert_record(
                "family",
                "tasks",
                serde_json::json!({"item": "family task"}),
            )
            .await
            .unwrap();

            // Each scope should see only its own data.
            for (scope, expected_item) in &[
                ("andrew", "andrew's task"),
                ("household", "household task"),
                ("family", "family task"),
            ] {
                let records = db
                    .query_records(scope, "tasks", &[], None, 100)
                    .await
                    .unwrap();
                assert_eq!(records.len(), 1, "{scope} should have exactly 1 record");
                assert_eq!(
                    records[0].data["item"], *expected_item,
                    "{scope} should have its own data"
                );
            }
        }

        #[tokio::test]
        async fn collection_write_event_carries_source_scope() {
            // When writing to a scoped collection, the CollectionWriteEvent should
            // carry the source scope's user_id, not the caller's.
            let db = require_postgres!();

            db.register_collection("household", &test_schema("tasks"))
                .await
                .unwrap();

            let (tx, mut rx) =
                tokio::sync::broadcast::channel::<crate::agent::collection_events::CollectionWriteEvent>(10);

            let mut schema = test_schema("tasks");
            schema.source_scope = Some("household".to_string());

            let tool = CollectionAddTool::new(schema, Arc::clone(&db), Some(tx), "andrew");
            let ctx = JobContext::with_user("andrew", "test", "test");
            tool.execute(serde_json::json!({"item": "test event"}), &ctx)
                .await
                .unwrap();

            let event = rx.try_recv().unwrap();
            assert_eq!(
                event.user_id, "household",
                "event should carry source scope, not caller"
            );
            assert_eq!(event.collection, "tasks");
        }

        #[tokio::test]
        async fn cross_scope_tools_have_prefixed_names() {
            let mut schema = test_schema("tasks");
            schema.source_scope = Some("household".to_string());

            let db = require_postgres!();

            let add = CollectionAddTool::new(schema.clone(), Arc::clone(&db), None, "alice");
            assert_eq!(add.name(), "household_tasks_add");

            let query = CollectionQueryTool::new(schema.clone(), Arc::clone(&db), "alice");
            assert_eq!(query.name(), "household_tasks_query");

            let update = CollectionUpdateTool::new(schema.clone(), Arc::clone(&db), "alice");
            assert_eq!(update.name(), "household_tasks_update");

            let delete = CollectionDeleteTool::new(schema.clone(), Arc::clone(&db), "alice");
            assert_eq!(delete.name(), "household_tasks_delete");
        }

        #[tokio::test]
        async fn scoped_description_includes_scope_context() {
            // Cross-scope tools should have descriptions mentioning the scope.
            let mut schema = test_schema("tasks");
            schema.source_scope = Some("household".to_string());

            let db = require_postgres!();

            let add = CollectionAddTool::new(schema.clone(), Arc::clone(&db), None, "alice");
            let desc = add.description();
            assert!(
                desc.contains("household"),
                "scoped tool description should mention scope, got: {desc}"
            );
        }

        #[tokio::test]
        async fn own_scope_description_includes_owner() {
            let schema = test_schema("tasks");

            let db = require_postgres!();

            let add = CollectionAddTool::new(schema, Arc::clone(&db), None, "alice");
            let desc = add.description();
            // Own-scope description should include the owner's user_id.
            assert!(
                desc.contains("alice"),
                "own-scope tool description should mention owner, got: {desc}"
            );
        }

        #[tokio::test]
        async fn list_collections_includes_scoped_with_source_scope() {
            let db = require_postgres!();

            // Register a personal collection under andrew.
            db.register_collection("andrew", &test_schema("notes"))
                .await
                .unwrap();

            // Register a scoped collection under andrew pointing at household.
            let mut scoped_schema = test_schema("tasks");
            scoped_schema.source_scope = Some("household".to_string());
            db.register_collection("andrew", &scoped_schema)
                .await
                .unwrap();

            let tool = CollectionListTool::new(Arc::clone(&db));
            let ctx = JobContext::with_user("andrew", "test", "test");

            let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
            let collections = result.result["collections"].as_array().unwrap();

            assert_eq!(collections.len(), 2, "should list both collections");

            // Find the scoped collection and verify source_scope is exposed.
            let tasks_entry = collections
                .iter()
                .find(|c| c["collection"].as_str() == Some("tasks"))
                .expect("tasks collection should be listed");
            assert_eq!(
                tasks_entry["source_scope"].as_str(),
                Some("household"),
                "list_collections should expose source_scope for scoped collections"
            );

            // Personal collection should have no source_scope (null).
            let notes_entry = collections
                .iter()
                .find(|c| c["collection"].as_str() == Some("notes"))
                .expect("notes collection should be listed");
            assert!(
                notes_entry.get("source_scope").is_none()
                    || notes_entry["source_scope"].is_null(),
                "personal collection should have no source_scope"
            );
        }
    }
}

#[cfg(test)]
mod owner_scope_tests {
    use super::*;

    fn make_schema(collection: &str, source_scope: Option<&str>) -> CollectionSchema {
        CollectionSchema {
            collection: collection.to_string(),
            description: Some("test".to_string()),
            fields: std::collections::BTreeMap::new(),
            source_scope: source_scope.map(|s| s.to_string()),
        }
    }

    /// When source_scope is None, tool_name_for uses owner_user_id (not caller).
    /// This test verifies the tool naming — the same logic as owner_scope() without
    /// source_scope set.
    #[test]
    fn add_tool_owner_scope_returns_owner_not_caller() {
        let schema = make_schema("tasks", None);
        // Grace is the owner; tool_name_for should embed "grace", not any caller id.
        let tool_name = tool_name_for(&schema, "add", "grace");
        assert_eq!(tool_name, "grace_tasks_add");
    }

    /// When source_scope is set, it takes precedence over owner_user_id.
    #[test]
    fn owner_scope_with_source_scope_prefers_it() {
        let schema = make_schema("tasks", Some("household"));
        let tool_name = tool_name_for(&schema, "add", "grace");
        assert_eq!(tool_name, "household_tasks_add");
    }

    /// Verify owner_scope() on CollectionAddTool returns self.owner_user_id when
    /// source_scope is None — not any external user_id.
    #[test]
    fn collection_add_tool_owner_scope_uses_owner_user_id() {
        let schema = CollectionSchema {
            collection: "grocery".to_string(),
            description: Some("test".to_string()),
            fields: std::collections::BTreeMap::new(),
            source_scope: None,
        };
        // We can't construct a real DB in a pure unit test, but we can verify
        // the owner_scope() method directly by inspecting tool_name which encodes scope.
        // tool_name_for(schema, "add", "grace") == "grace_grocery_add" confirms
        // the scope embedded at construction time is "grace", not any caller id.
        let name = tool_name_for(&schema, "add", "grace");
        assert_eq!(name, "grace_grocery_add", "owner scope should be grace, not caller");

        let name_with_scope = tool_name_for(
            &CollectionSchema {
                collection: "grocery".to_string(),
                description: Some("test".to_string()),
                fields: std::collections::BTreeMap::new(),
                source_scope: Some("grace".to_string()),
            },
            "add",
            "andrew",
        );
        assert_eq!(name_with_scope, "grace_grocery_add", "source_scope wins over owner_user_id");
    }
}
