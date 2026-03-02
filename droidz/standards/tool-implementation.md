# Tool Implementation Standard (AI Agents)

## Required

- Keep tool-specific behavior inside tool modules, not core agent flow.
- Implement the `Tool` contract consistently:
  - stable `name()`
  - clear `description()`
  - explicit `parameters_schema()`
  - structured `execute()` result via `ToolOutput`
- Set `requires_sanitization()` to `true` for tools that consume external/untrusted data.

## Capability design

- Declare auth/network/secret needs via capability metadata where applicable.
- Avoid hardcoding service-specific auth flow into unrelated modules.

## Safety

- Do not leak secrets into logs/tool output.
- Preserve sanitization and policy checks for external content.
