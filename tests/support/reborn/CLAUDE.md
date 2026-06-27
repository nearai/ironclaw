# Reborn Integration Tests

In-process tests that run a **whole Reborn turn** with the real internal stack —
product workflow, turn coordinator, scheduler, the agent loop, the real
`LlmProviderModelGateway` + the real `ironclaw_llm` decorator chain, and real
`RootFilesystem` persistence. The **only** thing faked is the bottom of the
stack: a scripted model at the vendor-SDK seam. A test reaches no network, no
real process, no real channel, and needs no setup.

This is a distinct tier from `RebornBinaryE2EHarness` (which swaps the whole
`HostManagedModelGateway` with `RebornTraceReplayModelGateway` at the *gateway*
seam, skipping `ironclaw_llm`). This tier mocks one layer lower so the decorator
chain runs for real.

## How it works

```
submit_turn(text)                      ← synthetic inbound (no HTTP, no Slack parsing)
   → product workflow → turn coordinator → scheduler → agent loop
       → LlmProviderModelGateway → real decorator chain (passthrough)
           → scripted model (TraceLlm fed an in-memory trace)   ← THE ONLY FAKE
   → turn state + assistant reply persisted to RootFilesystem (InMemory)
assert_* reads the persisted reply / state
```

The fake sits beneath the real chain (`apply_decorator_chain`), so
retry/routing/safety/`CompletionRequest`+tool-def assembly all execute. Mocking
higher (at the gateway) is wrong — it skips `ironclaw_llm`.

## Writing a test (the shape — always)

```rust
#[tokio::test]
async fn replies_to_greeting() {
    let h = RebornIntegrationHarness::test_default()
        .script([RebornScriptedReply::text("done")])
        .build().await.expect("harness builds");
    h.submit_turn("do something").await.expect("turn completes");
    h.assert_reply_contains("done").await.expect("reply finalized");
}
```

`build → submit_turn → assert`. Script each model turn with
`RebornScriptedReply::text(..)` — one line each. The harness is single-
conversation; `submit_turn`/`assert_reply_contains` take just the text.

## Requirements & expectations (non-negotiable)

1. **Test-first.** Write/update the test before the code; it must fail for the
   right reason first. (Root `CLAUDE.md` → Testing Discipline; `.claude/rules/testing.md`.)
2. **Consolidate.** Extend the existing test that covers a path; add a new test
   only for a genuinely distinct scenario, and say why.
3. **Readability contract.** ~3–12 lines, `build → submit_turn → assert`, no
   nested structs in the body. **Never** hand-build raw `TraceStep` /
   `LlmTrace::new` in a Reborn test — that is the verbosity the `RebornScriptedReply`
   façade removes.
4. **Mock only at the SDK seam.** Use `RebornScriptedReply`; do not swap the
   gateway or stub internals.
5. **Zero setup.** Must pass offline via a plain `cargo test --test reborn_<name>`
   — no services, no API keys, no `integration` feature, no Docker, no special
   linker. Hermetic env (keychain off, `TZ=UTC`, passthrough LLM config) is baked
   into `build()`.
6. **Edges captured/inert by default.** No real network/process/channel.
7. **Minimal setup.** Wire only the boundaries the scenario crosses. Don't stand
   up DB/HTTP/process capture for a text-only turn.
8. **Test through the real path**, asserting on the persisted reply / recorded
   boundary calls / state — not on internals.

## Files

- `scripted_provider.rs` — `scripted_trace_llm(..)`, the `TraceLlm` raw-provider seam.
- `reply.rs` — `RebornScriptedReply` (the one-line-per-turn façade).
- `builder.rs` — `RebornIntegrationHarness` + builder, hermetic env, the
  `assert_reply_contains` assertion (co-located with the harness fields).
- Tests live as flat `tests/reborn_*.rs` (Cargo requires top-level test files).

Design: `docs/superpowers/specs/2026-06-26-reborn-integration-test-framework-design.md`.

## Implemented now vs planned

Slice 1 ships the spine + one text-reply test. **Planned (do not assume present;
add behind a test that exercises it — no dead code):**
`RebornScriptedReply::tool_call(..)` + the CapabilityId→ProviderToolName mapping;
`StorageMode::LibSql` (real SQLite on tmp) and the InMemory-vs-libSQL backend
matrix; inert process port + `.with_live_shell()` / `.with_live_http_egress()`
opt-ins; outbound/HTTP/secrets/MCP capture wiring; a dedicated `assertions.rs`
once the `assert_*` family grows; the pre-commit test-style check. A
divergence between InMemory and libSQL, once the matrix lands, is a real
persistence bug — not test flake.
