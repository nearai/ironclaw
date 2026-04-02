# Structured Collections Benchmark Results

**Date:** 2026-04-02
**Branch:** `fix/collection-scoping-and-startup` (11 commits on `upstream/staging`)
**Build:** IronClaw v0.22.0 with full dual-backend collections support

## Hypothesis

Structured collections (schema-validated CRUD tools) outperform flat memory documents for data tracking tasks, regardless of model size.

## Method

2×2 factorial design: two models × two data modes, 28 scenarios across 4 categories.

### Models

| Model | Type | Tool Descriptions |
|-------|------|-------------------|
| Qwen 3.5-35B-A3B | Local (MLX, 8-bit) | Compressed (~15 tokens/tool) |
| Claude Haiku 4.5 | Cloud (Anthropic API) | Full JSON schema |

### Data Modes

- **Collections:** Data seeded as structured schemas via REST API. Model gets 5 typed CRUD tools per collection (add, query, update, delete, summary) with schema validation, filtering, and aggregation.
- **Memory Docs:** Data seeded as flat markdown files in workspace memory. Model uses generic `memory_search`/`memory_write` tools.

### Scenarios (28 total)

| Category | N | Operations Tested |
|----------|---|-------------------|
| Grocery | 7 | List queries, add items, remove items, store filtering, counting |
| Nanny | 7 | Hour totals by week/month, shift logging, cost calculation |
| Todo | 6 | List queries, priority filtering, status updates, item creation |
| Transactions | 8 | Spending queries, vendor filtering, category sums, date ranges |

## Results

### Overall Scores

|  | Collections | Memory Docs | Delta |
|--|:-----------:|:-----------:|:-----:|
| **Qwen 3.5** | **0.65** | 0.37 | **+0.28** |
| **Haiku 4.5** | **0.70** | 0.26 | **+0.44** |

### By Category

| Category | Qwen+Coll | Qwen+Mem | Haiku+Coll | Haiku+Mem |
|----------|:---------:|:--------:|:----------:|:---------:|
| Grocery | **0.76** | 0.29 | **0.76** | 0.28 |
| Nanny | 0.56 | 0.21 | **0.69** | 0.24 |
| Todo | **0.72** | 0.52 | **0.72** | 0.35 |
| Transactions | 0.59 | 0.48 | **0.65** | 0.21 |

### Per-Scenario Detail

```
ID           Category       Qwen+C Qwen+M Haiku+C Haiku+M
-----------------------------------------------------------
grocery_01   grocery         1.00   1.00   1.00   0.00
grocery_02   grocery         1.00   0.00   1.00   1.00
grocery_03   grocery         0.65   0.30   0.65   0.35
grocery_04   grocery         0.40   0.00   0.40   0.20
grocery_05   grocery         1.00   0.70   1.00   0.00
grocery_06   grocery         0.30   0.00   0.30   0.00
grocery_07   grocery         1.00   0.00   1.00   0.40
nanny_01     nanny           0.40   0.40   0.40   0.00
nanny_02     nanny           0.00   0.40   0.00   0.00
nanny_03     nanny           1.00   0.00   1.00   0.00
nanny_04     nanny           0.60   0.00   0.70   0.00
nanny_05     nanny           0.53   0.00   1.00   1.00
nanny_06     nanny           1.00   0.70   1.00   0.70
nanny_07     nanny           0.40   0.00   0.70   0.00
todo_01      todo            1.00   0.70   1.00   0.00
todo_02      todo            1.00   0.70   1.00   0.00
todo_03      todo            1.00   0.00   1.00   1.00
todo_04      todo            1.00   0.40   1.00   0.40
todo_05      todo            0.30   1.00   0.30   0.70
todo_06      todo            0.00   0.30   0.00   0.00
txn_01       transactions    0.30   0.40   1.00   0.00
txn_02       transactions    0.40   1.00   0.40   0.00
txn_03       transactions    1.00   0.00   1.00   0.60
txn_04       transactions    0.30   0.82   0.82   0.00
txn_05       transactions    0.35   0.00   0.65   0.35
txn_06       transactions    0.70   0.30   0.30   0.00
txn_07       transactions    0.70   1.00   0.40   0.00
txn_08       transactions    1.00   0.30   0.65   0.70
```

## Analysis

### Collections provide a universal improvement

Both models benefit substantially from structured collections:
- **Qwen:** +0.28 overall (0.37 → 0.65)
- **Haiku:** +0.44 overall (0.26 → 0.70)

The improvement is consistent across all 4 categories. The largest gains come from operations that are fundamentally difficult with flat files — adding structured records, removing specific items, and aggregating numeric values.

### Write operations drive the biggest gains

With memory docs, models must parse markdown, modify it in place, and write it back. This read-modify-write pattern fails frequently:
- **grocery_02** (add items): Qwen goes from 0.00 → 1.00 with collections
- **grocery_07** (add with details): Qwen 0.00 → 1.00, Haiku 0.40 → 1.00
- **nanny_03** (log shift): Both models 0.00 → 1.00

With collections, writes are a single tool call (`grocery_items_add`) with typed parameters. No parsing, no corruption risk.

### Aggregation queries benefit from structured storage

Nanny hour totals, transaction sums, and item counts are natural database operations but extremely difficult via text search:
- **nanny_04** (monthly total): Qwen 0.00 → 0.60, Haiku 0.00 → 0.70
- **nanny_07** (cost calculation): Both models 0.00 → 0.40/0.70

The `_summary` tool provides SUM/COUNT/AVG/MIN/MAX directly, eliminating the need for the model to extract numbers from text and compute arithmetic.

### Collections equalize model quality

Without collections, Haiku's advantage over Qwen is inconsistent — Haiku scores 0.26 vs Qwen's 0.37 on memory docs (Qwen actually wins). With collections, both converge: 0.70 vs 0.65 (1.08× ratio). Structured tools normalize the interface, reducing the importance of model-specific text manipulation ability.

### Grocery achieves parity across models

Both models score 0.76 on grocery with collections — identical performance. The grocery domain is well-suited to collections because operations map cleanly to CRUD: "add eggs" → `grocery_items_add`, "what's on the list" → `grocery_items_query`.

## Test Coverage

The branch includes 126 tests validating the feature:
- **79 unit tests** — schema validation, field types, alteration, serialization
- **35 integration tests** — CRUD, filtering (Eq/Lt/Lte/Gt/Gte/Ne), aggregation (Sum/Count/Avg/Min/Max/GroupBy), pagination, empty sets, non-existent resources, multi-user isolation, multiple combined filters
- **12 tool tests** — tool generation, typed parameters, user isolation, registry filtering, cross-user collision prevention

All tests pass on both PostgreSQL and libSQL backends.

## Implementation

11 commits, ~10,000 lines across 26 files:

1. **Database layer** — `StructuredStore` trait (10 methods), PostgreSQL implementation with JSONB operators, libSQL implementation with `json_extract()`, V16 migration
2. **Tool system** — 5 dynamic CRUD tools per collection, per-user name scoping (`{user}_{collection}_{op}`), `owner_user_id()` trait method for registry filtering
3. **REST API** — 6 endpoints for schema management and record CRUD
4. **Skill system** — Per-collection SKILL.md generation with keyword/pattern activation, collections router skill, workspace discovery docs for embedding search
5. **Startup initialization** — Existing schemas loaded and tools registered on restart
6. **Security** — Per-user tool filtering in dispatcher (tools only visible to owner), data isolation at DB layer, SQL injection prevention via parameterized queries and name validation
