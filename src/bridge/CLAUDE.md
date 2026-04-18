# Bridge Module

Adapter layer between the engine v2 (`ironclaw_engine`) and the host
crate's execution, auth, LLM, and persistence surfaces. Channels,
handlers, and tool runtimes must not re-implement auth or identity
resolution — they call through these adapters.

## Files

| File | Role |
|------|------|
| `auth_manager.rs` | Centralized authentication state machine. Pre-flight credential checks, setup instruction lookup, auth-flow extension-name resolution. **Single source of truth for turning a credential/action into an `ExtensionName`.** |
| `router.rs` | `handle_with_engine()` — maps engine outcomes to channel responses. Owns auth-gate display + submit target resolution. |
| `effect_adapter.rs` | Implements `EffectExecutor` for the engine. Wraps the host `ToolRegistry` with safety + rate limits. |
| `llm_adapter.rs` | Implements `LlmBackend` for the engine. |
| `store_adapter.rs` | Implements `Store` for the engine (threads, steps, events, memory docs). |
| `cost_guard_gate.rs` | Engine gate that checks cost budget before LLM calls. |
| `skill_migration.rs` | One-shot migration of legacy skill metadata into the engine's capability registry. |
| `workspace_reader.rs` | Read-side adapter between the engine memory store and the workspace. |

## Auth-flow extension resolution: one place, no re-derivation

**`AuthManager::resolve_extension_name_for_auth_flow(action_name, params, credential_fallback, user_id) -> String`** is the single authority that maps an auth gate or tool-call context to the installed extension identity.

The resolver's precedence order (defined in `auth_manager.rs`):

1. Explicit `name` param on `tool_install` / `tool_activate` / `tool_auth` invocations.
2. The action's provider extension, via `ToolRegistry::provider_extension_for_tool`.
3. Canonicalized `action_name` if the extension manager has an installed extension by that name.
4. The caller-supplied `credential_fallback` — last-resort, used only when no extension owns the action.

Every surface that needs an extension name for auth flow MUST call this function (or a thin wrapper around it). The approved wrappers are:

- `bridge::router::resolve_auth_gate_extension_name(pending) -> Option<ExtensionName>` — used for `GateRequired` SSE and `send_pending_gate_status`.
- `channels::web::server::pending_gate_extension_name(state, ...) -> Option<ExtensionName>` — used for `HistoryResponse.pending_gate` and rehydration.

Both wrappers **delegate** to the canonical resolver; they must not duplicate its precedence rules, reconstruct names from credential prefixes, or fall back to `format!()`-built strings.

**Why it's centralized:** four identity-confusion bugs (#2561, #2473, #2512, #2574) were the same pattern — two layers independently mapping credential→extension, each reaching a different answer when either one drifted. Newtypes (`CredentialName`, `ExtensionName`) prevent the *type* mix-up; this invariant prevents the *value* mix-up.

If you think you need a new derivation path, stop and consolidate into the shared resolver instead. See `.claude/rules/types.md` ("Typed Internals") and `src/channels/web/CLAUDE.md` ("Identity types at the web boundary") for the broader rule.
