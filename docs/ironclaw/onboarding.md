# IronClaw Onboarding

This document describes the standalone `ironclaw onboard` surface.
It is IronClaw-owned and must not call into v1 `src/setup`, v1 database
configuration, v1 channels, or v1 import state.

## Current Slice

`ironclaw onboard` is a first-run bootstrap command for the standalone
IronClaw binary. It currently:

1. resolves `IRONCLAW_HOME`, then the legacy `IRONCLAW_REBORN_HOME`, then an
   existing `~/.ironclaw/reborn`, or the new-install default `~/.ironclaw`;
2. creates the IronClaw home directory;
3. creates missing `config.toml` and `providers.json` using the same atomic
   writer as `ironclaw config init`;
4. preserves existing operator-edited config files unless `--force` is passed;
5. writes `.onboard-completed.json` in IronClaw home; and
6. prints explicit remaining setup work.

The completion marker schema is:

```json
{
  "schema_version": "ironclaw.reborn.onboarding/v1",
  "completed_at": "RFC3339 timestamp",
  "ironclaw_home": "/absolute/path",
  "home_source": "IRONCLAW_HOME, IRONCLAW_REBORN_HOME, legacy-default, or default",
  "config_file": "/absolute/path/config.toml",
  "providers_file": "/absolute/path/providers.json",
  "steps_completed": ["ironclaw_home", "config_files", "completion_marker"],
  "steps_pending": ["llm_credentials", "model_selection", "channel_setup"],
  "v1_state": "not-used"
}
```

## NEAR AI MCP Auto-Bootstrap

Standalone IronClaw local-dev startup detects `NEARAI_BASE_URL` plus
`NEARAI_API_KEY` when both are present and valid. In that case the local-dev
composition stores the API key through IronClaw product-auth manual-token storage,
installs the bundled `nearai` MCP extension if it has not been installed yet,
and activates it so `nearai.web_search` is model-visible without a separate
extension setup step. Runtime credential resolution treats this account as a
host-managed credential for the bundled `nearai` requester, so admitted WebUI
SSO users in the same tenant/agent scope can call `nearai.web_search` without
each storing a separate NEAR AI API key. If the host-managed credential is
project-scoped, runtime use must be in that same project; a tenant/agent-level
host credential covers project-scoped runtime calls in that tenant/agent. Other
requesters, providers, and host identity scopes do not see it. Existing explicit
disabled extension state is preserved; users can disable NEAR AI MCP after
bootstrap and startup will not re-enable it.

Legacy IronClaw startup also uses the same env pair to bootstrap the persisted
`nearai` MCP server config described in `.env.example`.

## Non-Goals In This Slice

- No v1 `src/setup/wizard.rs` reuse.
- No automatic first-run invocation before `ironclaw run`.
- No interactive provider credential prompts.
- No keychain or encrypted secret setup for LLM keys.
- No model picker.
- No channel, extension, or WebUI setup flow.
- No conversation-history import.

Passing `--import-history` records history import as pending and reports it in
the command output. It does not read external exports or write transcripts yet.

## Expected Follow-Up Shape

Future onboarding work should extend this IronClaw-owned command instead of
adding IronClaw behavior to v1 setup:

1. add an interactive prompt layer under `crates/ironclaw_cli`;
2. route provider/model writes through `IronClawProviderAdmin`;
3. route product credential setup through IronClaw product-auth facades;
4. add a history-import step after IronClaw home/storage initialization; and
5. only then consider first-run auto-detection before `run`.

Every new step should keep the IronClaw CLI boundary intact: commands may use
`IronClawCliContext` and facade-shaped composition APIs, but must not import v1
runtime, v1 setup, v1 DB, or v1 channel modules.
