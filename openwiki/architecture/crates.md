# Crate Reference

This page documents all 68+ crates in the IronClaw repository, organized by functional group. Use this as a reference when exploring code or deciding where to add new features.

## Quick Index

- [Core Contracts](#core-contracts-5-crates) — Shared types and traits
- [Authority & Gates](#authority--gates-9-crates) — Security and policy
- [Capability Execution](#capability-execution-11-crates) — Tools and sandboxes
- [Durable State & Events](#durable-state--events-9-crates) — Persistence
- [Products & Loops](#products--loops-27-crates) — Agent and workflows
- [Storage Backends](#storage-backends-8-crates) — Database adapters
- [Utilities](#utilities-7-crates) — Logging, embeddings, etc.

---

## Core Contracts (5 crates)

**Purpose:** Shared types, traits, and interfaces used everywhere.

### ironclaw_host_api
**Role:** Loop-to-kernel communication contract
- Defines `HostPort` trait (effects loops request)
- `CapabilityRequest`, `CapabilityResponse` types
- Observer trait for event subscriptions
- **When to touch:** Adding new request types or capabilities
- **Key modules:** `capabilities.rs`, `port.rs`
- **Depends on:** `ironclaw_common`

### ironclaw_common
**Role:** Shared types and utilities
- `Attachment`, `Event`, `Identity`, `Platform` types
- Environment helpers, hashing, timezone utilities
- Provider transcript types
- **When to touch:** Adding shared utilities or types
- **Key modules:** `attachment.rs`, `event.rs`, `identity.rs`
- **Depends on:** tokio, serde, chrono

### ironclaw_prompt_envelope
**Role:** Prompt composition and safety
- Prompt template system
- Variable substitution, placeholder handling
- Injection safety validation
- **When to touch:** Changing how prompts are constructed
- **Key modules:** `envelope.rs`, `validation.rs`
- **Depends on:** `ironclaw_common`, `ironclaw_safety`

### ironclaw_runtime_policy
**Role:** Policy types and profiles
- Policy profile definitions (secure_default, local-dev, etc.)
- Permission sets, resource limits
- Validation and normalization
- **When to touch:** Adding new policies or profiles
- **Key modules:** `profile.rs`, `permission.rs`, `limits.rs`
- **Depends on:** serde, toml

### ironclaw_architecture
**Role:** Architecture boundary tests and enforcement
- Dependency graph checking
- Composition boundary tests
- Reborn vs v1 boundary enforcement
- **When to touch:** Refactoring crate dependencies
- **Key modules:** `tests/reborn_composition_boundaries.rs`
- **Tests:** Run with `cargo test -p ironclaw_architecture --test '*'`

---

## Authority & Gates (9 crates)

**Purpose:** Policy enforcement, security gates, and access control.

### ironclaw_authorization
**Role:** Access control and permission checking
- RBAC: role-based access control
- Permission checks (who can invoke this capability?)
- Tenant isolation (multi-tenant access)
- **When to touch:** Adding new permission types or role definitions
- **Key modules:** `lib.rs`, `rbac.rs`
- **Tests:** `tests/capability_access_contract.rs`, `tests/capability_lease_contract.rs`
- **Depends on:** `ironclaw_host_api`, `ironclaw_common`

### ironclaw_approvals
**Role:** Human approval flows
- Approval request creation and resolution
- Lease management (time-bound permissions)
- Auto-approve rules
- CAS (compare-and-set) record tracking
- **When to touch:** Changing approval policies or lease terms
- **Key modules:** `auto_approve.rs`, `policy.rs`, `capability_permission.rs`
- **Tests:** `tests/approval_resolution_contract.rs`
- **Depends on:** `ironclaw_runtime_policy`, `ironclaw_common`

### ironclaw_trust
**Role:** Trust boundaries and identity verification
- Trust assessment (is this request trustworthy?)
- Identity verification (who are we talking to?)
- Tenant/user boundary enforcement
- **When to touch:** Changing trust models or identity verification
- **Key modules:** `boundary.rs`, `assessment.rs`
- **Depends on:** `ironclaw_common`

### ironclaw_resources
**Role:** Resource governance and cost tracking
- Quota enforcement (tokens, API calls, etc.)
- Cost tracking per user/tenant
- Resource reserve/reconcile/release cycle
- **When to touch:** Adding new resource types or quota models
- **Key modules:** `governor.rs`, `quota.rs`
- **Depends on:** `ironclaw_host_api`, `ironclaw_common`

### ironclaw_secrets
**Role:** Encrypted secret storage
- Master key management
- Encryption at rest (user secrets, API keys)
- Credential injection (at transit time, not at rest)
- Redaction (prevent logging of secrets)
- **When to touch:** Changing encryption schemes or secret formats
- **Key modules:** `encrypt.rs`, `redact.rs`, `vault.rs`
- **Tests:** Secret handling contract tests
- **Depends on:** `ironclaw_common`, crypto libraries

### ironclaw_safety
**Role:** Runtime safety and injection prevention
- Prompt injection detection
- Credential detection (prevent leaking of secrets in output)
- Input sanitization
- Unsafe language patterns
- **When to touch:** Adding new safety checks or threats
- **Key modules:** `injection.rs`, `credential_detector.rs`
- **Tests:** `tests/` with detailed safety scenarios
- **Depends on:** `ironclaw_common`, regex, language models

### ironclaw_network
**Role:** Network sandbox and allowlisting
- DNS allowlist/denylist enforcement
- IP filtering (private network protection)
- TLS validation
- Network timeout policy
- **When to touch:** Adding network restrictions or bypass rules
- **Key modules:** `sandbox.rs`, `allowlist.rs`
- **Depends on:** `ironclaw_host_api`, `reqwest`, `tokio`

### ironclaw_filesystem
**Role:** File access control and isolation
- File scoping (users can only access their files)
- Namespace isolation
- Integrity checking (content-addressed storage)
- Catalog of accessible files
- **When to touch:** Changing file access model or adding new backends
- **Key modules:** `catalog.rs`, `backend.rs`, `scoped.rs`
- **Backends:** Local, database-backed (PostgreSQL/libSQL)
- **Depends on:** `ironclaw_host_api`, `ironclaw_common`

### ironclaw_hooks
**Role:** Lifecycle hooks and event subscriptions
- Hook system for startup, shutdown, events
- Observer trait implementations
- Plugin registration
- **When to touch:** Adding new hook points or event subscribers
- **Key modules:** `lib.rs`, `observer.rs`
- **Backend implementations:** `ironclaw_hooks_postgres`, `ironclaw_hooks_libsql`
- **Depends on:** `ironclaw_host_api`, `ironclaw_common`

---

## Capability Execution (11 crates)

**Purpose:** Tool registration, dispatch, and sandboxed execution.

### ironclaw_capabilities
**Role:** Capability registry and host API
- Capability manifest (name, description, input schema, output schema)
- Profile conformance (which capabilities are allowed in this profile?)
- Host API implementation
- Request/response handling
- **When to touch:** Adding new capability types or conformance rules
- **Key modules:** `host.rs`, `conformance.rs`, `requests.rs`
- **Tests:** `tests/capability_host_contract.rs`, `tests/capability_host_auth_required_enrichment_contract.rs`
- **Depends on:** `ironclaw_host_api`, `ironclaw_common`

### ironclaw_dispatcher
**Role:** Multi-destination dispatch
- Route requests to multiple handlers (tools, channels, subscriptions)
- Load balancing and failover
- Saga pattern for distributed transactions
- **When to touch:** Adding new dispatch destinations or routing rules
- **Key modules:** `lib.rs`
- **Tests:** `tests/dispatch_contract.rs`, `tests/event_dispatch_contract.rs`
- **Depends on:** `ironclaw_host_api`, `ironclaw_common`

### ironclaw_wasm
**Role:** WASM sandbox runtime
- Tool execution in WASM sandbox
- Memory isolation (WASM linear memory)
- Host function bindings
- **When to touch:** Adding new host functions or sandbox features
- **Key modules:** `lib.rs`, `sandbox.rs`
- **Tests:** Sandbox contract tests
- **Depends on:** `wasmer`, `ironclaw_host_api`

### ironclaw_wasm_sandbox_core
**Role:** Low-level WASM integration
- Raw WASM interface
- Memory mapping
- Sandbox initialization
- **When to touch:** Low-level sandbox changes
- **Depends on:** `wasmer`, `wasmtime`

### ironclaw_wasm_limiter
**Role:** Resource limits in WASM
- Memory limits (prevent unbounded allocation)
- Time limits (timeout enforcement)
- Instruction counting
- **When to touch:** Changing resource limit policies
- **Key modules:** `limiter.rs`, `memory.rs`
- **Depends on:** `wasmer`, `ironclaw_resources`

### ironclaw_mcp
**Role:** Model Context Protocol support
- MCP server discovery
- Protocol implementation
- Tool mapping to MCP capabilities
- **When to touch:** Adding MCP features or new protocol versions
- **Key modules:** `discovery.rs`, `protocol.rs`
- **Depends on:** `ironclaw_host_api`, `ironclaw_extensions`

### ironclaw_scripts
**Role:** Script execution (Python, Bash, etc.)
- CodeAct (Code Action) execution
- Inline script support
- Script sandboxing and limits
- **When to touch:** Adding new script languages or execution modes
- **Key modules:** `lib.rs`, `executor.rs`
- **Depends on:** `ironclaw_host_api`, `ironclaw_process_sandbox`

### ironclaw_extensions
**Role:** Extension lifecycle and discovery
- Manifest parsing (capabilities, metadata)
- Installation/activation/removal flow
- Discovery (installed extensions, available extensions)
- Version management
- **When to touch:** Changing extension format or lifecycle
- **Key modules:** `registry.rs`, `lifecycle.rs`, `v2.rs`
- **Tests:** `tests/extension_contract.rs`, `tests/manifest_v2_contract.rs`
- **Depends on:** `ironclaw_host_api`, `ironclaw_capabilities`

### ironclaw_host_runtime
**Role:** Host-side effect execution
- Shell execution (subprocess)
- HTTP requests
- File I/O
- External service calls
- **When to touch:** Adding new host-side effect types
- **Key modules:** `lib.rs`
- **Tests:** `tests/` with subprocess/HTTP/IO contracts
- **Depends on:** `ironclaw_host_api`, `tokio`, `reqwest`

### ironclaw_processes
**Role:** Process sandbox and subprocess management
- Subprocess isolation
- I/O capture (stdout, stderr)
- Resource limits (memory, CPU time)
- **When to touch:** Changing subprocess policy or isolation
- **Key modules:** `sandbox.rs`, `lib.rs`
- **Depends on:** `tokio`, `ironclaw_resources`

### ironclaw_first_party_extensions
**Role:** Built-in tools (GitHub, Google Drive, etc.)
- GitHub tool (issues, PRs, repos)
- Google Drive, Sheets, Docs, Slides
- Notion, Slack (as tools)
- WASM-compiled tools
- **When to touch:** Adding new tools or modifying existing ones
- **Key modules:** `assets/` (manifests, schemas, prompts)
- **Assets format:** Manifests in `assets/*/manifest.toml`, prompts in `assets/*/prompts/`, schemas in `assets/*/schemas/`
- **Build:** Requires WASM compilation; run `./scripts/build-extensions.sh`
- **Depends on:** WASM compiler, tool SDKs (github-rs, google-api-rs, etc.)

---

## Durable State & Events (9 crates)

**Purpose:** Persistence, event sourcing, and state recovery.

### ironclaw_events
**Role:** Immutable event log
- Event types (capability executed, approval requested, etc.)
- Event serialization (JSONL format)
- Event cursor (position in log)
- **When to touch:** Adding new event types or changing schema
- **Key modules:** `lib.rs`, `runtime_event.rs`
- **Backends:** JSONL file, in-memory (for tests)
- **Tests:** `tests/durable_log_contract.rs`
- **Depends on:** `serde_json`, `ironclaw_common`

### ironclaw_event_projections
**Role:** Snapshot computation and caching
- Projection system (computes snapshots from events)
- Pending gate projection (what's waiting for approval?)
- Runtime checkpoint cache
- State derivation
- **When to touch:** Adding new projections or snapshot types
- **Key modules:** `pending_gate_projection.rs`, `runtime_projection.rs`
- **Tests:** `tests/memory_prompt_safety_projection_contract.rs`, `tests/replay_projection_contract.rs`
- **Depends on:** `ironclaw_events`, `ironclaw_common`

### ironclaw_event_streams
**Role:** Event subscription and delivery
- Event filtering (only send matching events)
- Redaction (remove sensitive data)
- Admission control (who can subscribe to what?)
- Subscription management
- **When to touch:** Adding new event filters or admission rules
- **Key modules:** `manager.rs`, `redaction.rs`, `admission.rs`
- **Tests:** `tests/event_stream_manager_contract.rs`
- **Depends on:** `ironclaw_events`, `ironclaw_common`

### ironclaw_reborn_event_store
**Role:** Backend-agnostic event storage
- Trait: `EventStore`
- Implementations: PostgreSQL, libSQL (Turso)
- Migration support
- **When to touch:** Adding new backends or storage operations
- **Key modules:** `lib.rs`
- **Features:** `postgres`, `libsql` (enable both for dual-backend testing)
- **Depends on:** `ironclaw_events`, database drivers

### ironclaw_run_state
**Role:** Checkpoint and recovery state
- Checkpoint storage (save loop progress)
- Recovery state (resume from checkpoint)
- Step tracking
- **When to touch:** Changing checkpoint format or recovery logic
- **Key modules:** `lib.rs`
- **Depends on:** `ironclaw_events`, `ironclaw_common`

### ironclaw_threads
**Role:** Thread (conversation) lifecycle and metadata
- Thread creation and archiving
- Thread metadata (owner, created_at, tags)
- Active-thread locking (only one loop per thread at a time)
- **When to touch:** Adding new thread metadata or lifecycle states
- **Key modules:** `lib.rs`, `contract.rs`
- **Backends:** PostgreSQL, libSQL
- **Tests:** Thread contract tests
- **Depends on:** `ironclaw_common`

### ironclaw_conversations
**Role:** Conversation state and message store
- Message history (turns within a thread)
- State machine (new, active, completed, error)
- Trusted inbound (secure entry point)
- **When to touch:** Adding new message types or state transitions
- **Key modules:** `lib.rs`, `state_store.rs`, `trusted_trigger.rs`
- **Tests:** `tests/inbound_contract.rs`, `tests/filesystem_store_contract.rs`
- **Depends on:** `ironclaw_threads`, `ironclaw_common`

### ironclaw_memory
**Role:** Embedding index and semantic search
- Embedding provider abstraction
- Vector database (PostgreSQL pgvector, Bedrock, Pinecone, etc.)
- Semantic search over conversation history
- **When to touch:** Adding new memory backends or search algorithms
- **Key modules:** `lib.rs`, `retrieval.rs`, `skill_tracker.rs`
- **Depends on:** `ironclaw_embeddings`, `ironclaw_common`

### ironclaw_memory_native
**Role:** Native (on-disk) memory implementation
- File-based memory store (lightweight)
- For local/dev deployments
- **When to touch:** Optimizing native memory performance
- **Depends on:** filesystem I/O, `ironclaw_memory`

---

## Products & Loops (27 crates)

**Purpose:** Agent loops, product surfaces, and user-facing features.

### ironclaw_agent_loop
**Role:** Core agent executor
- Planning phase (what should we do?)
- Execution phase (run tools, call LLM)
- Checkpointing (save progress)
- Loop exit criteria
- Compaction (summarize history to save tokens)
- **When to touch:** Changing loop logic, adding loop strategies, or modifying planning
- **Key modules:** `executor.rs`, `planner.rs`, `state.rs`, `executor/strategies/`
- **Tests:** `tests/executor_happy_paths.rs`, `tests/strategy_interactions.rs`, `tests/safety_nets.rs`
- **Depends on:** `ironclaw_host_api`, `ironclaw_capabilities`, `ironclaw_llm`

### ironclaw_loop_support
**Role:** Utilities for loop implementations
- Candidate generation (tools to consider)
- Score aggregation (which tool is best?)
- Loop context (system state)
- **When to touch:** Adding new loop utilities or strategies
- **Depends on:** `ironclaw_agent_loop`, `ironclaw_llm`

### ironclaw_executor
**Role:** (v1) Legacy executor
- Old loop implementation (pre-Reborn)
- In maintenance mode
- **Status:** Do not extend; Reborn replaces this
- **When to touch:** Only to maintain existing v1 behavior
- **Depends on:** `src/` v1 types

### ironclaw_turns
**Role:** Turn (interaction) state and sequencing
- Turn message composition
- Turn history tracking
- Submission handling (new user input)
- **When to touch:** Adding new turn types or message fields
- **Key modules:** `lib.rs`, `message.rs`
- **Depends on:** `ironclaw_common`

### ironclaw_llm
**Role:** LLM provider abstraction
- Provider trait (different LLM APIs)
- Model selection and routing
- Prompt formatting (OpenAI, Anthropic, etc.)
- Token counting
- **Supported providers:** OpenAI, Anthropic, Ollama, OpenRouter, OpenAI-compatible, Bedrock, etc.
- **When to touch:** Adding new LLM provider, changing token counting, or model routing
- **Key modules:** `provider.rs`, `model.rs`
- **Tests:** Provider contract tests
- **Depends on:** `ironclaw_common`, `reqwest`

### ironclaw_embeddings
**Role:** Embedding provider abstraction
- Embedding provider trait
- Model selection
- Batch processing
- **Supported providers:** OpenAI, Bedrock, Ollama, etc.
- **When to touch:** Adding new embedding provider or changing pooling strategy
- **Key modules:** `provider.rs`, `factory.rs`
- **Tests:** Provider contract tests
- **Depends on:** `ironclaw_common`, `reqwest`

### ironclaw_engine
**Role:** (v1) Legacy orchestration
- Old runtime orchestration logic
- Prompt envelope, memory, skills
- In maintenance mode
- **Status:** Do not extend; Reborn replaces this
- **When to touch:** Only to maintain existing v1 behavior
- **Depends on:** `src/` v1 types, database

### ironclaw_reborn
**Role:** Reborn runtime kernel
- TurnCoordinator (serialization, locking)
- CapabilityHost (effect execution, gating)
- Snapshot management
- **When to touch:** Changing core kernel behavior (rare)
- **Key modules:** `lib.rs`, `coordinator.rs`, `host.rs`
- **Tests:** Core runtime contract tests
- **Depends on:** `ironclaw_host_api`, `ironclaw_capabilities`, `ironclaw_agent_loop`

### ironclaw_reborn_cli
**Role:** Primary CLI/WebUI binary entrypoint
- `ironclaw-reborn` binary
- Commands: `run`, `repl`, `serve`, `models`, `config`, `doctor`, `doctor-profile`
- CLI argument parsing
- **When to touch:** Adding new commands or CLI flags
- **Key modules:** `main.rs`, `commands/`
- **Build:** `cargo build -p ironclaw_reborn_cli --bin ironclaw-reborn`
- **Features:** `webui-v2-beta` (for serve command), `slack-v2-host-beta` (for Slack)
- **Depends on:** `ironclaw_reborn`, `ironclaw_reborn_config`, `clap`

### ironclaw_reborn_config
**Role:** Configuration parsing and resolution
- `config.toml` parsing
- Environment variable resolution
- Profile selection (local-dev, production, etc.)
- Defaults and validation
- **When to touch:** Adding new config fields or profiles
- **Key modules:** `lib.rs`, `config.rs`
- **Config schema:** See README.md or crate docs for TOML format
- **Depends on:** `serde`, `toml`, `ironclaw_runtime_policy`

### ironclaw_reborn_composition
**Role:** Dependency injection and app builder
- AppBuilder (wires database, LLM, tools, etc.)
- Service registration
- Feature flag handling
- **When to touch:** Adding new services or changing composition logic
- **Key modules:** `lib.rs`, `builder.rs`
- **Depends on:** All infrastructure crates

### ironclaw_reborn_identity
**Role:** User/owner identity and session management
- Owner identification
- Session creation and validation
- **When to touch:** Changing identity model or session format
- **Depends on:** `ironclaw_common`

### ironclaw_reborn_traces
**Role:** Trace recording and replay
- Record execution traces (for testing and debugging)
- Replay traces (deterministic test fixtures)
- Trace serialization
- **When to touch:** Adding new trace event types
- **Key modules:** `lib.rs`, `recorder.rs`
- **Depends on:** `ironclaw_common`

### ironclaw_reborn_openai_compat
**Role:** OpenAI-compatible API surface
- `/v1/chat/completions` endpoint
- Streaming responses
- Token counting
- **When to touch:** Exposing new endpoints or changing API format
- **Key modules:** `lib.rs`
- **Tests:** OpenAI API compatibility contract
- **Depends on:** `ironclaw_reborn`, `ironclaw_llm`

### ironclaw_reborn_openai_compat_storage
**Role:** PostgreSQL/libSQL adapters for OpenAI API
- Backend for OpenAI-compatible conversation storage
- **When to touch:** Adding new storage operations for API
- **Depends on:** `ironclaw_reborn_openai_compat`, database drivers

### ironclaw_reborn_webui_ingress
**Role:** WebUI HTTP routing and session management
- Session cookies and auth
- CORS configuration
- OAuth integration
- **When to touch:** Adding new routes or changing auth model
- **Key modules:** `lib.rs`, `routes.rs`
- **Depends on:** `ironclaw_gateway` (routing), `ironclaw_oauth`

### ironclaw_gateway
**Role:** (v1) HTTP gateway / (Reborn) routing utilities
- HTTP endpoint definitions
- Request/response serialization
- WebSocket management
- **Status:** Mostly ported to Reborn; v1 in maintenance mode
- **When to touch:** For new Reborn API endpoints
- **Key modules:** `lib.rs`
- **Depends on:** `axum`, `tokio`, `ironclaw_common`

### ironclaw_product_context
**Role:** Product-specific request context
- User metadata (who's making the request?)
- Tenant context (isolation)
- Request enrichment
- **When to touch:** Adding new context fields
- **Depends on:** `ironclaw_common`, `ironclaw_trust`

### ironclaw_product_workflow
**Role:** Missions, projects, skills, routines, approvals
- **Missions:** Goals the agent is trying to accomplish
- **Projects:** User-created containers for missions
- **Skills:** Learned behaviors the agent has acquired
- **Routines:** Automated tasks that run periodically
- **Approvals:** Human sign-off on dangerous operations
- **When to touch:** Changing mission/project/skill model or approval policies
- **Key modules:** `lib.rs`, `mission.rs`, `project.rs`, `approval.rs`
- **Tests:** Product workflow contract tests
- **Depends on:** `ironclaw_product_context`, `ironclaw_approvals`, `ironclaw_skill_learning`

### ironclaw_product_adapters
**Role:** Product adapter framework
- Adapter trait (interface for channel adapters)
- Request/response handling
- Registry
- **When to touch:** Adding new adapter types or framework features
- **Key modules:** `lib.rs`, `adapter.rs`
- **Depends on:** `ironclaw_host_api`, `ironclaw_product_context`

### ironclaw_product_adapter_registry
**Role:** Adapter discovery and lifecycle
- Install/activate/remove flows
- Adapter discovery (installed vs. available)
- Version management
- **When to touch:** Changing adapter lifecycle
- **Depends on:** `ironclaw_product_adapters`, `ironclaw_extensions`

### ironclaw_wasm_product_adapters
**Role:** WASM-based adapter implementations
- Adapters compiled to WASM (sandboxed)
- **When to touch:** Adding new WASM adapters
- **Depends on:** `ironclaw_wasm`, `ironclaw_product_adapters`

### ironclaw_slack_extension
**Role:** Slack workspace adapter
- Slack OAuth flow
- Slack message handling
- Slack tool exposure
- **When to touch:** Adding new Slack features
- **Key modules:** `lib.rs`
- **Tests:** Slack adapter contract tests
- **Depends on:** `ironclaw_product_adapters`, `slack-morphism` or similar

### ironclaw_telegram_extension
**Role:** Telegram bot adapter
- Telegram bot API
- Message routing
- **When to touch:** Adding new Telegram features
- **Key modules:** `lib.rs`
- **Depends on:** `ironclaw_product_adapters`, `telegram-bot` or similar

### ironclaw_outbound
**Role:** Outbound message delivery
- Send replies to user
- Notifications
- Message formatting per channel (Slack, Telegram, etc.)
- **When to touch:** Adding new message types or channels
- **Key modules:** `lib.rs`
- **Depends on:** `ironclaw_dispatcher`, `ironclaw_product_adapters`

### ironclaw_triggers
**Role:** Trigger system and event-driven automation
- Create triggers (on event X, do Y)
- Trigger evaluation
- Automation execution
- **When to touch:** Adding new trigger types or evaluation rules
- **Key modules:** `lib.rs`, `trigger.rs`
- **Depends on:** `ironclaw_events`, `ironclaw_dispatcher`

### ironclaw_skill_learning
**Role:** Skill extraction and classification
- Extract skills from user interactions
- Classify new missions (does this match existing skills?)
- Skill refinement and evolution
- **When to touch:** Changing skill extraction or classification
- **Key modules:** `lib.rs`, `extractor.rs`
- **Depends on:** `ironclaw_skills`, `ironclaw_llm`

---

## Storage Backends (8 crates)

**Purpose:** Database-agnostic persistence with PostgreSQL and libSQL adapters.

### ironclaw_hooks_postgres
**Role:** PostgreSQL event hook implementation
- PostgreSQL-specific event persistence
- Connection pooling
- Migration support (refinery)
- **When to touch:** Adding new database operations
- **Key modules:** `lib.rs`
- **Tests:** PostgreSQL contract tests
- **Depends on:** `tokio-postgres`, `deadpool-postgres`, `refinery`

### ironclaw_hooks_libsql
**Role:** libSQL/Turso event hook implementation
- libSQL-specific event persistence
- Embedded vs. remote (Turso) support
- Replication handling
- **When to touch:** Adding new database operations
- **Key modules:** `lib.rs`
- **Tests:** libSQL contract tests
- **Depends on:** `libsql`, `tokio`

### ironclaw_hooks_parity
**Role:** Feature parity testing across backends
- Ensures PostgreSQL and libSQL support the same operations
- Runs same contract tests on both backends
- **When to touch:** Adding new database operations (add contract test here first)
- **Key modules:** `tests/`
- **Tests:** Run with `cargo test -p ironclaw_hooks_parity`
- **Depends on:** `ironclaw_hooks_postgres`, `ironclaw_hooks_libsql`

### Dual-Backend Crates
These crates support both PostgreSQL and libSQL transparently:

- **ironclaw_reborn_event_store** — Event storage with `Db` trait
- **ironclaw_run_state** — Checkpoint storage
- **ironclaw_threads** — Thread metadata storage
- **ironclaw_conversations** — Conversation state storage
- **ironclaw_filesystem** (with `db.rs`) — File catalog storage

**Pattern:** Each has a `Db` trait; implementations for PostgreSQL and libSQL are registered at startup in `ironclaw_reborn_composition`.

---

## Utilities (7 crates)

**Purpose:** Cross-cutting concerns, logging, and integrations.

### ironclaw_observability
**Role:** Tracing, metrics, and structured logging
- Tracing subscriber setup
- Metrics collection (Prometheus format)
- Structured logging (JSON format in production)
- **When to touch:** Adding new metrics or changing tracing format
- **Key modules:** `lib.rs`
- **Depends on:** `tracing`, `tracing-subscriber`

### ironclaw_skills
**Role:** Skill system and definitions
- Skill data types
- Skill metadata
- Skill evaluation
- **When to touch:** Changing skill format or adding new skill properties
- **Key modules:** `lib.rs`
- **Depends on:** `ironclaw_common`

### ironclaw_oauth
**Role:** OAuth flow management
- OAuth token exchange
- Token refresh
- PKCE support
- **When to touch:** Adding new OAuth providers or flow types
- **Key modules:** `lib.rs`, `flow.rs`
- **Depends on:** `ironclaw_common`, `reqwest`

### ironclaw_auth
**Role:** Authentication and credential types
- Authentication method types
- Credential storage (transient)
- Auth state machine
- **When to touch:** Adding new auth methods
- **Key modules:** `lib.rs`, `credential.rs`
- **Tests:** `tests/auth_product_contract.rs`
- **Depends on:** `ironclaw_common`, `ironclaw_oauth`

### ironclaw_llm
**Role:** (Also in Products) LLM provider abstraction
- Used by both agent loops and general LLM invocation
- **See:** [Products & Loops: ironclaw_llm](#ironclaw_llm)

### ironclaw_embeddings
**Role:** (Also in Products) Embedding provider abstraction
- Used by both memory and skill learning
- **See:** [Products & Loops: ironclaw_embeddings](#ironclaw_embeddings)

### ironclaw_extractors
**Role:** Data extraction utilities
- Text extraction from attachments
- Structured data extraction
- **When to touch:** Adding new extraction methods
- **Depends on:** `ironclaw_common`

### ironclaw_tui
**Role:** Terminal UI components (if used)
- TUI rendering
- Interactive prompts
- **When to touch:** Changing CLI output or adding interactive features
- **Depends on:** `tokio`, TUI libraries (ratatui, etc.)

---

## Relationship to v1

The v1 monolith in `src/` coexists with Reborn but is being phased out:

- **v1 crates:** `ironclaw_executor`, `ironclaw_engine` (in `crates/`)
- **v1 code:** `src/` directory (agent, channels, db, extensions, tools, workspace, etc.)
- **Dual binary:** `src/main.rs` (legacy) and `crates/ironclaw_reborn_cli` (modern)

**When to use each:**
- **v1 (src/):** Only maintain existing v1 behavior; don't add features
- **Reborn (crates/):** All new features, product layer, modern patterns

For architectural decisions about v1, see the [Architecture Overview](overview.md#the-dual-stack-v1-and-reborn).

---

## Cross-Crate Patterns

### Pattern: Dual-Backend Support

```rust
// In ironclaw_reborn_composition startup:
let db: Box<dyn Db> = if use_postgres {
    Box::new(PostgresDb::new(...).await?)
} else {
    Box::new(LibSqlDb::new(...).await?)
};

// Throughout the system, code uses `db` as the abstraction
let threads = db.list_threads(...).await?;  // Works on both
```

**Relevant crates:** `ironclaw_hooks_postgres`, `ironclaw_hooks_libsql`, `ironclaw_hooks_parity`

### Pattern: LLM Provider Abstraction

```rust
// In composition:
let llm: Arc<dyn LlmProvider> = match config.llm_backend {
    "openai" => Arc::new(OpenAiProvider::new(...)),
    "anthropic" => Arc::new(AnthropicProvider::new(...)),
    ...
};

// Loops use it without knowing the specific provider
let response = llm.complete(request).await?;
```

**Relevant crates:** `ironclaw_llm`, composition in `ironclaw_reborn_cli`

### Pattern: Event Subscriptions

```rust
// In startup:
let subscriber = EventStreamManager::new(db.clone());

// Systems subscribe to relevant events
subscriber.subscribe(filter, |event| async {
    // Handle the event (e.g., update index, trigger automation)
}).await;

// When events occur, all subscribers are notified
```

**Relevant crates:** `ironclaw_events`, `ironclaw_event_streams`, `ironclaw_event_projections`

---

## Suggested Reading Order

1. **To understand the overall architecture:** Start with [overview.md](overview.md)
2. **To understand a specific crate:** Find it in this reference
3. **To add a new feature:** Use [overview.md: Where to Build New Features](overview.md#where-to-build-new-features) to pick a crate, then read its docs
4. **To understand data flow:** Read [data-model.md](data-model.md)
5. **To understand security:** Read [security.md](security.md)

---

## See Also

- **[Overview](overview.md)** — System design and four-layer model
- **[Data Model](data-model.md)** — Events, threads, turns, capabilities
- **[Security & Safety](security.md)** — Kernel boundary and threat model
- **[AGENTS.md](/AGENTS.md)** — Quick rules and code discovery
- **[CLAUDE.md](/CLAUDE.md)** — Subsystem deep-dives

---

**Last updated:** Auto-generated by OpenWiki. For corrections, file a PR.
