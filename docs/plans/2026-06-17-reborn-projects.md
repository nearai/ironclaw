# Reborn Projects — first-class Project entity in the Reborn stack

**Status:** PRs 1–4, 6 landed + reviewed (local-dev). **PR5 (thread binding) reverted** after review — see below. Follow-ups: per-request **active-project plumbing** (the real PR5: browser → auth middleware authorizes an active project → `caller.project_id` for *all* thread ops, so create/submit/timeline/list stay consistent); production-runtime project repo wiring; tenant-scoped groups (#3796); agent-facing `project` tool; per-project missions/threads frontend endpoints.

### Post-review note (PR5 reverted)

The first cut bound a browser-supplied `project_id` onto the new thread's scope at
`create_thread`. Review found this creates **orphaned threads**: thread scope keys
include `project_id`, but `submit_turn`/timeline/stream/cancel/gate/delete derive
their scope from the caller's *static* installation `project_id`, so a thread bound
to a non-default project is unreachable (every follow-up op 404s). The binding was
also inert (no caller sends `project_id` yet), so it was a latent foot-gun. Reverted
the field, helper, tests, and call site. Correct thread binding requires the
active-project-per-request plumbing above (a cohesive, security-sensitive change
that authorizes the active project once in middleware and applies it to every thread
operation), tracked as the follow-up. #2809's root cause — no project entity/creation
path — remains closed by PRs 1–4.
**Date:** 2026-06-17
**Owner:** @ilblackdragon
**Related issues:** #2809 (create-project misroutes to mission), #2369 (Projects as living spaces UX), #3796 (tenant-scoped groups + project ACLs), #3697 (project live turn milestones)

> Scope note: `crates/ironclaw_engine` (engine v2) has its own legacy `Project`
> type. This plan is about the **Reborn stack** (`ironclaw_product` →
> `ironclaw_reborn_composition` → `ironclaw_webui_v2` → `ironclaw_webui_v2_static`),
> which has **no** first-class Project entity today — only `project_id` as a
> scope identifier on `ThreadScope`, `ProductAgentBoundCaller`, and
> `TriggerRecord`.

## Goal

Make Projects a first-class, persisted, access-controlled entity in the Reborn
stack, bind threads and automations to a project, and light up the existing
(hidden, stubbed) Projects page in WebChat v2.

## Locked scope decisions

1. **CRUD + thread binding.** Project lifecycle (create/list/get/update/delete),
   new threads + automations bind to the active project, and the thread /
   automation lists scope by project.
2. **Identity-only record with extensible `metadata`.** `name`, `description`,
   `icon`, `color`, plus a `metadata: serde_json::Value` bag so goals, GitHub
   links, etc. attach later with **no schema migration**.
3. **ACLs included now.** Owner + per-user role grants (Owner/Editor/Viewer),
   live-checked on every project-scoped operation, designed so tenant-scoped
   groups (#3796) drop in as an additional grant source without touching call
   sites.

## What already exists (do not rebuild)

- `ThreadScope.project_id: Option<ProjectId>` — `crates/ironclaw_threads/src/contract.rs:11`.
- `WebUiAuthenticatedCaller.project_id` / `ProductAgentBoundCaller` carry the
  project scope through every turn.
- `TriggerRecord.project_id` + `list_scoped_triggers(...)` already filter
  automations by project.
- `ProjectId` / `UserId` / `TenantId` newtypes — `ironclaw_host_api::ids` (`string_id!`).
- Frontend shell: `pages/projects/projects-page.js`, stub `projects-api.js`,
  nav entry `{id:"projects", hidden:true}` in `app/routes.js`, `folder` icon.
- Reference patterns to mirror: `ironclaw_triggers` (entity + dual-backend repo),
  `reborn_services/project_fs.rs` (port/DTO/error), `local_trigger_access.rs`
  (live access store), the automations webui_v2 vertical.

## Design

### New crate `ironclaw_projects` (mirrors `ironclaw_triggers`)

Domain record:

```rust
pub struct ProjectRecord {
    pub project_id: ProjectId,
    pub tenant_id: TenantId,
    pub owner_user_id: UserId,
    pub name: String,
    pub description: String,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub metadata: serde_json::Value, // extensible: goals, github, ... (object or null)
    pub state: ProjectState,         // Active | Archived
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}
```

Access model:

```rust
pub enum ProjectRole { Owner, Editor, Viewer }       // Viewer < Editor < Owner
pub enum ProjectMemberStatus { Active, Revoked }
pub struct ProjectMemberRecord {
    pub tenant_id, project_id, user_id, granted_by: ...,
    pub role: ProjectRole,
    pub status: ProjectMemberStatus,
    pub created_at, updated_at: Timestamp,
}
```

`ProjectRepository` trait (single `FilesystemProjectRepository` over the Reborn
`ScopedFilesystem`; backend selection is the host's `RootFilesystem` concern, so
no SQL in this crate):

- `create_project`, `get_project`, `update_project`, `delete_project`
- `list_projects_for_user(tenant, user, limit)` — owner OR active grant
- `list_members`, `upsert_member`, `remove_member`
- `resolve_access(tenant, project, user) -> Option<ProjectRole>` — owner
  short-circuit + active grants; `None` = no access. **Live, no caching.**

### Authorization model

`project_id` may arrive from the request as a **scope selector**, but identity
(`tenant_id`, `user_id`) comes only from the authenticated caller — never the
body. Every facade method calls `resolve_access` first; `None` → denied/404.
Groups (#3796) become an additional source inside `resolve_access`; call sites
unchanged.

## Implementation — layer by layer (dependency order)

1. **`crates/ironclaw_projects/`** (this PR) — entity + repo trait + error +
   `FilesystemProjectRepository` over `ScopedFilesystem` + contract tests.
   Register in workspace.
2. **Port** — `ironclaw_product/src/reborn_services/projects.rs`:
   `ProjectService` trait + sanitized DTOs + `ProjectServiceError`. Re-export.
3. **Facade** — `RebornServicesApi`: `Option<Arc<dyn ProjectService>>` field +
   `with_project_service` builder + methods with default "unavailable" bodies +
   error mapper. Update `fakes.rs`.
4. **Composition adapter** — `ironclaw_reborn_composition/src/project_service.rs`:
   `RebornProjectService` (repo + `resolve_access` gating). Construct repo in
   `factory.rs` next to `trigger_repository`; thread via `RebornRuntimeInput`;
   attach in the runtime-backed product surface.
5. **HTTP** — `ironclaw_webui_v2`: route consts + patterns + descriptors +
   `webui_v2_routes()` + thin handlers + router mounting + contract-test rows.
6. **Thread + automation binding** — `create_thread`/`submit_turn` stamp the
   authorized `project_id` onto `ThreadScope`; `list_threads`/`list_automations`
   scope by active project. Closes #2809.
7. **Frontend** — real `projects-api.js`, `useProjects` hook, presenters,
   member-management UI; unhide nav; confirm `nav.projects` i18n.

## Sequencing (one PR per layer)

PR1 crate → PR2 port+facade+fakes → PR3 composition+wiring → PR4 webui_v2 +
contract test → PR5 thread/automation binding (closes #2809) → PR6 frontend.
Per-crate gates as we go: `cargo build/clippy -p <crate>`, `node --check` for JS.

## Open items

- **Groups depth (#3796):** ship per-user grants now; full tenant-scoped groups
  (groups table + group→project grants + management UI) is a follow-on PR.
- **Default "General" project:** bind existing project-less threads to an
  implicit per-user default so nothing orphans (decide during PR5).
- **Agent-facing `project` tool:** optional, fixes #2809 at the intent layer too
  (decide during PR5).

## Persistence notes (ScopedFilesystem substrate)

**Decision (revised):** the repository persists over the Reborn
`ScopedFilesystem` substrate, **not** raw SQL handles. One backend-agnostic
`FilesystemProjectRepository<F: RootFilesystem>` replaces the original
in-memory/Postgres/libSQL trio; backend selection (Postgres / libSQL / JSONL /
in-memory) is the host's `RootFilesystem` concern, so the crate has no SQL and no
`libsql`/`postgres` features. This matches the convention that durable Reborn
stores ride one substrate seam (event store, identity, conversations), and keeps
the ACL — authorization data the agent must never write — on a control-plane
mount rather than the agent `/workspace` VFS.

Layout (base64url segments), tenant isolation by per-call `ResourceScope` **and**
path segment, CAS for create/delete atomicity, `created_at` immutable:

```text
/tenant-shared/reborn-projects/<tenant>/records/<project_id>.json
/tenant-shared/reborn-projects/<tenant>/members/<project_id>/<user_id>.json
```

The contract test runs the full suite against an in-memory `RootFilesystem`;
backend correctness is `ironclaw_filesystem`'s concern.
