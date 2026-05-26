# Reborn Contract - First-Party Skills Extension

**Status:** Draft implementation contract
**Date:** 2026-05-20
**Depends on:** [`kernel-boundary.md`](kernel-boundary.md), [`filesystem.md`](filesystem.md), [`extensions.md`](extensions.md), [`capability-access.md`](capability-access.md), [`capabilities.md`](capabilities.md), [`turns-agent-loop.md`](turns-agent-loop.md)
**Tracking:** nearai/ironclaw#3473

---

## 1. Purpose

Reborn skills are portable `SKILL.md` bundles supplied by a first-party
in-process extension. The extension owns skill storage, catalog, lifecycle, and
source enumeration. Reborn core owns the authority boundary: trust and
visibility enforcement, capability leases, and model-context injection.

This contract intentionally keeps skills out of the kernel. The kernel exposes
narrow ports and enforces policy. The first-party skills extension is userland
code, even when it ships with IronClaw and runs in process.

Core invariant:

```text
skills can provide instructions and supporting files;
skills cannot grant authority.
```

---

## 2. Ownership split

### Reborn core owns

- the loop-facing `HostSkillContextSource` port;
- trust and visibility policy before model-context injection;
- capability authorization, leases, grants, approvals, obligations, and resource
  budgets;
- scoped filesystem mount authority;
- redaction and no-leak requirements for model-visible snippets;
- fail-closed behavior when required trust or visibility data is absent.

### First-party skills extension owns

- skill bundle storage and enumeration;
- `SKILL.md` catalog/index metadata;
- install, update, delete, audit, and promotion lifecycle;
- learned/agent-created skill writes;
- skill search/list/view UX and extension APIs;
- conversion from configured virtual roots into skill bundle descriptors.

The extension must not resolve secrets, invoke tools, perform network requests,
spawn processes, or access raw storage unless it receives a narrow capability or
mount for that action.

---

## 3. Canonical format

The canonical portable skill format is a directory bundle:

```text
<skill-name>/
  SKILL.md
  references/
  templates/
  scripts/
  assets/
```

`SKILL.md` is the authoritative instruction file. The extension and adapter
should reuse `ironclaw_skills` for parsing, manifest validation, skill-name
validation, frontmatter handling, and any compatible selection helpers.

Supporting files are data by default. `scripts/` files do not become executable
just because they live in a skill bundle. Executing a script requires a separate
kernel-mediated capability lease and sandbox/runtime decision.

URL installs preserve the same bundle boundary as filesystem installs. A direct
raw `SKILL.md` URL installs only that document, while ZIP archives and supported
GitHub repository/tree URLs install the selected skill directory including
supporting files such as `references/`, `templates/`, `scripts/`, and `assets/`.
Supporting file paths must remain relative to the skill directory and pass the
same scoped-path containment checks before write.

---

## 4. Contract surfaces

Skill implementations must keep three surfaces separate:

### Catalog/admin descriptor

Cold control-plane representation for users, admins, UI, and extension tools.
It may include safe metadata such as source kind, provenance, audit state,
promotion state, requested capabilities, blocked reason, content hash, and
install/update status.

The catalog/admin descriptor is not model context.

### Model selection descriptor

Small model-facing representation used only after host policy has marked a skill
visible for the current invocation. It may include a safe name, safe
description, and bounded activation hint. It must not include hidden/denied
skills or leak raw paths, backend keys, source internals, audit hashes, secret
handles, capability lease IDs, or unapproved prompt content.

### Runtime skill context

Approved `SKILL.md` instruction content transformed into loop snippets through
`HostSkillContextSource`. This is the only path by which full skill instructions
become model-visible runtime context.

Implementations must not reuse a catalog/admin descriptor as a model selection
descriptor, and must not reuse either descriptor as runtime skill context.

---

## 5. Descriptor schema stance

The final machine-readable descriptor schema is deferred to the provider/source
port implementation PR. This contract freezes the direction, not the concrete
Rust or serialized shape.

The bundle format remains portable `SKILL.md`. Reborn-specific declarations
should live in a namespaced metadata block rather than creating a competing file
format. Candidate fields include:

```yaml
name:
description:
activation_hints:
metadata:
  reborn:
    requested_capabilities:
    requested_filesystem_mounts:
    requested_egress:
    requested_credentials:
    runtime_requirements:
    audit_requirements:
```

Rules:

- `requested_credentials` are credential or capability requirements, not secret
  handles and not secret material.
- `requested_capabilities`, `requested_filesystem_mounts`,
  `requested_egress`, and `requested_credentials` are requests only.
- frontmatter cannot declare tenant, user, agent, project, trust promotion, or
  capability grants.
- unknown Reborn metadata must be ignored or rejected according to the provider
  schema PR; it must not silently grant authority.

---

## 6. Storage boundary

The first-party skills extension stores and reads skills through
`ironclaw_filesystem::ScopedFilesystem` or an equivalent scoped host API handle.
It must not use raw host paths, direct database handles, `src/workspace`, engine
`MemoryDoc`, or `ironclaw_memory` internals as its architectural boundary.

Initial virtual roots:

