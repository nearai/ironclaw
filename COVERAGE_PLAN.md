# IronClaw Coverage Plan: 63.3% to 95%

> Generated 2025-03-06 from [Codecov](https://app.codecov.io/gh/nearai/ironclaw/tree/main/src)

## Current State

| Metric | Value |
|--------|-------|
| **Current coverage** | 48,571 / 76,694 lines = **63.33%** |
| **Target** | 72,859 / 76,694 lines = **95.0%** |
| **Gap** | **24,288 lines** need coverage |
| **Files >= 95%** | 43 / 239 |
| **Files < 95%** | 196 (27,872 total misses) |

## Module Summary

Sorted by uncovered lines (descending):

| Module | Lines | Hits | Miss | Coverage | Priority |
|--------|------:|-----:|-----:|---------:|----------|
| `channels/` | 14,079 | 8,677 | 5,402 | 61.6% | P0 |
| `tools/` | 13,445 | 9,407 | 4,038 | 70.0% | P1 |
| `agent/` | 9,152 | 6,096 | 3,056 | 66.6% | P0 |
| `setup/` | 3,005 | 462 | 2,543 | 15.4% | P1 |
| `extensions/` | 3,540 | 1,298 | 2,242 | 36.7% | P0 |
| `cli/` | 2,834 | 697 | 2,137 | 24.6% | P1 |
| `history/` | 1,626 | 0 | 1,626 | 0.0% | P0 |
| `llm/` | 7,029 | 5,776 | 1,253 | 82.2% | P2 |
| `(root)` | 4,122 | 3,121 | 1,001 | 75.7% | P2 |
| `worker/` | 1,274 | 480 | 794 | 37.7% | P1 |
| `sandbox/` | 1,615 | 897 | 718 | 55.5% | P2 |
| `registry/` | 1,588 | 1,107 | 481 | 69.7% | P2 |
| `db/` | 921 | 441 | 480 | 47.9% | P1 |
| `workspace/` | 2,006 | 1,584 | 422 | 79.0% | P2 |
| `orchestrator/` | 1,199 | 795 | 404 | 66.3% | P2 |
| `config/` | 1,464 | 1,095 | 369 | 74.8% | P2 |
| `hooks/` | 1,379 | 1,081 | 298 | 78.4% | P2 |
| `secrets/` | 687 | 407 | 280 | 59.2% | P2 |
| `skills/` | 1,714 | 1,585 | 129 | 92.5% | P3 |
| `context/` | 693 | 586 | 107 | 84.6% | P3 |
| `estimation/` | 467 | 369 | 98 | 79.0% | P3 |
| `safety/` | 1,424 | 1,337 | 87 | 93.9% | P3 |
| `evaluation/` | 226 | 152 | 74 | 67.3% | P3 |
| `pairing/` | 498 | 446 | 52 | 89.6% | P3 |
| `tunnel/` | 391 | 368 | 23 | 94.1% | P3 |
| `observability/` | 316 | 307 | 9 | 97.2% | Done |

## Top 40 Files by Uncovered Lines

These files account for the vast majority of the coverage gap:

| File | Lines | Miss | Coverage | Lines to 95% |
|------|------:|-----:|---------:|--------------:|
| `src/extensions/manager.rs` | 2,404 | 2,083 | 13.3% | 1,962 |
| `src/setup/wizard.rs` | 2,150 | 1,789 | 16.8% | 1,681 |
| `src/history/store.rs` | 1,486 | 1,486 | 0.0% | 1,411 |
| `src/channels/web/server.rs` | 1,985 | 993 | 50.0% | 893 |
| `src/channels/wasm/wrapper.rs` | 2,237 | 934 | 58.2% | 822 |
| `src/agent/thread_ops.rs` | 1,044 | 763 | 26.9% | 710 |
| `src/cli/tool.rs` | 757 | 735 | 2.9% | 697 |
| `src/setup/channels.rs` | 645 | 596 | 7.6% | 563 |
| `src/agent/commands.rs` | 587 | 587 | 0.0% | 557 |
| `src/main.rs` | 740 | 522 | 29.4% | 485 |
| `src/channels/web/handlers/jobs.rs` | 513 | 456 | 11.1% | 430 |
| `src/tools/builder/core.rs` | 524 | 456 | 13.0% | 429 |
| `src/worker/job.rs` | 1,078 | 467 | 56.7% | 413 |
| `src/channels/web/handlers/chat.rs` | 564 | 417 | 26.1% | 388 |
| `src/tools/wasm/wrapper.rs` | 1,005 | 436 | 56.6% | 385 |
| `src/channels/signal.rs` | 1,814 | 472 | 74.0% | 381 |
| `src/tools/mcp/auth.rs` | 472 | 378 | 19.9% | 354 |
| `src/worker/container.rs` | 350 | 330 | 5.7% | 312 |
| `src/tools/builtin/job.rs` | 1,014 | 359 | 64.6% | 308 |
| `src/cli/mcp.rs` | 322 | 319 | 0.9% | 302 |
| `src/cli/oauth_defaults.rs` | 730 | 335 | 54.1% | 298 |
| `src/llm/nearai_chat.rs` | 854 | 340 | 60.2% | 297 |
| `src/sandbox/container.rs` | 407 | 317 | 22.1% | 296 |
| `src/tools/mcp/client.rs` | 341 | 291 | 14.7% | 273 |
| `src/registry/installer.rs` | 765 | 311 | 59.3% | 272 |
| `src/orchestrator/job_manager.rs` | 405 | 270 | 33.3% | 249 |
| `src/channels/web/handlers/routines.rs` | 249 | 249 | 0.0% | 236 |
| `src/agent/scheduler.rs` | 559 | 263 | 53.0% | 235 |
| `src/tools/wasm/storage.rs` | 296 | 243 | 17.9% | 228 |
| `src/channels/repl.rs` | 233 | 233 | 0.0% | 221 |
| `src/llm/session.rs` | 413 | 242 | 41.4% | 221 |
| `src/worker/claude_bridge.rs` | 629 | 247 | 60.7% | 215 |
| `src/agent/agent_loop.rs` | 523 | 234 | 55.2% | 207 |
| `src/worker/api.rs` | 258 | 207 | 19.8% | 194 |
| `src/sandbox/proxy/http.rs` | 307 | 192 | 37.5% | 176 |
| `src/channels/wasm/storage.rs` | 182 | 182 | 0.0% | 172 |
| `src/cli/registry.rs` | 177 | 177 | 0.0% | 168 |
| `src/llm/reasoning.rs` | 1,163 | 219 | 81.2% | 160 |
| `src/tools/builder/testing.rs` | 308 | 174 | 43.5% | 158 |
| `src/db/postgres.rs` | 166 | 166 | 0.0% | 157 |

---

## Tier 1 -- High-Impact Unit Tests (~8,500 lines)

Pure logic, serialization, and database queries testable in isolation without real
infrastructure. Highest coverage gain per unit of effort.

### `src/history/store.rs` -- 0% -> 95% (+1,411 lines)

PostgreSQL repository layer (conversations, jobs, actions, LLM calls, estimation
snapshots). Test query construction and result mapping. Can use the libSQL backend
as a real in-memory database or test doubles for the `Database` trait.

**Tests to write:**
- `test_store_conversation_crud` -- create, read, update, delete conversations
- `test_store_job_lifecycle` -- insert job, update status through state machine
- `test_store_action_recording` -- record and query job actions
- `test_store_llm_call_tracking` -- insert and aggregate LLM call records
- `test_store_estimation_snapshots` -- save and retrieve estimation data

### `src/history/analytics.rs` -- 0% -> 95% (+133 lines)

Aggregation queries (JobStats, ToolStats). Test the query builders and result
deserialization.

**Tests to write:**
- `test_job_stats_aggregation` -- verify counts, durations, success rates
- `test_tool_stats_ranking` -- verify tool usage frequency sorting
- `test_analytics_empty_db` -- graceful handling of no data

### `src/extensions/manager.rs` -- 13.3% -> 95% (+1,962 lines)

Largest single file gap. Extension lifecycle orchestration (install, auth,
activate, remove), config parsing, and state transitions.

**Tests to write:**
- `test_extension_install_from_manifest` -- parse manifest, create extension record
- `test_extension_auth_flow` -- OAuth token setup, credential storage
- `test_extension_activate_deactivate` -- state transitions, tool registration
- `test_extension_remove_cleanup` -- remove extension, clean up artifacts
- `test_extension_config_validation` -- reject invalid configs, handle defaults
- `test_extension_list_filtering` -- filter by status, type, search query
- `test_extension_capability_check` -- verify required capabilities before activation

### `src/extensions/discovery.rs` -- 27.8% -> 95% (+125 lines)

Extension discovery from filesystem and registry.

**Tests to write:**
- `test_discover_local_extensions` -- scan directory, parse manifests
- `test_discover_skip_invalid` -- gracefully skip malformed extension dirs
- `test_discover_dedup` -- handle duplicate extensions across paths

### `src/tools/builder/core.rs` -- 13% -> 95% (+429 lines)

`BuildRequirement`, `SoftwareType`, `Language` types and project scaffolding.

**Tests to write:**
- `test_build_requirement_parsing` -- deserialize from JSON
- `test_scaffold_project_structure` -- verify generated file tree
- `test_language_detection` -- detect language from file extensions
- `test_software_type_constraints` -- validate type-specific requirements

### `src/tools/builder/testing.rs` -- 43.5% -> 95% (+158 lines)

Test harness integration for built tools.

**Tests to write:**
- `test_harness_setup_teardown` -- lifecycle of test environment
- `test_harness_run_tests` -- execute tests and capture results
- `test_harness_failure_reporting` -- verify error details on test failure

### `src/tools/mcp/auth.rs` -- 19.9% -> 95% (+354 lines)

OAuth token management for MCP servers.

**Tests to write:**
- `test_token_refresh_on_expiry` -- auto-refresh when token expires
- `test_token_header_injection` -- correct Authorization header format
- `test_token_persistence` -- save/load tokens across restarts
- `test_oauth_pkce_flow` -- code verifier/challenge generation
- `test_auth_config_parsing` -- parse various auth config formats

### `src/tools/mcp/client.rs` -- 14.7% -> 95% (+273 lines)

JSON-RPC client for MCP protocol.

**Tests to write:**
- `test_jsonrpc_request_serialization` -- correct JSON-RPC 2.0 format
- `test_jsonrpc_response_parsing` -- handle success, error, and batch responses
- `test_jsonrpc_error_codes` -- map MCP error codes to ToolError
- `test_tool_list_discovery` -- parse tools/list response
- `test_tool_call_roundtrip` -- serialize call, parse result

### `src/tools/wasm/storage.rs` -- 17.9% -> 95% (+228 lines)

WASM tool persistence (store, load, delete, list).

**Tests to write:**
- `test_wasm_tool_store_roundtrip` -- store and retrieve tool binary + metadata
- `test_wasm_tool_delete` -- remove tool and verify gone
- `test_wasm_tool_list_filtering` -- filter by name, capability
- `test_wasm_tool_update_metadata` -- update without re-uploading binary

### `src/tools/wasm/wrapper.rs` -- 56.6% -> 95% (+385 lines)

Tool trait wrapper for WASM modules.

**Tests to write:**
- `test_wasm_param_marshalling` -- JSON params to WASM component model types
- `test_wasm_output_conversion` -- WASM return values to ToolOutput
- `test_wasm_error_propagation` -- WASM traps to ToolError
- `test_wasm_fuel_exhaustion` -- verify fuel limit enforcement
- `test_wasm_memory_limit` -- verify memory ceiling

### `src/tools/wasm/loader.rs` -- 62.4% -> 95% (+156 lines)

WASM tool discovery from filesystem.

**Tests to write:**
- `test_loader_scan_directory` -- find .wasm files with capabilities.json
- `test_loader_skip_invalid` -- skip files without valid WIT exports
- `test_loader_cache_invalidation` -- reload when file changes

### `src/tools/builtin/job.rs` -- 64.6% -> 95% (+308 lines)

Job management tools (CreateJob, ListJobs, JobStatus, CancelJob).

**Tests to write:**
- `test_create_job_params` -- validate required/optional parameters
- `test_list_jobs_formatting` -- verify output structure
- `test_job_status_transitions` -- query status at each state
- `test_cancel_job_running` -- cancel an in-progress job
- `test_cancel_job_completed` -- error on already-completed job

### `src/secrets/store.rs` -- 48.1% -> 95% (+145 lines)

Encrypted secret storage.

**Tests to write:**
- `test_secret_store_roundtrip` -- store encrypted, retrieve decrypted
- `test_secret_update` -- overwrite existing secret
- `test_secret_delete` -- remove and verify inaccessible
- `test_secret_list_redacted` -- list shows names but not values

### `src/llm/session.rs` -- 41.4% -> 95% (+221 lines)

Session token management with auto-renewal.

**Tests to write:**
- `test_session_token_parsing` -- parse `sess_xxx` format
- `test_session_expiry_detection` -- detect expired tokens
- `test_session_auto_renewal` -- trigger renewal before expiry
- `test_session_concurrent_renewal` -- only one renewal in flight

### `src/llm/nearai_chat.rs` -- 60.2% -> 95% (+297 lines)

NEAR AI Chat Completions provider.

**Tests to write:**
- `test_nearai_request_building` -- correct endpoint, headers, body
- `test_nearai_response_parsing` -- parse streaming and non-streaming responses
- `test_nearai_tool_message_flattening` -- tool messages flattened to text
- `test_nearai_auth_modes` -- session token vs API key auth
- `test_nearai_error_handling` -- rate limits, auth failures, server errors

### `src/llm/mod.rs` -- 53.7% -> 95% (+112 lines)

Provider factory and backend selection.

**Tests to write:**
- `test_provider_factory_nearai` -- select NEAR AI from config
- `test_provider_factory_openai` -- select OpenAI from config
- `test_provider_factory_ollama` -- select Ollama from config
- `test_provider_factory_invalid` -- error on unknown backend

### `src/llm/reasoning.rs` -- 81.2% -> 95% (+160 lines)

Planning, tool selection, evaluation logic.

**Tests to write:**
- `test_reasoning_step_parsing` -- parse planning steps from LLM output
- `test_tool_selection_scoring` -- rank tools by relevance
- `test_evaluation_rubric` -- score completions against criteria
- `test_reasoning_with_no_tools` -- handle tool-less responses

### `src/db/postgres.rs` -- 0% -> 95% (+157 lines)

PostgreSQL backend delegation to Store + Repository.

**Tests to write:**
- `test_postgres_backend_delegates` -- verify delegation pattern (trait-level)
- `test_postgres_connection_config` -- TLS, pool size, timeout parsing

### `src/workspace/mod.rs` -- 75.9% -> 95% (+109 lines)

Memory operations (write, read, search, tree).

**Tests to write:**
- `test_workspace_write_read` -- write document, read it back
- `test_workspace_search_hybrid` -- FTS + vector search via RRF
- `test_workspace_tree` -- directory listing of memory filesystem
- `test_workspace_overwrite` -- update existing document

### `src/workspace/embeddings.rs` -- 35.1% -> 95% (~100 lines)

Embedding provider abstraction.

**Tests to write:**
- `test_embedding_dimension_handling` -- verify dimension config
- `test_embedding_batch_processing` -- batch multiple chunks
- `test_embedding_provider_fallback` -- graceful degradation when unavailable

---

## Tier 2 -- Trace Tests (~7,000 lines)

End-to-end tests that exercise the agent loop, worker, scheduler, and dispatcher
by replaying LLM traces through `TestRig` (see `tests/support/test_rig.rs`). Each
trace test covers multiple modules simultaneously, making them high-leverage.

Each trace test needs:
1. A JSON fixture in `tests/fixtures/llm_traces/`
2. A test file in `tests/` using `TestRigBuilder`

### Trace: Thread Operations

**Covers:** `agent/thread_ops.rs` (+710 lines)

Test thread creation, listing, switching, and deletion via trace replay.

**Fixture:** `thread_operations.json`
**Tests:**
- `test_thread_create_and_switch` -- create thread, switch to it, verify context
- `test_thread_list` -- list all threads, verify metadata
- `test_thread_delete` -- delete thread, verify removal
- `test_thread_switch_nonexistent` -- error handling for missing thread

### Trace: Agent Commands

**Covers:** `agent/commands.rs` (+557 lines)

Test slash commands through the agent loop.

**Fixture:** `agent_commands.json`
**Tests:**
- `test_command_help` -- /help returns command list
- `test_command_clear` -- /clear resets conversation
- `test_command_compact` -- /compact triggers summarization
- `test_command_undo_redo` -- /undo then /redo restores state
- `test_command_status` -- /status shows agent state

### Trace: Worker Multi-Turn Execution

**Covers:** `worker/job.rs` (+413 lines), `agent/agent_loop.rs` (+207 lines)

Test multi-turn tool calling, error recovery, and completion flows.

**Fixture:** `worker_multi_turn.json`
**Tests:**
- `test_worker_sequential_tools` -- call tool A, then tool B based on A's result
- `test_worker_tool_error_recovery` -- tool fails, agent retries or adapts
- `test_worker_max_turns` -- verify turn limit enforcement

### Trace: Scheduler Parallel Jobs

**Covers:** `agent/scheduler.rs` (+235 lines)

Test parallel job dispatch and completion tracking.

**Fixture:** `scheduler_parallel.json`
**Tests:**
- `test_scheduler_parallel_dispatch` -- dispatch 3 jobs, all complete
- `test_scheduler_job_dependency` -- job B waits for job A
- `test_scheduler_stuck_detection` -- detect and recover stuck job

### Trace: Dispatcher Skill Selection

**Covers:** `agent/dispatcher.rs` (+153 lines)

Test skill-aware routing and tool attenuation.

**Fixture:** `dispatcher_skills.json`
**Tests:**
- `test_dispatcher_skill_match` -- match message to skill, inject prompt
- `test_dispatcher_tool_attenuation` -- installed skill loses dangerous tools
- `test_dispatcher_no_skill` -- fallback when no skill matches

### Trace: Routine Execution

**Covers:** `agent/routine_engine.rs` (~80 lines), `agent/routine.rs` (~40 lines)

Test cron tick and event-triggered routine execution.

**Fixture:** `routine_execution.json`
**Tests:**
- `test_routine_cron_trigger` -- routine fires on schedule
- `test_routine_event_trigger` -- routine fires on matching event
- `test_routine_guardrails` -- routine respects policy constraints

### Trace: Compaction and Context Pressure

**Covers:** `agent/compaction.rs` (~50 lines), `agent/context_monitor.rs` (~30 lines)

Test turn summarization and memory pressure detection.

**Fixture:** `compaction_flow.json`
**Tests:**
- `test_compaction_triggers_at_threshold` -- summarize when context exceeds limit
- `test_compaction_preserves_recent` -- keep recent turns intact
- `test_context_pressure_warning` -- emit warning at high usage

### Trace: Job Tool Coverage

**Covers:** `tools/builtin/job.rs` (+308 lines), `tools/builtin/skill_tools.rs` (+110 lines)

Test job and skill management tools through agent execution.

**Fixture:** `job_and_skill_tools.json`
**Tests:**
- `test_create_and_list_jobs` -- create job, list shows it
- `test_job_status_query` -- query status of running job
- `test_skill_list_and_search` -- list local skills, search registry

### Trace: Memory Tools

**Covers:** `tools/builtin/memory.rs` (~20 lines), `workspace/` (+109 lines)

Test memory operations through agent tool calls.

**Fixture:** `memory_tools.json`
**Tests:**
- `test_memory_write_and_search` -- write doc, search finds it
- `test_memory_read_by_path` -- read specific document
- `test_memory_tree` -- list memory filesystem structure

### Trace: Extension Management

**Covers:** `tools/builtin/extension_tools.rs` (~40 lines)

Test extension lifecycle via agent tool calls.

**Fixture:** `extension_management.json`
**Tests:**
- `test_extension_install_via_tool` -- agent installs an extension
- `test_extension_auth_via_tool` -- agent configures auth
- `test_extension_activate_via_tool` -- agent activates extension

### Trace: Self-Repair

**Covers:** `agent/self_repair.rs` (~40 lines)

Test stuck job detection and recovery.

**Fixture:** `self_repair.json`
**Tests:**
- `test_stuck_job_detected` -- job stuck for > threshold triggers repair
- `test_stuck_job_recovered` -- recovery restarts job successfully
- `test_stuck_job_fails_permanently` -- recovery fails, job marked failed

### Trace: Heartbeat

**Covers:** `agent/heartbeat.rs` (+80 lines)

Test periodic proactive execution.

**Fixture:** `heartbeat.json`
**Tests:**
- `test_heartbeat_periodic_fire` -- heartbeat triggers at interval
- `test_heartbeat_reads_checklist` -- reads HEARTBEAT.md, processes items
- `test_heartbeat_notification` -- sends notification on findings

---

## Tier 3 -- Web/Channel Handler Tests (~4,500 lines)

Test HTTP handlers and SSE/WS endpoints using `axum_test` or
`tower::ServiceExt::oneshot` with a real router and in-memory database.

### `src/channels/web/server.rs` -- 50% -> 95% (+893 lines)

The single biggest web gap. 40+ API endpoints.

**Tests to write:**
- `test_api_health` -- GET /health returns 200
- `test_api_chat_submit` -- POST /api/chat sends message
- `test_api_jobs_list` -- GET /api/jobs returns job list
- `test_api_jobs_create` -- POST /api/jobs creates job
- `test_api_routines_crud` -- full CRUD cycle for routines
- `test_api_settings_get_set` -- GET/PUT settings
- `test_api_memory_search` -- POST /api/memory/search
- `test_api_extensions_list` -- GET /api/extensions
- `test_api_skills_list` -- GET /api/skills
- `test_api_sse_connect` -- SSE stream connects and receives events
- `test_api_auth_required` -- endpoints reject missing/bad tokens
- `test_api_cors_headers` -- verify CORS configuration

### `src/channels/web/handlers/chat.rs` -- 26.1% -> 95% (+388 lines)

Chat message submission and SSE streaming.

**Tests to write:**
- `test_chat_submit_message` -- submit message, receive response
- `test_chat_sse_stream` -- verify SSE event format
- `test_chat_thread_context` -- messages scoped to thread
- `test_chat_invalid_payload` -- reject malformed requests

### `src/channels/web/handlers/jobs.rs` -- 11.1% -> 95% (+430 lines)

Job CRUD endpoints.

**Tests to write:**
- `test_jobs_list_empty` -- empty list returns []
- `test_jobs_create_and_get` -- create, then GET by ID
- `test_jobs_cancel` -- cancel running job
- `test_jobs_filter_by_status` -- filter by pending/running/completed
- `test_jobs_pagination` -- limit/offset parameters

### `src/channels/web/handlers/routines.rs` -- 0% -> 95% (+236 lines)

Routine CRUD endpoints.

**Tests to write:**
- `test_routines_create` -- POST creates routine
- `test_routines_list` -- GET lists all routines
- `test_routines_update` -- PUT updates routine config
- `test_routines_delete` -- DELETE removes routine
- `test_routines_history` -- GET history for a routine

### `src/channels/web/handlers/extensions.rs` -- 0% -> 95% (+129 lines)

Extension management endpoints.

**Tests to write:**
- `test_extensions_list` -- list installed extensions
- `test_extensions_install` -- install from manifest URL
- `test_extensions_activate` -- activate/deactivate toggle
- `test_extensions_remove` -- remove installed extension

### `src/channels/web/handlers/memory.rs` -- 0% -> 95% (+110 lines)

Memory/workspace endpoints.

**Tests to write:**
- `test_memory_search` -- search returns ranked results
- `test_memory_write` -- write a document
- `test_memory_read` -- read by path
- `test_memory_tree` -- tree returns filesystem structure

### `src/channels/web/handlers/settings.rs` -- 0% -> 95% (+103 lines)

Settings endpoints.

**Tests to write:**
- `test_settings_get` -- retrieve current settings
- `test_settings_update` -- update individual setting
- `test_settings_validation` -- reject invalid setting values

### `src/channels/web/handlers/static_files.rs` -- 0% -> 95% (+97 lines)

Static file serving.

**Tests to write:**
- `test_static_index_html` -- GET / serves index.html
- `test_static_css_js` -- serve CSS/JS with correct content types
- `test_static_404` -- missing file returns 404

### `src/channels/wasm/wrapper.rs` -- 58.2% -> 95% (+822 lines)

WASM channel wrapper (message routing, lifecycle).

**Tests to write:**
- `test_wasm_channel_start` -- initialize WASM channel module
- `test_wasm_channel_message_routing` -- route incoming message to WASM
- `test_wasm_channel_response` -- return WASM response to caller
- `test_wasm_channel_error_handling` -- handle WASM trap gracefully
- `test_wasm_channel_lifecycle` -- start, process, shutdown

### `src/channels/wasm/loader.rs` -- 38.1% -> 95% (+141 lines)

WASM channel discovery.

**Tests to write:**
- `test_channel_loader_scan` -- find channel WASM modules
- `test_channel_loader_validation` -- reject invalid modules
- `test_channel_loader_manifest` -- parse channel capabilities

### `src/channels/wasm/storage.rs` -- 0% -> 95% (+172 lines)

WASM channel state persistence.

**Tests to write:**
- `test_channel_storage_save_load` -- persist and restore channel state
- `test_channel_storage_isolation` -- per-channel state isolation
- `test_channel_storage_cleanup` -- remove state on channel uninstall

### `src/channels/signal.rs` -- 74% -> 95% (+381 lines)

Signal protocol channel.

**Tests to write:**
- `test_signal_message_send` -- send encrypted message
- `test_signal_message_receive` -- decrypt incoming message
- `test_signal_attachment_handling` -- handle media attachments
- `test_signal_group_message` -- group chat routing
- `test_signal_error_handling` -- handle connection failures

### `src/channels/repl.rs` -- 0% -> 95% (+221 lines)

Simple REPL channel.

**Tests to write:**
- `test_repl_input_parsing` -- parse user input lines
- `test_repl_output_formatting` -- format agent responses
- `test_repl_multiline` -- handle multi-line input
- `test_repl_special_commands` -- handle /quit, /help

---

## Tier 4 -- CLI Tests (~2,100 lines)

CLI subcommands can be tested by invoking clap-parsed command structs directly
or by calling the handler functions with constructed arguments.

### `src/cli/tool.rs` -- 2.9% -> 95% (+697 lines)

Tool CLI (install, list, remove, build).

**Tests to write:**
- `test_cli_tool_list` -- list installed tools
- `test_cli_tool_install_local` -- install from local .wasm file
- `test_cli_tool_install_registry` -- install from registry
- `test_cli_tool_remove` -- remove installed tool
- `test_cli_tool_build` -- scaffold and build tool project
- `test_cli_tool_info` -- display tool details

### `src/cli/mcp.rs` -- 0.9% -> 95% (+302 lines)

MCP server management CLI.

**Tests to write:**
- `test_cli_mcp_list` -- list configured MCP servers
- `test_cli_mcp_add` -- add MCP server config
- `test_cli_mcp_remove` -- remove MCP server config
- `test_cli_mcp_tools` -- list tools from MCP server
- `test_cli_mcp_test_connection` -- verify MCP server reachable

### `src/cli/oauth_defaults.rs` -- 54.1% -> 95% (+298 lines)

OAuth default configurations.

**Tests to write:**
- `test_oauth_defaults_loading` -- load default OAuth configs
- `test_oauth_url_construction` -- build auth/token URLs
- `test_oauth_scope_merging` -- merge requested scopes with defaults
- `test_oauth_provider_lookup` -- lookup by provider name

### `src/cli/registry.rs` -- 0% -> 95% (+168 lines)

Registry CLI commands.

**Tests to write:**
- `test_cli_registry_search` -- search for packages
- `test_cli_registry_install` -- install package from registry
- `test_cli_registry_info` -- display package details

### `src/cli/status.rs` -- 0% -> 95% (+142 lines)

Status display commands.

**Tests to write:**
- `test_cli_status_gathering` -- collect system status info
- `test_cli_status_formatting` -- render status output
- `test_cli_status_components` -- check individual components

### `src/cli/memory.rs` -- 15.5% -> 95% (+138 lines)

Memory CLI subcommands.

**Tests to write:**
- `test_cli_memory_search` -- search workspace from CLI
- `test_cli_memory_write` -- write document from CLI
- `test_cli_memory_read` -- read document from CLI
- `test_cli_memory_tree` -- display memory tree

### `src/cli/doctor.rs` -- 28.7% -> 95% (+115 lines)

Diagnostic checks.

**Tests to write:**
- `test_doctor_check_database` -- verify DB connectivity check
- `test_doctor_check_llm` -- verify LLM provider check
- `test_doctor_check_tools` -- verify tool availability check
- `test_doctor_report_format` -- verify output format

### `src/cli/config.rs` -- 36.5% -> 95% (~100 lines)

Config CLI subcommands.

**Tests to write:**
- `test_cli_config_get` -- read config value
- `test_cli_config_set` -- write config value
- `test_cli_config_list` -- list all config keys
- `test_cli_config_reset` -- reset to defaults

---

## Tier 5 -- Setup/Infra Tests (~2,400 lines)

Hardest to test: interactive wizards, Docker, process spawning. Strategy: extract
pure logic into testable functions, test the interactive parts by injecting mock
input.

### `src/setup/wizard.rs` -- 16.8% -> 95% (+1,681 lines)

7-step interactive onboarding wizard. Refactor to extract validation functions,
step logic, and config generation into testable units.

**Tests to write:**
- `test_wizard_step_validation` -- each step validates input correctly
- `test_wizard_config_generation` -- generate config from wizard answers
- `test_wizard_default_values` -- verify sensible defaults
- `test_wizard_skip_completed` -- skip already-configured steps
- `test_wizard_llm_backend_selection` -- provider-specific config paths
- `test_wizard_channel_setup` -- channel configuration logic

### `src/setup/channels.rs` -- 7.6% -> 95% (+563 lines)

Channel setup helpers.

**Tests to write:**
- `test_channel_setup_defaults` -- default channel configuration
- `test_channel_setup_validation` -- reject invalid channel configs
- `test_channel_setup_telegram` -- Telegram-specific setup logic
- `test_channel_setup_signal` -- Signal-specific setup logic
- `test_channel_setup_webhook` -- webhook URL validation

### `src/setup/prompts.rs` -- 24.8% -> 95% (+147 lines)

Terminal prompt utilities.

**Tests to write:**
- `test_prompt_select` -- selection from list
- `test_prompt_confirm` -- yes/no confirmation
- `test_prompt_secret` -- masked input
- `test_prompt_validation` -- input validation rules

### `src/sandbox/container.rs` -- 22.1% -> 95% (+296 lines)

Docker container lifecycle. Test command construction without actual Docker.

**Tests to write:**
- `test_container_config_to_docker_args` -- generate correct docker run args
- `test_container_volume_mounts` -- workspace mount configuration
- `test_container_env_scrubbing` -- sensitive env vars removed
- `test_container_resource_limits` -- CPU/memory limit args
- `test_container_network_config` -- proxy network setup

### `src/sandbox/manager.rs` -- 59% -> 95% (+114 lines)

Sandbox orchestration.

**Tests to write:**
- `test_sandbox_policy_enforcement` -- policy to container config mapping
- `test_sandbox_cleanup` -- cleanup on job completion
- `test_sandbox_concurrent_limit` -- enforce max concurrent containers

### `src/sandbox/proxy/http.rs` -- 37.5% -> 95% (+176 lines)

HTTP proxy for container network access.

**Tests to write:**
- `test_proxy_allowlist_enforcement` -- block disallowed domains
- `test_proxy_credential_injection` -- inject auth headers
- `test_proxy_connect_tunnel` -- HTTPS CONNECT method handling
- `test_proxy_logging` -- request/response logging

### `src/worker/container.rs` -- 5.7% -> 95% (+312 lines)

Worker execution loop (runs inside containers).

**Tests to write:**
- `test_worker_tool_dispatch` -- dispatch tool call, return result
- `test_worker_llm_interaction` -- send prompt, receive response
- `test_worker_turn_limit` -- enforce max turns
- `test_worker_error_propagation` -- tool error surfaces to agent

### `src/worker/claude_bridge.rs` -- 60.7% -> 95% (+215 lines)

Claude CLI bridge.

**Tests to write:**
- `test_claude_command_construction` -- build claude CLI command
- `test_claude_output_parsing` -- parse claude CLI JSON output
- `test_claude_error_handling` -- handle CLI crashes gracefully
- `test_claude_config_injection` -- inject config dir and model

### `src/worker/api.rs` -- 19.8% -> 95% (+194 lines)

Worker HTTP client to orchestrator.

**Tests to write:**
- `test_worker_api_request_building` -- correct endpoint URLs and headers
- `test_worker_api_response_parsing` -- parse orchestrator responses
- `test_worker_api_auth_token` -- bearer token injection
- `test_worker_api_retry` -- retry on transient failures

### `src/main.rs` -- 29.4% -> 95% (+485 lines)

Entry point and startup. Extract startup logic into testable functions.

**Tests to write:**
- `test_cli_arg_parsing` -- verify clap argument parsing
- `test_startup_config_loading` -- config from env + file
- `test_startup_channel_selection` -- select channels from config
- `test_startup_feature_flags` -- feature-gated code paths

---

## Tier 6 -- Remaining Files to 95% (~2,000 lines)

Smaller files that each need a handful of additional tests.

| File | Lines Needed | Test Focus |
|------|-------------:|------------|
| `src/tools/builtin/skill_tools.rs` | 110 | skill_list, skill_search, skill_install, skill_remove |
| `src/hooks/bundled.rs` | 115 | bundled hook execution, hook discovery |
| `src/registry/installer.rs` | 272 | package download, verification, installation |
| `src/registry/artifacts.rs` | 72 | artifact packaging, checksums |
| `src/orchestrator/job_manager.rs` | 249 | container lifecycle, job routing |
| `src/orchestrator/api.rs` | 125 | LLM proxy, event dispatch endpoints |
| `src/app.rs` | 137 | AppBuilder configuration, startup sequence |
| `src/service.rs` | 120 | service lifecycle, signal handling |
| `src/config/channels.rs` | 55 | channel config parsing |
| `src/config/sandbox.rs` | 61 | sandbox config parsing |
| `src/config/tunnel.rs` | 43 | tunnel config parsing |
| `src/config/mod.rs` | 63 | config merging, env override |
| `src/config/database.rs` | 38 | database URL parsing |
| `src/evaluation/success.rs` | 34 | success evaluator logic |
| `src/evaluation/metrics.rs` | 40 | metrics collection |
| `src/context/manager.rs` | 57 | concurrent job context isolation |
| `src/context/memory.rs` | 36 | action recording, conversation memory |

---

## Execution Priority

Maximize coverage gain per unit of effort:

| Order | Category | Lines Gained | Effort |
|------:|----------|-------------:|--------|
| 1 | Trace tests (Tier 2) | ~7,000 | Medium (high leverage, each test covers many modules) |
| 2 | Unit tests for 0% files (Tier 1 subset) | ~3,500 | Low (pure logic, no infrastructure) |
| 3 | Web handler tests (Tier 3) | ~4,500 | Medium (axum_test + in-memory DB) |
| 4 | Extension/MCP/WASM unit tests (Tier 1 remainder) | ~3,500 | Medium |
| 5 | CLI subcommand tests (Tier 4) | ~2,100 | Low-Medium |
| 6 | Setup wizard extraction + tests (Tier 5) | ~2,400 | High (requires refactoring) |
| 7 | LLM provider tests (Tier 1 subset) | ~800 | Medium |
| 8 | Remaining small files (Tier 6) | ~2,000 | Low |

## Notes

- All trace tests require `--features libsql` and use `TestRigBuilder` from `tests/support/`
- Web handler tests can use `axum::test` helpers or build the router directly
- CLI tests should call handler functions directly, not shell out to the binary
- Setup wizard tests require extracting pure logic from interactive prompts first
- Sandbox/container tests should verify command construction, not run Docker
- Worker tests can use `TraceLlm` for the LLM provider, same as trace tests
