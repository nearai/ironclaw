# Enabling skills-with-scripts on hosted multi-tenant

Everything about skills works on hosted multi-tenant today **except executing a script a skill
carries**. This is the checklist for turning that on once the tenant sandbox lands, and the reason
each piece is currently off.

## What already works there

- Skills live wholly in the database. `production_database_root_filesystem` routes `/tenants`,
  `/projects`, `/memory` and `/system/*` to Postgres, so there is no host disk and no second
  namespace to disagree with — see `crates/ironclaw_reborn_composition/src/filesystem_assembly.rs`.
- The bundled skills are seeded into that database at boot
  (`ensure_bundled_reborn_skills_installed_in`), so a fresh tenant has all of them.
- An agent can author a skill **including** `scripts/*.py`, it persists, Settings → Skills reports
  `has_scripts`, and a later conversation activates it. Verified end to end by
  `scripts/e2e-skill-self-creation.sh` with `E2E_PROFILE=multi-tenant`.
- Skill files are readable through the ordinary filesystem tools (`read_file`, `glob`, `list_dir`),
  read-only. Writes stay with `skill_install`/`skill_update`, which validate the manifest that
  discovery requires.

## Why execution is off

`DeploymentMode::HostedMultiTenant` + `RuntimeProfile::SecureDefault` resolves to
`ProcessBackendKind::None`. `PROCESS_PORT_BACKED_BUILTIN_CAPABILITY_IDS` then removes
`builtin.shell` from the toolset entirely, so there is no interpreter to run `scripts/egfr.py` and no
host path it could live at.

That is deliberate: a tenant's script must not run as a host process on shared infrastructure.
`multi_tenant_skill_scripts_cannot_execute` pins it, and that test failing is the signal that the
posture changed.

**The failure mode this creates, and why the sandbox matters.** A model deprived of execution does
not degrade gracefully. Measured on a production-profile server, asked to apply a skill whose body
said *"execute it with `python3 scripts/egfr.py`"*, the agent read the script, worked out it had no
tool to run it, hand-expanded Taylor series for `ln`/`exp`, and then **POSTed the patient's
creatinine and age to `api.mathjs.org`** three times to do the arithmetic. Correct answers, via a
third-party service, from a tenant runtime. Until execution exists, the mitigation is to stop telling
the model to execute — see the first flip point below.

## Flip points, in order

### 1. `SkillActivationSelectorConfig::process_execution_available`

`crates/ironclaw_first_party_extension_ports/src/activation.rs`

When `false`, a skill body that mentions execution (`scripts/`, `python3`, `bash `, …) gets
`NO_PROCESS_EXECUTION_NOTE` appended: the instruction cannot be followed here, apply the documented
method directly, and do not call an external service to compute. That last clause exists because of
the `api.mathjs.org` incident above.

It is derived from the resolved policy in
`crates/ironclaw_reborn_composition/src/runtime.rs::filesystem_skill_context_source`, so **nothing
needs editing here**: the moment the resolver returns `TenantSandbox` instead of `None`, this flips to
`true` on its own and the note disappears.

### 2. The bundle needs a path the sandbox can reach

This is the real work, and it is not done. A skill's `scripts/*.py` exists only in the database. Even
with a process backend, `python3 scripts/egfr.py` has nothing to open. Two candidate designs:

- **Materialize the activated bundle** into a per-run directory the sandbox mounts, and inject that
  path into the skill's context so the model uses it. Closest to Claude Code, and it also fixes
  `references/*.md`, which progressive disclosure expects the agent to open on demand.
- **A capability that runs a skill script by name**, streaming the file out of the virtual filesystem
  into the sandbox. Nothing lands on a filesystem, but it only helps scripts, not other bundle assets.

Whichever is chosen, `E2E_PROFILE=multi-tenant` already asserts the current behaviour and the
assertion to change is named: `B5`, which today checks that **no shell was available** and in
local-dev checks whether the bundled script actually ran.

### 3. Turn the E2E's B5 into a hard assertion

`scripts/e2e-skill-self-creation.sh` reports B5 as a known gap rather than a failure. Once execution
works, run it with `E2E_REQUIRE_SCRIPT_EXEC=1`, which makes "the bundled script was never executed"
fail the suite. Do this in the same change as #2, so the gap cannot quietly reopen.

### 4. Re-check network egress

Independent of the sandbox, `builtin.http` reached an arbitrary public host from a hosted
multi-tenant runtime and carried clinical values with it. Whether that is intended under the resolved
`NetworkMode` needs a decision — allowlist, broker, or audit — and it should be settled before
execution lands, not after, because a sandboxed process is a second egress path.

## Test matrix

| Profile | Storage | Workspace | Shell | Skill scripts |
|---|---|---|---|---|
| `local-dev` | libSQL | host disk (server cwd) | yes (LocalHost) | reachable only if materialized |
| `single-tenant` (`hosted-single-tenant`) | Postgres | host disk (server cwd) | yes (LocalHost) | same as local-dev |
| `multi-tenant` (`production`) | Postgres | **database** | **no** (`None`) | blocked by policy |

```bash
# single-tenant: production storage, real process backend -- the shape most deployments run
E2E_PROFILE=single-tenant scripts/e2e-skill-self-creation.sh

# multi-tenant: the strict shape; asserts the shell is withheld
E2E_PROFILE=multi-tenant scripts/e2e-skill-self-creation.sh

# after the sandbox lands
E2E_PROFILE=multi-tenant E2E_REQUIRE_SCRIPT_EXEC=1 scripts/e2e-skill-self-creation.sh
```
