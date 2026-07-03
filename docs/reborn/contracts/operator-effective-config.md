# Reborn Operator Effective Config Contract

Issue: #4593

This document pins the WebUI v2 operator config surface that follows `GET /api/webchat/v2/operator/config`, `GET /api/webchat/v2/operator/config/{key}`, `POST /api/webchat/v2/operator/config/{key}`, and `POST /api/webchat/v2/operator/config/validate`.

## Supported first slice

The first implementation slice should expose the LLM default route through the existing typed LLM config service. The operator config facade must not bypass config layering or write TOML/secrets directly.

| Key | Read | Write | Value shape | Source metadata | Redaction |
| --- | --- | --- | --- | --- | --- |
| `provider.default` | yes | yes | string provider id or `null` | active LLM selection | not redacted |
| `model.default` | yes | yes | string model id or `null` | active LLM selection | not redacted |
| `provider.api_key` | yes | yes when active provider accepts API keys | redacted secret state | secret store or env metadata only | value always serialized as `null` |

## Precedence

Responses should include precedence metadata in this order:

1. Bootstrap environment and immutable process config.
2. Operator config file/default slot.
3. Secret store or provider credential environment metadata.
4. Provider catalog defaults.

The API should report effective values after this precedence has been applied. It must not collapse bootstrap config, DB-backed settings, and encrypted secrets into a single untyped layer.

## Required behavior

- `list` returns every supported key with `key`, `value`, `source`, `redacted`, and `mutable`.
- `get` returns one supported key and returns a loud error for unknown keys.
- `set provider.default` delegates to `LlmConfigService::set_active`.
- `set model.default` delegates to `LlmConfigService::set_active` using the active provider id.
- `set provider.api_key` delegates to `LlmConfigService::upsert_provider` using the active provider metadata and never echoes the submitted secret.
- `validate` returns an empty diagnostics list for supported keys that are currently writable.
- Unsupported, deprecated, immutable, missing-active-provider, and secret-not-supported keys return stable diagnostics.

## Key validation constraints

All operator config keys must satisfy the boundary-level validation rules:

- Must not be empty.
- Must not exceed 128 bytes. The limit is byte-based, matching the handler boundary check.
- Must not be the reserved word `validate`.
- Must contain only ASCII lowercase letters, digits, underscores (`_`), dots (`.`), or hyphens (`-`).

## Stable diagnostic reason codes

Existing reason codes remain stable:

- `operator_config_service_not_wired`
- `operator_config_secret_not_wired`
- `operator_config_deprecated`
- `operator_config_immutable`
- `operator_config_not_wired`
- `operator_config_unknown_key`

The LLM-backed slice may additionally return:

- `operator_config_missing_active_provider`
- `operator_config_secret_not_supported`

## Test requirements

Implementation PRs must drive the caller path, not only helper functions:

- Service-facade tests for list, get, set, and validate using a fake `LlmConfigService`.
- Router/handler tests proving key validation and secret-bearing bodies do not echo raw values.
- Redaction tests for `provider.api_key` serialization.
- Failure-path tests for unknown keys, malformed values, missing active provider, and providers that do not accept API keys.
- Precedence assertions on list responses.