```text
/system/skills        read-only system/bundled skills
/skills               invocation-scoped user skills
```

Optional future root:

```text
/tenant-shared/skills tenant-shared skills, if product policy accepts shared
                      visibility and ownership semantics
```

Production mount expectations:

- `/system/skills` remains read-only.
- `/skills` resolves per invocation through the same scoped filesystem fabric as
  other consumer-store aliases, e.g. to
  `/tenants/<tenant_id>/users/<user_id>/skills/...`.
- system and user roots must not collide.
- tenant-shared skills are not loaded or merged unless host policy explicitly
  grants a `/tenant-shared/skills` read mount for that invocation.
- cross-tenant and cross-user reads/writes must be denied by mount/scope checks
  before backend dispatch.
- agent/project-specific storage layers are deferred until the policy model
  defines precedence and visibility. Implementations must not infer agent or
  project scope from path text in `SKILL.md` or model input.

Scope source:

- tenant, user, agent, project, and invocation identity come from the
  `ResourceScope` / execution context that created the scoped filesystem view;
- skill files and manifests must not supply or override tenant/user/agent/project
  identity;
- provider APIs must treat `/skills/...` as a scoped alias, not a durable global
  namespace;
- descriptor metadata may include the effective source scope label for audit and
  UI, but must not expose backend keys or raw virtual targets to model context.

The extension can maintain metadata beside bundles, but metadata is cold
control-plane data. It must not be leaked into model context unless explicitly
converted into a safe summary by the runtime adapter.

First-slice precedence:

```text
1. /system/skills        read-only global system skills
2. /skills               scoped user skills
```

No tenant-shared, agent-specific, project-specific, or automatic cross-scope
merge behavior is part of the first slice. Those require explicit contract
updates because they affect visibility, promotion, and trust inheritance.

---

## 7. Source and provider ports

Implementation PRs should introduce a narrow source/provider contract before
adding concrete storage. The source contract should describe bundles, not raw
storage handles.

Recommended DTOs:

```text
SkillBundleId
SkillFilePath
SkillBundleDescriptor
SkillSourceKind
SkillProvenance
SkillAuditState
SkillActivationHint
```

Rules:

- DTOs must not include raw host paths or backend-specific keys.
- Descriptor ordering must be deterministic.
- source-local file paths are relative bundle paths and must reject absolute
  paths, `..`, backslashes, NUL/control characters, URL-like schemes, and scoped
  aliases.
- provider APIs may expose `SKILL.md` and supporting files to authorized callers,
  but runtime model injection must use the `HostSkillContextSource` path.

---

## 8. Extension trust is not skill trust

Skill trust is an input to policy, not a permission grant. The extension may
report source trust, provenance, audit state, content hash, and user/admin
promotion state. Reborn core still computes the effective runtime decision.

The first-party skills extension may be shipped by IronClaw and run in process,
but individual skills remain independently classified. A skill does not become
trusted because a first-party extension enumerated it.

Minimum source categories:

```text
system          host-bundled or admin-provisioned read-only skills
user            user-authored skills under /skills
agent_generated generated/learned skills under /skills
installed       hub/tap/repo-installed skills
external        configured read-only external sources
```

Minimum runtime decisions:

```text
visible         model may receive approved skill context
hidden          model must not receive name, description, or prompt content
denied          skill is inapplicable or blocked; no model-visible leakage
```

Fail-closed cases:

- missing trust data;
- missing visibility data;
- stale audit state after content hash changes when audit is required;
- unreadable or malformed `SKILL.md` for a visible candidate;
- unsupported source kind in a production profile.

Best-effort cases:

- optional supporting file read failures for files not needed by selected
  runtime context;
- search/index refresh failures after durable skill writes, provided the write
  outcome is explicit and later reads remain correct.

---

## 9. Capability model

Skill manifests may declare desired capabilities or host API requirements, but
those declarations are requests only.

Runtime flow:

```text
skill declares desired capability/profile/egress/secret needs
  -> skills extension reports descriptors and metadata
  -> Reborn policy evaluates trust, visibility, grants, leases, and obligations
  -> approved skill context may become model-visible
  -> approved capabilities may be leased separately
```

Rules:

- model-visible skill context must not make a hidden or denied capability
  invokable;
- a skill's declared capability list must not bypass `CapabilityHost`;
- the skills extension must not call dispatcher/runtime lanes directly;
- secret handles must remain declarations or lease requests, never prompt
  content;
- installed/community/agent-generated skills must not become trusted merely
  because the extension is first-party.

Promotion affects skill visibility/trust for model context. Promotion does not
grant capability execution. Capability execution still requires
`CapabilityHost` authorization through explicit grants or leases.

V1 approvals are exact-invocation leases. "Approve this blocked tool call" is
not equivalent to "always allow this skill." Reusable approvals for a
skill/capability pair are future policy work and require a separate contract
slice.

---

## 10. Runtime context path

The runtime integration path is:

```text
SkillBundleSource
  -> skills extension descriptor/source policy
  -> Reborn trust and visibility gate
  -> HostSkillContextCandidate
  -> SkillContextService
  -> loop/model instruction snippets
```

