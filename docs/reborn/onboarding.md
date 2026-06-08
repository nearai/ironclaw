# Reborn Onboarding

This document describes the standalone `ironclaw-reborn onboard` surface.
It is Reborn-owned and must not call into v1 `src/setup`, v1 database
configuration, v1 channels, or v1 import state.

## Current Slice

`ironclaw-reborn onboard` is a first-run bootstrap command for the standalone
Reborn binary. It currently:

1. resolves `IRONCLAW_REBORN_HOME` or the default `~/.ironclaw/reborn`;
2. creates the Reborn home directory;
3. creates missing `config.toml` and `providers.json` using the same atomic
   writer as `ironclaw-reborn config init`;
4. preserves existing operator-edited config files unless `--force` is passed;
5. writes `.onboard-completed.json` in Reborn home; and
6. prints explicit remaining setup work.

The completion marker schema is:

```json
{
  "schema_version": "ironclaw.reborn.onboarding/v1",
  "completed_at": "RFC3339 timestamp",
  "reborn_home": "/absolute/path",
  "home_source": "IRONCLAW_REBORN_HOME or default",
  "config_file": "/absolute/path/config.toml",
  "providers_file": "/absolute/path/providers.json",
  "steps_completed": ["reborn_home", "config_files", "completion_marker"],
  "steps_pending": ["llm_credentials", "model_selection", "channel_setup"],
  "v1_state": "not-used"
}
```

## Non-Goals In This Slice

- No v1 `src/setup/wizard.rs` reuse.
- No automatic first-run invocation before `ironclaw-reborn run`.
- No interactive provider credential prompts.
- No keychain or encrypted secret setup for LLM keys.
- No model picker.
- No channel, extension, or WebUI setup flow.
- No conversation-history import.

Passing `--import-history` records history import as pending and reports it in
the command output. It does not read external exports or write transcripts yet.

## Expected Follow-Up Shape

Future onboarding work should extend this Reborn-owned command instead of
adding Reborn behavior to v1 setup:

1. add an interactive prompt layer under `crates/ironclaw_reborn_cli`;
2. route provider/model writes through `RebornProviderAdmin`;
3. route product credential setup through Reborn product-auth facades;
4. add a history-import step after Reborn home/storage initialization; and
5. only then consider first-run auto-detection before `run`.

Every new step should keep the Reborn CLI boundary intact: commands may use
`RebornCliContext` and facade-shaped composition APIs, but must not import v1
runtime, v1 setup, v1 DB, or v1 channel modules.
