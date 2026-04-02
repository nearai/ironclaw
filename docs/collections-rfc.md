# Collections: Typed Structured Storage for Agent Workspaces

## Summary

"Add milk to the grocery list." This is the simplest thing a user expects a personal assistant to do, and IronClaw can't do it. The agent either creates a new document every time (fragmenting the list) or tries to edit an existing markdown file and corrupts it. Across 28 test scenarios and 2 models, modifying structured data stored in memory documents fails **every time**.

This PR adds schema-defined collections with auto-generated typed CRUD tools. Register a collection with a schema, get a tool that handles inserts, queries, updates, deletes, and aggregations with validation.

| Model | Collections | Memory docs | Delta |
|-------|:-----------:|:-----------:|:-----:|
| Qwen 3.5-35B (local) | **76%** | 37% | **+39** |
| Claude Haiku 4.5 | **70%** | 26% | **+44** |

35 files changed, ~12,600 lines. Dual-backend (PostgreSQL + libSQL), per-user tool isolation. Also provides the storage layer for #1474.

## The problem

A user says "add milk to the grocery list." Both options fail:

1. **Write a new document.** Creates fragments. "What's on the list?" requires deduplication across scattered files.

2. **Edit an existing document.** Read, parse, modify, write back. Models can't do this — they lose items, duplicate entries, or corrupt formatting. Fails on every model tested.

The problem is structural: append-only documents don't support mutation.

## What a user sees

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

This generates a tool called `grocery_items`. The model calls:

```
grocery_items(operation: "add", data: { "name": "milk", "category": "dairy" })
grocery_items(operation: "query")
grocery_items(operation: "summary", agg_operation: "count")
```

The schema validates inputs and coerces LLM quirks: "12" becomes 12, "true" becomes true, "tomorrow" becomes an ISO date.

## Evidence

### Collections vs memory docs (28 scenarios, same data, same models)

| Category | Qwen+Coll | Qwen+Mem | Haiku+Coll | Haiku+Mem |
|----------|:---------:|:--------:|:----------:|:---------:|
| Grocery (7) | **76%** | 29% | **76%** | 28% |
| Nanny hours (7) | 56% | 21% | **69%** | 24% |
| Todo (6) | **72%** | 52% | **72%** | 35% |
| Transactions (8) | 59% | 48% | **65%** | 21% |

### Tool design matters

| Approach | Score | Why |
|----------|:-----:|-----|
| 1 unified tool/collection + skills | **76%** | Low tool count + guided operations |
| 5 per-operation tools + skills | 65% | Self-documenting but 20 tools adds noise |
| 1 unified tool/collection, no skills | 68% | Model handles the operation enum fine |
| Flat files (memory docs) | 37% | Writes broken |
| Generic CRUD (5 tools for all) | 41% | Model forgets collection names |

One tool per collection with an `operation` parameter is the best design. Auto-generated skills (+8%) teach the model which operation to use for natural language intents.

## How it works

### Storage

`StructuredStore` trait, fully implemented for both PostgreSQL (JSONB) and libSQL (`json_extract`). Records in a shared `structured_records` table, discriminated by `(user_id, collection)` with composite index + GIN index. 7 field types: Text, Number, Date, Time, DateTime, Bool, Enum.

Schema alteration supports adding/removing fields and enum values. Existing records are preserved — queries, filters, and aggregations handle missing fields correctly (SUM/AVG skip records where the field doesn't exist).

### Tools

Each collection gets one tool named `{user}_{collection}` with parameters:

- `operation` (required enum: query, add, update, delete, summary)
- `data` (object — validated against the collection's field schema)
- `record_id` (string — for update/delete)
- `filters` (object — field → {op, value}, supports eq/neq/gt/gte/lt/lte/is_null/is_not_null)
- `field`, `agg_operation`, `group_by` (for summary aggregations)

Tool names include the owner prefix; the dispatcher filters per-user via `tool_definitions_for_user()`.

When a collection is registered, a SKILL.md is auto-generated with activation keywords from the schema. Keyword/regex matching injects the skill into the system prompt when relevant. On restart, existing schemas are loaded and tools registered before the first conversation.

### REST API

6 endpoints for external integration (webhooks, cron, scripts). Inherits gateway auth.

- `GET/POST /api/collections` — List / register schemas
- `GET/POST /api/collections/{name}/records` — Query / insert records
- `PUT/DELETE /api/collections/{name}/records/{id}` — Update / delete records

## Multi-tenant scoping

Collections are scoped by `user_id`. Every query includes `WHERE user_id = $1`. Tool definitions are filtered per-user in the dispatcher, job workers, and routine engine.

For cross-user read access, `source_scope` allows a tool to query another user's data. `source_scope` is stripped in `CollectionRegisterTool::execute()` and `collections_register_handler` — only server-side seeding can set it.

## Tool scaling

Each collection adds 1 tool. 20 collections = 20 tools, ~300 extra tokens with compressed descriptions. Per-user filtering and on-demand skill injection keep prompts lean.

## Drawbacks

- Adds a new storage abstraction alongside memory documents. Zero impact if unused.
- Fields must be defined upfront. Adding a required field doesn't backfill old records.
- Delete by description succeeds ~40%. Record IDs in query results make delete-by-ID reliable.

## Alternatives considered

- **Improve document-based search**: writes remain 0%.
- **External database/service**: breaks single-deployment model.
- **Generic CRUD tools**: 41%. Model forgets collection names as parameters.
- **5 per-operation tools per collection**: 65%. Scales poorly (50 tools at 10 collections).

## Compatibility

Additive only. No existing tables modified, no existing tool behavior changed, no changes to memory documents. V16 migration adds two new tables (`structured_schemas`, `structured_records`). `owner_user_id()` added to Tool trait with default `None` — existing tools unaffected. Both backends implemented and tested at parity. Set `COLLECTION_TOOL_MODE=unified` to enable (recommended).
