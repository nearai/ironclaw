# Collections: Typed Structured Storage for Agent Workspaces

## Summary

"Add milk to the grocery list." This is the simplest thing a user expects a personal assistant to do, and IronClaw can't do it. The agent either creates a new document every time (fragmenting the list) or tries to edit an existing markdown file and corrupts it. Across 28 test scenarios and 2 models, modifying structured data stored in memory documents (add to a list, update a record, remove an item) fails **every time**. Read-only queries partially work but aggregation and filtering don't.

This PR adds schema-defined collections with auto-generated typed CRUD tools. When a collection is registered with a schema, the system generates a tool that handles inserts, queries, updates, deletes, and aggregations with schema validation. No markdown parsing, no read-modify-write, no corruption.

| Model | Collections | Memory docs | Delta |
|-------|:-----------:|:-----------:|:-----:|
| Qwen 3.5-35B (local) | **76%** | 37% | **+39** |
| Claude Haiku 4.5 | **70%** | 26% | **+44** |

35 files changed, ~12,600 lines. Full dual-backend support (PostgreSQL + libSQL), per-user tool isolation, 168 tests. Also provides the storage layer needed for #1474 (auto-extract structured memories from conversations).

## The problem

A user says "add milk to the grocery list." Today the agent has two options, and both fail:

1. **Write a new document.** `memory_write("grocery list: milk")`. Next time someone says "add eggs," the agent writes another document. Now there are two documents, no unified list, and "what's on the grocery list?" requires searching and deduplicating fragments.

2. **Edit an existing document.** Read the "grocery list" document, parse it, add the item, and write it back. Models can't do this. The model either rewrites the entire document (losing items), appends a duplicate, or corrupts the format. This fails on every model we tested: Qwen 3.5, Haiku 4.5, and three LoRA fine-tunes.

I tested 11 approaches including search hints, proactive RAG, and LoRA fine-tuning. The problem is structural: append-only documents don't support mutation.

## What a user sees

Register a collection:

```json
POST /api/collections
{
  "collection": "grocery_items",
  "description": "Shopping list",
  "fields": {
    "name":     { "type": "text", "required": true },
    "quantity": { "type": "number" },
    "category": { "type": { "enum": ["produce", "dairy", "meat", "bakery", "other"] } },
    "store":    { "type": "text" },
    "done":     { "type": "bool", "default": false }
  }
}
```

This generates a tool called `grocery_items` with an `operation` parameter. The user says "add milk" and the model calls:

```json
grocery_items(operation: "add", data: { "name": "milk", "category": "dairy" })
```

The user says "what's on the list?" and the model calls:

```json
grocery_items(operation: "query")
```

The user says "how many items do we need?" and the model calls:

```json
grocery_items(operation: "summary", agg_operation: "count")
```

No markdown, no parsing, no corruption. The schema validates inputs, coerces types (string "2" becomes number 2), and handles LLM quirks (natural language dates like "tomorrow" are converted to ISO format).

## Evidence

### Collections vs memory docs (28 scenarios, same data, same models)

| Category | Qwen+Coll | Qwen+Mem | Haiku+Coll | Haiku+Mem |
|----------|:---------:|:--------:|:----------:|:---------:|
| Grocery (7) | **76%** | 29% | **76%** | 28% |
| Nanny hours (7) | 56% | 21% | **69%** | 24% |
| Todo (6) | **72%** | 52% | **72%** | 35% |
| Transactions (8) | 59% | 48% | **65%** | 21% |

Grocery achieves parity across model sizes (both 76%). This is the simplest category (list management). Complex categories like transactions still show a model quality gap but collections make them functional where memory docs scored near zero on writes.

### Tool design matters

| Approach | Score | Why |
|----------|:-----:|-----|
| 1 unified tool/collection + skills | **76%** | Best: low tool count + guided operations |
| 5 per-operation tools + skills | 65% | Tool name is self-documenting but 20 tools adds noise |
| 1 unified tool/collection, no skills | 68% | Model handles the operation enum without guidance |
| Collections, no hints | 51% | No discovery guidance |
| Flat files (memory docs) | 37% | Writes broken |
| Generic CRUD (5 tools for all) | 41% | Model forgets collection names |

One tool per collection with an `operation` parameter is the best design. Fewer tools means less prompt noise. Auto-generated skills (+8% over no-skills) teach the model which operation to use for natural language intents.

## How it works

### Storage

`StructuredStore` trait (10 async methods), fully implemented for both PostgreSQL (JSONB operators) and libSQL (`json_extract`). Records stored as JSONB in a shared `structured_records` table, discriminated by `(user_id, collection)`. 7 field types: `Text`, `Number`, `Date`, `Time`, `DateTime`, `Bool`, `Enum{values}`.

