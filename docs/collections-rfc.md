# Collections: Typed Structured Storage for Agent Workspaces

## Summary

Agents need structured data operations (grocery lists, task trackers, shift logs) but the only storage primitive is append-only memory documents. Write operations against markdown documents succeed 0% of the time across every model tested. This adds schema-defined collections with auto-generated typed CRUD tools, dual-backend support (PostgreSQL + libSQL), and per-user tool isolation.

28-scenario benchmark across 4 categories (grocery, nanny hours, todo, transactions):

| Model | Collections | Memory docs | Delta |
|-------|:-----------:|:-----------:|:-----:|
| Qwen 3.5-35B (local) | **65%** | 37% | **+28** |
| Claude Haiku 4.5 | **70%** | 26% | **+44** |

A unified tool mode (1 tool per collection instead of 5) with auto-generated skill injection scores **73%** — the best configuration tested.

This also provides the storage layer needed for #1474 (auto-extract structured memories from conversations).

## The problem

A user says "add milk to the grocery list." Today the agent has two options:

1. Write a new memory document ("grocery list: milk"). Next time someone says "add eggs," the agent writes another document. Now there are two documents, no unified list, and "what's on the grocery list?" requires searching, deduplicating, and hoping the model can synthesize a coherent answer.

2. Read an existing "grocery list" document, parse it, add the item, and write it back. Models can't do this reliably. The model either rewrites the entire document (losing items), appends a duplicate, or corrupts the format.

I tested 11 approaches including search hints, proactive RAG, and LoRA fine-tuning. The problem is structural: append-only documents don't support mutation.

## Evidence

### Collections vs memory docs (28 scenarios, upstream build)

| Category | Qwen+Coll | Qwen+Mem | Haiku+Coll | Haiku+Mem |
|----------|:---------:|:--------:|:----------:|:---------:|
| Grocery (7) | **76%** | 29% | **76%** | 28% |
| Nanny hours (7) | 56% | 21% | **69%** | 24% |
| Todo (6) | **72%** | 52% | **72%** | 35% |
| Transactions (8) | 59% | 48% | **65%** | 21% |

Grocery achieves parity across model sizes (76% for both). Structured tools normalize the interface, reducing the importance of model-specific text manipulation ability.

### Why per-collection tools beat alternatives

| Approach | Success | Why |
|----------|---------|-----|
| Per-collection typed tools + hints | 83% | Tool name is the discovery mechanism |
| Proactive RAG | 71% | Embedding mismatch on some queries |
| Skill injection only | 69% | Missed synonyms |
| Baseline collections (no hints) | 51% | No discovery guidance |
| Flat files | 46% | Writes broken |
| Generic CRUD (5 tools) | 41% | Model forgets collection names |

`grocery_items_add` is self-documenting. The model doesn't need to remember which collection to target. Generic CRUD (`collection_query("grocery_items")`) drops accuracy by half because the model forgets or misspells the collection name.

## How it works

```
POST /api/collections (schema JSON)
  → db.register_collection() persists to structured_schemas table
  → generate_collection_tools() creates 5 Tool instances
  → registry.register(tool) makes them available to the agent loop
  → model calls grocery_items_add(item: "milk", category: "dairy")
  → CollectionAddTool.execute() validates against schema, inserts record
  → CollectionWriteEvent fires on broadcast channel (routine triggers)
```

### Schema and validation

`StructuredStore` trait (10 async methods) with full implementations for both PostgreSQL and libSQL. 7 field types: `Text`, `Number`, `Date`, `Time`, `DateTime`, `Bool`, `Enum{values}`. Validation includes LLM-friendly coercion: numbers-as-strings, booleans-as-strings, and natural language dates ("tomorrow" → ISO date) are handled automatically.

Schema alteration supports adding/removing fields and enum values. Existing records are preserved without migration.

### Per-collection tools

Two modes, controlled by `COLLECTION_TOOL_MODE`:

**Per-operation mode** (default): 5 typed tools per collection.

- `{user}_{collection}_add` — Insert with typed parameters from field definitions
- `{user}_{collection}_query` — Filter by any field (eq, neq, gt, gte, lt, lte, between, in, is_null, is_not_null), sort, limit
- `{user}_{collection}_update` — Partial update by record ID
- `{user}_{collection}_delete` — Delete by record ID
- `{user}_{collection}_summary` — Aggregations: sum, count, avg, min, max with optional group_by and filters

