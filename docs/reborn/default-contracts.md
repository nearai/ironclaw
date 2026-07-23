# Default IronClaw Contracts and Legacy Compatibility

Issue: #6551

IronClaw is the default shipping product. New configuration and runtime
integrations should use the neutral contracts below. The former Reborn names
remain compatibility aliases during the migration window.

## Environment precedence

For every runtime variable whose previous name began with
`IRONCLAW_REBORN_`, remove `REBORN_` to obtain the preferred name:

```text
IRONCLAW_HOME                 > IRONCLAW_REBORN_HOME
IRONCLAW_PROFILE              > IRONCLAW_REBORN_PROFILE
IRONCLAW_WEBUI_TOKEN          > IRONCLAW_REBORN_WEBUI_TOKEN
IRONCLAW_RUNNER_WORKER_COUNT  > IRONCLAW_REBORN_RUNNER_WORKER_COUNT
```

The preferred name wins when both are set. Legacy names remain accepted and
are never logged with their values. This rule also covers prefix families such
as `IRONCLAW_WEBUI_*`, `IRONCLAW_RUNNER_*`, and
`IRONCLAW_DEV_SECRET__*`.

The container entrypoint applies the same precedence before it resolves
shell-owned controls such as `HOME`, `PROFILE`, `DEFAULT_CONFIG`, `SERVE_HOST`,
`SERVE_PORT`, and the Railway safety switches. Existing deployment manifests
therefore continue to boot while new examples use only the neutral names.

## Home directory

Resolution order is:

1. `IRONCLAW_HOME`;
2. `IRONCLAW_REBORN_HOME`;
3. an existing `~/.ironclaw/reborn` directory, used in place without moving or
   copying data;
4. `~/.ironclaw` for a new installation.

The resolver is side-effect free. It does not automatically merge or delete
either directory. Rollback is therefore configuration-only: unset
`IRONCLAW_HOME` or point `IRONCLAW_REBORN_HOME` at the prior directory.

## Product-auth HTTP routes

New clients use `/api/product-auth/*`. The corresponding
`/api/reborn/product-auth/*` routes remain mounted to the same handlers with
the same authentication, body limits, rate limits, scope derivation, CORS,
audit class, and effect path. Existing provider redirect URIs therefore remain
valid during the migration window.

## OS service identities

New installs use:

- launchd: `com.ironclaw`
- systemd: `ironclaw.service`

The CLI continues to detect, start, stop, inspect, restart, and uninstall
existing `com.ironclaw.reborn` and `ironclaw-reborn.service` installations.
It prefers the neutral identity when both files exist.

The CLI does not silently install a second service or destructively rewrite an
existing legacy definition. To migrate explicitly, uninstall the legacy
service with the CLI and then run `ironclaw service install`; the newly
installed service uses the neutral identity.

## Deprecation and rollback

This change does not delete legacy state, environment support, routes, or
service definitions. Removal of any compatibility alias requires a separately
announced deprecation with upgrade telemetry and rollback evidence. To roll
back this release, continue using the legacy environment names, home path,
service definition, and product-auth callback URLs.