Validation coerces LLM quirks: "12" → 12, "true" → true, "tomorrow" → 2026-04-03. Schema alteration supports adding/removing fields and enum values without migrating existing records. Composite index on `(user_id, collection, created_at)` plus GIN index on JSONB data.

### Tools

Each collection gets one tool named `{user}_{collection}` with an `operation` enum (query, add, update, delete, summary) plus typed parameters for data, filters, and aggregations. Tool names include the owner prefix; the dispatcher filters per-user via `tool_definitions_for_user()`.

When a collection is registered, a SKILL.md is auto-generated with activation keywords from the schema (name, description, field names, enum values). Keyword/regex matching injects the skill into the system prompt when relevant — no LLM call. Skills teach the model which operation to use: "mark done" → `operation: "update"`.

On restart, existing schemas are loaded and tools registered before the first conversation.

### REST API

6 endpoints for external integration (webhooks, cron, scripts). Inherits gateway auth.

- `GET/POST /api/collections` — List / register schemas
- `GET/POST /api/collections/{name}/records` — Query / insert records
- `PUT/DELETE /api/collections/{name}/records/{id}` — Update / delete records

## Multi-tenant scoping

Collections are scoped by `user_id`, same as existing memory isolation. Every query includes `WHERE user_id = $1`. Tool definitions are filtered per-user in the dispatcher, job workers, and routine engine.

For cross-user read access (e.g., a shared household list), `source_scope` allows a tool to query another user's data. `source_scope` is stripped in two places: `CollectionRegisterTool::execute()` sets `schema.source_scope = None` after deserializing LLM params, and `collections_register_handler` does the same for REST API input. Only server-side seeding (direct `db.register_collection()` calls) can set `source_scope`.

## Tool scaling

Each collection adds 1 tool. At 10 collections, that's 10 tools. With compressed descriptions (~15 tokens each), 20 collections add ~300 tokens to the prompt.

Per-user filtering ensures each tenant only sees their own tools. Auto-generated skills inject collection context on demand rather than always.

## Test coverage

168 tests across both backends:

- 88 unit tests — schema validation, field types, coercion, alteration, history capping, natural language dates
- 39 integration tests — CRUD, all filter operators, all aggregation types, pagination, empty sets, non-existent resources, multi-user data isolation
- 13 per-operation tool tests — generation, typed parameters, user isolation, registry filtering, collision prevention, drop cleanup
- 18 unified tool tests — all operations, validation, error cases, user isolation
- 10 skill generation tests — SKILL.md output, router skill, edge cases

## Drawbacks

- Adds a new storage abstraction alongside memory documents. Zero impact if unused — the feature is additive.
- Schema rigidity: fields must be defined upfront. Schema evolution is handled via alteration (add/remove fields), not migration. Adding a required field to an existing collection doesn't backfill old records.
- Delete by description succeeds ~40% of the time. The model struggles to identify which record to remove without seeing IDs. Record IDs in query results make delete-by-ID reliable.
- Skill keyword extraction includes some hardcoded domain synonyms ("eggs" → grocery). These are hints, not logic — they affect skill activation scoring, not query execution. Deployments with different domains would get keywords from their own field names and enum values.

## Alternatives considered

- **Improve document-based search**: best variant hit 71% on reads, writes remain 0%. Can't search-hint your way out of a data model mismatch.
- **Convention-based structure in documents**: fragile formatting, no query semantics, concurrent updates collide.
- **External database/service**: breaks single-deployment model.
- **Generic CRUD tools (5 tools for all collections)**: tested at 41%. Model forgets collection names when they're parameters instead of tool names.
- **5 per-operation tools per collection**: tested at 65%. Works but scales poorly (50 tools at 10 collections). Unified tool mode is simpler and scores higher.

## Compatibility

- **Additive only.** No existing behavior is changed. Collections are a new feature that coexists with memory documents.
- **Database migration**: V16 adds `structured_schemas` and `structured_records` tables. No changes to existing tables.
- **Tool trait**: `owner_user_id()` added with default `None`. No existing tool implementations affected.
- **Feature flag**: set `COLLECTION_TOOL_MODE=unified` to enable. Default behavior is per-operation mode (5 tools/collection) for backward compatibility. Unified mode is recommended for new deployments.

## Future work

- **Computed fields**: cross-field expressions (hours × rate) currently require the LLM to do arithmetic.
- **Schema versioning**: no rollback mechanism if a schema alteration breaks existing records.
- **Export formats**: REST API returns JSON only.