**Unified mode** (`COLLECTION_TOOL_MODE=unified`): 1 tool per collection with an `operation` parameter.

- `{user}_{collection}(operation: "query|add|update|delete|summary", data?, record_id?, filters?, field?, group_by?)`

Unified mode reduces tool count from 5N to N. With 10 collections, that's 10 tools instead of 50. Benchmarks show unified mode with skill injection scores 73% vs 65% for per-operation mode — fewer tools means less prompt noise and better tool selection.

| Mode | Tools (4 collections) | Score | Skill impact |
|------|:---------------------:|:-----:|:------------:|
| Per-operation + skills | 20 | 65% | +0 (skills don't help) |
| Unified, no skills | 4 | 68% | — |
| Unified + skills | 4 | **73%** | **+5** (skills teach operations) |

Skills help unified tools because the model needs guidance on which operation to use ("mark as done" → `operation: "update"`, `data: {status: "done"}`). With per-operation tools, the tool name IS the guidance (`_update` is self-explanatory), so skills add noise.

Tool names include the owner user_id prefix to prevent collisions in multi-tenant deployments. The `Tool` trait gains an `owner_user_id()` method, and the dispatcher filters tool definitions per-user so each tenant only sees their own collection tools.

### Startup initialization

On restart, `initialize_collection_tools_for_users()` loads all existing collection schemas from the database and registers their CRUD tools before the first conversation. Without this, collections created in prior sessions would have no tools available.

### What happens when a collection is registered

The full lifecycle, step by step:

**1. Schema persisted.** `db.register_collection(user_id, schema)` writes to `structured_schemas` (upsert on `(user_id, collection)` primary key). The schema JSON includes field names, types, required flags, defaults, and enum values.

**2. Five CRUD tools generated.** `generate_collection_tools(schema, db, owner_user_id)` creates tool instances with typed parameter schemas derived from the field definitions. A `Number` field becomes `"type": "number"`, an `Enum` becomes `"enum": ["value1", "value2"]`, required fields appear in `"required"`. The LLM sees exactly the right types and constraints without any generic "pass a JSON blob" interface.

**3. Tools registered in the shared registry.** Each tool is named `{owner}_{collection}_{op}` (e.g., `andrew_grocery_items_add`). The `owner_user_id()` trait method marks them as user-scoped, so the dispatcher only shows them to their owner.

**4. Per-collection SKILL.md generated and loaded.** A SKILL.md file is written to the skills directory with:
- **Activation keywords** extracted deterministically from the schema: collection name words, description words (minus stopwords), field names, enum values. Capped at 25 keywords.
- **Activation patterns** (regex): matches intent verbs ("add", "put", "show me", "how many"), question patterns ("what's", "what do"), and action patterns ("needs to", "have to").
- **Prompt content**: instructions telling the model which tools to call and how, with field-level documentation and example usage.

When a user message matches the keywords or patterns (scored by `prefilter_skills()` — no LLM call, pure keyword/regex scoring), the skill content is injected into the system prompt for that turn. This gives the model collection-specific guidance without bloating every turn's context.

**5. Collections-router skill updated.** A meta-skill listing all registered collections is regenerated. Its keywords are the union of all collection name words. This catches messages that mention collections generally ("what data do I have?") without matching any specific collection.

**6. Discovery doc written to workspace memory.** A markdown file at `collections/{name}.md` containing the schema description, domain synonyms (hardcoded mappings like "eggs" → grocery, "babysitter" → nanny), example queries, tool names, and field documentation. This is searchable via `memory_search` embeddings, providing a fallback discovery path when skill keyword matching misses.

**7. CollectionWriteEvent broadcast.** On insert/update/delete, a broadcast event fires so the routine engine can trigger collection-write routines (e.g., "when a new nanny shift is logged, notify the household chat").

### What happens when a user asks "what's on the grocery list?"

**1. Skill activation.** `prefilter_skills("what's on the grocery list?")` scores all loaded skills. The `grocery_items` skill matches on keyword "grocery" (10 points) and pattern `(?i)what('s|...)` (20 points). It's injected into the system prompt.

**2. Tool selection.** The system prompt now contains the skill's instructions: "Use `grocery_items_query` to search and filter records." The tool list includes `andrew_grocery_items_query` with a typed parameter schema. The model calls it.

**3. Query execution.** `CollectionQueryTool::execute()` receives the parameters, builds a `query_records(user_id, collection, filters, order_by, limit)` call, and returns the results as JSON.

**4. Response.** The model reads the query results and formats a natural language answer.

If the skill doesn't activate (e.g., user says "do we need eggs?" without saying "grocery"), the model may call `memory_search("eggs")`, which surfaces the discovery doc at `collections/grocery_items.md` via embedding similarity. The doc tells the model about the grocery collection and its tools. This is the fallback path.

### REST API

6 endpoints alongside the tool API. Not everything that writes structured data should go through the agent loop. Webhooks, cron jobs, and IoT triggers hit the HTTP endpoints directly.

- `GET /api/collections` — List schemas
- `POST /api/collections` — Register schema
- `GET /api/collections/{name}/records` — Query with filters
- `POST /api/collections/{name}/records` — Insert record
- `PUT /api/collections/{name}/records/{id}` — Update record
- `DELETE /api/collections/{name}/records/{id}` — Delete record

## Multi-tenant scoping

Collections are scoped by `user_id`, same as existing memory isolation. Each user's collections are invisible to other users at both the data layer (WHERE clauses) and the tool layer (`tool_definitions_for_user()` filtering in the dispatcher, job workers, and routine engine).

Tool names are prefixed with the owner: `andrew_grocery_items_query`, `grace_tasks_query`. This makes ownership explicit to the model and prevents registry collisions.

For cross-user read access (e.g. a shared household list), `source_scope` allows a tool to query another user's data. `source_scope` is stripped from untrusted inputs (LLM tool calls, REST API) and can only be set through trusted seeding paths.

## Tool scaling

In per-operation mode, each collection adds 5 tools. In unified mode, each adds 1. This is the primary scaling mitigation.

| Collections | Per-operation tools | Unified tools |
|:-----------:|:-------------------:|:-------------:|
| 5 | 25 | 5 |
| 10 | 50 | 10 |
| 20 | 100 | 20 |

Additional mitigations:

- **Compressed descriptions** (~15 tokens per tool): small models perform better with compressed (65%) than full JSON schemas (41%).
- **Per-user filtering**: each user only sees their own collection tools.
- **Skill-based discovery**: auto-generated SKILL.md files with keyword matching inject relevant collection context. Particularly effective with unified tools (+5% accuracy).

Unified mode is the recommended default for deployments with more than a few collections.

## Test coverage

126 tests across both backends:

- 79 unit tests — schema validation, field types, coercion, alteration, serialization
- 35 integration tests — CRUD, all filter operators, all aggregation types, pagination, empty sets, non-existent resources, multi-user data isolation, multiple combined filters
- 12 tool tests — tool generation, typed parameters, user isolation, registry filtering, cross-user collision prevention, drop cleanup

All tests pass on both PostgreSQL and libSQL.

## Drawbacks

- Adds a new storage abstraction alongside memory documents.
- Schema rigidity: fields must be defined upfront. Schema evolution is handled via alteration (add/remove fields), not migration.
- Delete operations succeed ~40% of the time. Models struggle to identify which record to remove by description alone. Record IDs in query results make explicit delete-by-ID more reliable.

## Alternatives considered

- **Improve document-based search**: best variant hit 71% on reads, writes remain 0%. Can't search-hint your way out of a data model mismatch.
- **Convention-based structure in documents**: fragile formatting, no query semantics, concurrent updates collide.
- **External database/service**: breaks single-deployment model.
- **Generic CRUD tools**: tested at 41%. Model forgets collection names when they're parameters instead of tool names.

## Future work

- **Computed fields**: cross-field expressions (hours × rate) currently require the LLM to do arithmetic. A richer summary API could support this.
- **Schema evolution**: adding required fields to existing collections doesn't backfill old records. A migration story may be needed at scale.
- **Export formats**: REST API returns JSON only. CSV/other formats could be added as needed.
