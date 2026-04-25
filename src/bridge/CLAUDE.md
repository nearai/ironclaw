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

## Engine-v2 enablement contract

For engine v2, blocked managed integrations are not part of the normal
model-facing callable action list. They surface in capability background
under `Activatable Integrations`, and the single model-facing enablement
path is `tool_activate(name=...)`.

`tool_activate` may internally install, authenticate, or activate the
integration as needed. `tool_install` and `tool_auth` still exist as
runtime/compatibility surfaces, but they are not the normal v2 prompt
contract. After `tool_activate` succeeds, the newly ready tools appear on
the next top-level turn rather than mid-CodeAct step.

## Auth-flow extension resolution: one place, no re-derivation

The single authority that maps an auth gate or tool-call context to the installed extension identity is the free function:

**`bridge::auth_manager::resolve_auth_flow_extension_name(action_name, params, credential_fallback, user_id, tool_registry, extension_manager) -> ExtensionName`**

Its precedence order:

1. **User-influenced** — explicit `name` param on enablement/auth tool invocations such as `tool_activate` (and internal/compat paths like `tool_install` / `tool_auth`). This comes from the model or caller arguments, so it's validated via `ExtensionName::new`; invalid values fall through.
2. The action's provider extension, via `ToolRegistry::provider_extension_for_tool`.
3. Canonicalized `action_name` if the extension manager has an installed extension by that name.
4. The caller-supplied `credential_fallback` — last-resort, used only when no extension owns the action.

Every surface that needs an extension name for auth flow MUST call this free function (or delegate through a thin wrapper). The approved wrappers are:

- `AuthManager::resolve_extension_name_for_auth_flow(...) -> ExtensionName` — delegates with `self.tools` and `self.extension_manager`. Used by `bridge::router`.
- `bridge::router::resolve_auth_gate_extension_name(pending) -> Option<ExtensionName>` — used for `GateRequired` SSE and `send_pending_gate_status`.
- `channels::web::features::chat::pending_gate_extension_name(state, ...) -> Option<ExtensionName>` — used for `HistoryResponse.pending_gate` and rehydration. Calls the free function directly so the bare-test-harness path (no `AuthManager` built yet) still runs every branch, not a drift-prone subset.

Wrappers **delegate**; they must not duplicate the precedence rules, reconstruct names from credential prefixes, or fall back to `format!()`-built strings.

**Why it's centralized:** four identity-confusion bugs (#2561, #2473, #2512, #2574) were the same pattern — two layers independently mapping credential→extension, each reaching a different answer when either one drifted. Newtypes (`CredentialName`, `ExtensionName`) prevent the *type* mix-up; this invariant prevents the *value* mix-up. PR #2617 (Copilot review on `server.rs:1420`) caught a near-fifth: the `pending_gate_extension_name` wrapper's no-auth-manager fallback had grown a three-branch copy of the resolver's precedence that quietly skipped branch 3 (canonicalize + installed-extension check). Extracting the free function collapsed the duplicate and restored the invariant.

If you think you need a new derivation path, stop and consolidate into the shared resolver instead. See `.claude/rules/types.md` ("Typed Internals") and `src/channels/web/CLAUDE.md` ("Identity types at the web boundary") for the broader rule.