`HostSkillContextCandidate` should carry only the data needed to build the
snapshot:

- `SKILL.md` content for visible candidates;
- effective trust;
- effective visibility;
- deterministic ordering key.

The final model snippets must exclude raw paths, source metadata, audit
internals, capability IDs, secret handles, content hashes, backend keys, and
registry internals.

---

## 11. Creation and installation flows

Expected creation paths:

- user imports or uploads a skill bundle;
- a user or agent provides an HTTPS raw `SKILL.md`, ZIP bundle, or supported
  GitHub repository/tree URL to the networked `builtin.skill_install_url`
  capability, fetched through host-mediated network egress before writing to
  scoped skill storage with installed-skill provenance metadata;
- an agent/LLM creates or patches a learned skill under scoped `/skills`;
- a hub, tap, or repository install normalizes into the same bundle layout;
- admin/system provisioning installs read-only `/system/skills`.

Lifecycle operations such as `add_skill`, `update_skill`, `delete_skill`,
`audit_skill`, and `promote_skill` are durable writes or policy changes. They
must be capability-gated and audited. They must not be ordinary helper methods
available to model-facing code without authorization.

---

## 12. Learned skill lifecycle

Agent-created skills are ordinary skill bundles under scoped skill storage, but
they start with a lower trust state.

Rules:

- agent-generated skills default to draft or low-trust;
- promotion to trusted requires explicit user/admin policy, not self-declared
  frontmatter;
- patch/update is preferred over blind overwrite;
- content hash changes invalidate audit, promotion, and runtime cache decisions;
- generated skills must not write into `/system/skills`;
- generated skills must not modify installed/system skills unless a future
  explicit admin capability allows it.

---

## 13. Edge-case defaults

Implementation PRs should preserve these defaults unless a later contract change
explicitly overrides them:

- duplicate names across `/system/skills` and `/skills` must not silently shadow
  each other. Providers should use source-qualified stable IDs. Unqualified
  activation may resolve only when policy leaves one unambiguous visible
  candidate; otherwise ask or fail closed.
- content hash changes invalidate audit, promotion, visibility, and runtime
  cache decisions.
- installed skill updates that expand requested capabilities, mounts, egress, or
  credentials require renewed policy evaluation and must not inherit approval
  for the expanded request.
- hidden and denied skills must not leak through model selection descriptors,
  runtime snippets, search/list/view responses exposed to model-facing code, or
  error messages.
- eligible skill ranking must be deterministic and bounded. When too many
  skills are eligible, the provider/policy layer must cap candidates before
  runtime context injection.

---

## 14. In-process extension guardrails

In-process is an optimization, not authority.

The first-party extension should receive explicit handles such as:

```text
read_system_skills
read_user_skills
write_user_skills
install_skill
audit_skill
```

It must not receive:

- root filesystem handles;
- raw host paths;
- direct database pools;
- secret material;
- network clients;
- dispatcher/tool hosts;
- process runners;
- broad memory repositories.

If the extension later moves out of process, the Reborn core contract should not
change. Only the provider implementation and handle transport should change.

---

## 15. Acceptance tests

Implementation tasks under this contract must include caller-level tests for
the relevant slice. Required coverage over the roadmap:

- `/system/skills` is read-only;
- `/skills` is tenant/user scoped and cannot collide cross-tenant or cross-user;
- skill manifests cannot override tenant/user/agent/project scope;
- tenant-shared skills are not visible unless the invocation has an explicit
  `/tenant-shared/skills` read mount;
- the extension cannot read/write outside configured virtual roots;
- source DTOs reject traversal, absolute paths, scoped aliases, and control
  characters;
- valid `SKILL.md` bundles parse through `ironclaw_skills`;
- catalog/admin, model selection, and runtime context surfaces do not reuse each
  other's DTOs or leak unsafe fields;
- malformed visible candidates fail closed with sanitized errors;
- hidden and denied skills do not leak names, descriptions, or prompt content;
- visible installed/untrusted skills expose only safe descriptions;
- visible trusted skills may expose approved prompt content;
- skill context never grants capability authority;
- skill promotion does not authorize capability execution without grants/leases;
- scripts/assets are not executable without a separate capability lease;
- agent-generated skills are not auto-trusted;
- content updates invalidate audit/trust/runtime cache state;
- duplicate names across roots do not silently shadow each other;
- expanded requested authority after update requires renewed policy evaluation;
- output ordering is deterministic for identical inputs;
- end-to-end runtime test proves a real `SKILL.md` bundle reaches the model via
  `HostSkillContextSource`.

---

## 16. Non-goals

- Do not make skills kernel state.
- Do not move parsing/catalog/lifecycle logic into `ironclaw_turns`.
- Do not couple Reborn skills to engine `MemoryDoc` or `src/workspace`.
- Do not make `ironclaw_memory` the skills architectural boundary.
- Do not expose raw filesystem paths, source metadata, capability IDs, secret
  handles, audit internals, hashes, or registry internals to the model.
- Do not let skill context invoke capabilities or grant tool authority.
- Do not give the in-process skills extension ambient root filesystem,
  database, secrets, network, or tool access.
