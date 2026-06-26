# Outbound attachments + generic project-filesystem download (Reborn)

**Status:** complete (path/link-based design) — generic FS read/download backend + WebUI download chips landed; tool+harvest deliberately not pursued
**Date:** 2026-06-15

## Implementation status (2026-06-15)

**Done (built, contract-locked, unit-tested, clippy `-D warnings` clean):** the generic
project-filesystem read/download backend — §3.4 in full, plus the `mime_for_extension`
registry helper.

- Port `ProjectFilesystemReader` + DTOs + `ProjectFsError`
  (`crates/ironclaw_product_workflow/src/reborn_services/project_fs.rs`).
- Facade field/builder + `list_project_dir` / `stat_project_path` / `read_project_file`
  on `RebornServicesApi` (default-unavailable) with the shared ownership probe
  (`resolve_thread_access_for_caller`) + `ProjectFsError → RebornServicesError` mapper.
- Composition impl `ProjectScopedFilesystemReader` over the read-only workspace
  `ScopedFilesystem`, wired in `webui.rs`
  (`crates/ironclaw_reborn_composition/src/project_filesystem_reader.rs`).
- HTTP: three GET routes (`/threads/{id}/files`, `/files/stat`, `/files/content`),
  descriptors, handlers (JSON + streamed download with
  `Content-Disposition: attachment` + `nosniff`), router mount, descriptors-contract
  test updated. `mime_for_extension` added to `ironclaw_common`.
- **Already works on history reload:** the timeline endpoint returns
  `ThreadMessageRecord.attachments`, so any message carrying refs (inbound today)
  renders downloadable once the frontend link lands.

**Design decision (2026-06-15):** went **path/link-based**, not the tool+harvest
approach (§3.1–3.3 dropped). The agent references files it creates by their `/workspace`
path; the WebUI renders workspace-path references as downloadable file chips over the
generic endpoint. Rationale: the download is a presentation concern over the filesystem
(the path *is* the file's identity), it reuses the already-built generic endpoint, and
it avoids a deep/risky change to the core agent loop (`ironclaw_loop_support` finalize +
`AssistantReply`). A structured `attach_file` tool + harvest can be added later *iff* a
non-text surface needs native structured attachments (e.g. Slack file upload) — at which
point a concrete consumer justifies the cost. The `attach_file` tool work explored this
session was reverted.

**Done (task 5 — frontend):**
- WebUI: extracts `/workspace/...` path references from an assistant reply and renders a
  **file-chip row** below the message — icon, filename, size (via the `/files/stat`
  endpoint), download action. Chips render on assistant messages only.
- Download via authenticated **blob fetch** against `/files/content?path=…` (the route
  is bearer-only — a plain `<a download href>` can't carry the bearer; fetch with the
  SPA's bearer → object URL → click → revoke), with `threadId` threaded to the chip.
- Agent prompt: one line telling the agent to reference files it creates by their
  `/workspace/...` path so they surface as downloads.

---
**Goal:** Let the Reborn agent produce files on the project filesystem (CSV, reports,
…) and attach them to its chat reply, downloadable by the user. Build the download
side as a **generic project-filesystem read API** (list + content) that doubles as the
substrate for future filesystem navigation in the web UI.

---

## 1. Why this is mostly wiring

The expensive primitives already exist, built for **inbound** user uploads:

- **One filesystem authority, shared by agent and backend.** The agent's
  `file_write`/`file_read`/`list_dir` tools and inbound-attachment landing both go
  through the same `ScopedFilesystem<F>` at the `/workspace` alias
  (`crates/ironclaw_filesystem/src/scoped.rs`,
  `crates/ironclaw_reborn_composition/src/local_dev_mounts.rs`:
  `/workspace` → `/projects/workspace` → host). A handler holding that
  `ScopedFilesystem` can `read_bytes`/`list_dir`/`stat` anything the agent wrote.
- **Durable attachment slot already on transcript messages.**
  `ThreadMessageRecord.attachments: Vec<AttachmentRef>`
  (`crates/ironclaw_threads/src/contract.rs:206`) and
  `MessageContent::with_attachments(...)` (same file, ~`:68`) exist and are wired
  through `finalize_assistant_message`. Inbound user messages already populate them.
- **`AttachmentRef` is byte-free and path-keyed.** `storage_key` is a rendered
  scoped path (`/workspace/...`), not raw bytes (`crates/ironclaw_common/src/attachment.rs:86`).
  So an attachment *is* a filesystem path — which is exactly the input the generic
  download endpoint takes. One endpoint serves both.
- **Frontend already renders attachment chips** (`message-bubble.js:165`), just
  without a download link and with no backend populating the field.

So the only genuinely new surface is: (1) a tool for the agent to *mark* a written
file as an attachment, (2) harvesting those marks onto the finalized assistant
message, (3) projecting `attachments` into the UI timeline, and (4) the generic
read/download endpoint + a download affordance in the chip.

---

## 2. Architecture at a glance

```
LLM ── attach_file({path:"/workspace/report.csv"}) ──► first-party coding tool
        validates path, stat, mime/kind from registry → AttachmentRef in tool output JSON
                                                              │
turn loop (ironclaw_turns) accumulates attach_file refs as capability results stream by
                                                              │
finalize_assistant_message(MessageContent::with_attachments(text, refs))
                                                              │
ThreadMessageRecord.attachments  (durable; same slot inbound uses)
                                                              │
projection ► FinalReplyView.attachments / ProductProjectionItem ► SSE ► browser
                                                              │
chip renders <a download href=.../files/content?path={storage_key}>
                                                              │
GET /api/webchat/v2/threads/{id}/files/content?path=…  ──► generic FS read API
        ProjectFilesystemReader (facade) → ScopedFilesystem.read_bytes(scope, path)
```

The download endpoint never knows about "attachments" — it reads a path under the
caller's project workspace. Attachments are just one producer of those paths;
filesystem navigation (a future file browser) is another.

---

## 3. Layer-by-layer implementation

Ordered by dependency. Each layer compiles independently
(`cargo build -p <crate>` / `node --check`). Follows the `reborn-feature` ladder.

> **Historical (not implemented):** §3.1–§3.3 below describe the original
> tool+harvest approach that was **dropped** in favor of the path/link-based
> design (see the 2026-06-15 design decision above). They are retained only as
> the rejected alternative; **only §3.4 (generic FS read API) and §3.5
> (frontend) actually landed.** Do not build against §3.1–§3.3.

### 3.1 `attach_file` first-party tool _(dropped — not implemented)_

**Crate:** `ironclaw_first_party_extensions` (the coding tools live here, dispatched
by `ironclaw_host_runtime`). Mirror `write_file` in
`crates/ironclaw_first_party_extensions/src/coding/file.rs:95`.

New handler `attach_file(request: &CodingCapabilityRequest) -> Result<CodingCapabilityOutput, CodingCapabilityError>`:

1. `let path_str = required_str(request.input, "path")?;`
2. `let resolved = resolve_required_path(request, "path", FilesystemOperation::ReadFile)?;`
   (read permission is sufficient — we only reference, never mutate.)
3. `let stat = stat_optional(request, &resolved.virtual_path).await?` →
   error if absent; error if `stat.sensitive` (never attach secrets/`/host`-sensitive files);
   error if `stat.file_type != FileType::File`; enforce a max size cap (reuse the
   inbound `DEFAULT_MAX_ATTACHMENT_BYTES` = 25 MiB).
4. Derive `kind`/`mime_type` from the filename via the existing registry
   (`ironclaw_common::attachment_format`: `kind_for_mime` / `canonical_extension`,
   plus a small extension→mime helper). Reject active-content types the same way
   inbound does (e.g. SVG) since these become downloadable.
5. Build the ref:
   ```rust
   AttachmentRef {
       id: deterministic_id(turn_run_id, resolved.scoped_path.as_str()), // stable, unique-within-message
       kind,
       mime_type,
       filename: file_name_of(resolved.scoped_path),
       size_bytes: Some(stat.len),
       storage_key: Some(resolved.scoped_path.as_str().to_string()),
       extracted_text: None,
   }
   ```
6. Return output JSON carrying the ref so the loop can harvest it:
   ```rust
   let output = json!({ "path": resolved.scoped_path.as_str(), "attached": true,
                        "attachment_ref": attachment_ref });
   Ok(CodingCapabilityOutput::new(output))
   ```

**Registration:** add an entry to `CODING_CAPABILITIES` (id `attach_file`, JSON
schema `{ path: string }`, description) so it registers in
`builtin_first_party_base_registry()`
(`crates/ironclaw_host_runtime/src/first_party_tools/mod.rs:236`) and is offered on
the capability surface. Confirm it lands in the model's tool list for the coding
profile.

> Naming: keep `storage_key` semantically equal to the scoped path
> (`/workspace/...`) — the download endpoint resolves it directly. Do **not** invent
> a second opaque id scheme; the scoped path already can't escape the mount.

### 3.2 Harvest refs onto the finalized assistant message _(dropped — not implemented)_

**Crate:** `ironclaw_turns` (loop host) + `ironclaw_reborn`/composition (the
`LoopTranscriptPort` impl).

The loop already streams capability results past
`LoopTranscriptPort::append_capability_result_ref`
(`crates/ironclaw_turns/src/run_profile/host.rs:1858`). `FinalizeAssistantMessage`
(same file, ~`:1843`) currently carries only `reply: AssistantReply` and has **no
attachment field** — this is the one schema gap on the write path.

Plan:
1. Add `attachments: Vec<AttachmentRef>` to `AssistantReply` (or to
   `FinalizeAssistantMessage`).
2. In the loop host, keep a per-turn `Vec<AttachmentRef>`. As each capability result
   is processed, if `capability_id == "attach_file"`, parse `attachment_ref` from the
   output and push it (dedupe by `storage_key`; last-write-wins on same path).
3. At finalize, pass the accumulated refs into `FinalizeAssistantMessage`.
4. The composition impl of `LoopTranscriptPort::finalize_assistant_message` maps them
   into `MessageContent::with_attachments(text, refs)` and calls
   `SessionThreadService::finalize_assistant_message`
   (`crates/ironclaw_threads/src/service.rs:78`). Existing
   `validate_attachment_refs` at the thread-service boundary enforces id-uniqueness.

This reuses the durable `ThreadMessageRecord.attachments` slot end-to-end — no new
persistence.

> Alternative considered: a writable sink on `InvocationServices`
> (`crates/ironclaw_host_runtime/src/invocation_services.rs:34`). Rejected — it only
> exposes `audit_sink`, which is for audit events, not structured turn state, and
> adding a turn-scoped collector there widens a shared interface for one tool. The
> loop-accumulation path keeps the change local to where capability results already
> flow.

### 3.3 Project `attachments` into the timeline + SSE _(dropped — not implemented)_

**Crate:** `ironclaw_product_adapters` + `ironclaw_webui_v2`.

1. Add `attachments: Vec<AttachmentRefView>` to `FinalReplyView`
   (`crates/ironclaw_product_adapters/src/outbound.rs:187`) and to the assistant
   `ProductProjectionItem` text variant (so replayed history shows them too, not just
   the live `final_reply` frame).
2. Define `AttachmentRefView { id, kind, mime_type, filename, size_bytes, storage_key }`
   — the byte-free projection the browser needs to build a download URL. Map from
   `ThreadMessageRecord.attachments` wherever the projection is built from transcript
   records.
3. Reflect the field in the webui_v2 SSE schema (`crates/ironclaw_webui_v2/src/schema.rs`,
   `WebChatV2Event` / `final_reply` + projection frames).

> Bonus: this also surfaces **inbound** attachments in the UI (user messages already
> store them but the projection drops them today). Verify the inbound chip renders
> after this change.

> Keep the wire byte-free: ship only `AttachmentRefView`; bytes flow exclusively
> through the download endpoint (§3.4). Never inline file bytes into SSE frames.

### 3.4 Generic project-filesystem read API (the reusable core)

This is the piece the user explicitly wants generic — used by attachments now and a
file browser later.

**Port** — `ironclaw_product_workflow`, new
`reborn_services/project_fs.rs`:

```rust
#[async_trait]
pub trait ProjectFilesystemReader: Send + Sync {
    async fn list_dir(
        &self, caller: &WebUiAuthenticatedCaller, thread_id: &ThreadId, path: &str,
    ) -> Result<Vec<ProjectFsEntry>, ProjectFsError>;

    async fn read_file(
        &self, caller: &WebUiAuthenticatedCaller, thread_id: &ThreadId, path: &str,
    ) -> Result<ProjectFsFile, ProjectFsError>; // { bytes, mime_type, filename, size }

    async fn stat(
        &self, caller: &WebUiAuthenticatedCaller, thread_id: &ThreadId, path: &str,
    ) -> Result<ProjectFsStat, ProjectFsError>;
}
```
DTOs: `ProjectFsEntry { name, path, kind: File|Dir|Symlink|Other, size: Option<u64> }`
(mirrors `DirEntry`), `ProjectFsStat` (mirrors `FileStat` minus host paths),
`ProjectFsFile`. Re-export in `reborn_services.rs` + `lib.rs`.

**Facade** — `RebornServicesApi` (`reborn_services.rs:887`): add
`Option<Arc<dyn ProjectFilesystemReader>>` field + `with_project_filesystem_reader(..)`
builder + the three `RebornServicesApi` methods with **default "unavailable" bodies**
(`RebornServicesError::service_unavailable(false)`) so existing fakes/tests compile
untouched. Add an error mapper `ProjectFsError → RebornServicesError`
(`not_found`/`forbidden`/`service_unavailable`).

**Impl** — `ironclaw_reborn_composition`, new
`project_filesystem_reader.rs` (gate behind the appropriate feature, mirror
`attachment_landing.rs`):
1. Hold `Arc<ScopedFilesystem<LocalDevRootFilesystem>>` (read-only view) — wired the
   same way `ProjectScopedAttachmentLander` gets its handle in `factory.rs`
   (`workspace_filesystem`, `factory.rs:474`).
2. **Verify thread ownership** for `caller` (bind/resolve the thread via the session
   thread service) → obtain a `ThreadScope`. Identity comes from the trusted caller,
   never the request.
3. `let scope = thread_scope.to_resource_scope();` (fresh `invocation_id` is created
   for us — `crates/ironclaw_threads/src/contract.rs:29`).
4. Normalize+validate the request `path`: must resolve under `/workspace`;
   `ScopedPath::new` rejects `..`/absolute escapes. Reject `stat.sensitive`.
5. `read_bytes` / `list_dir` / `stat` on the `ScopedFilesystem`
   (`crates/ironclaw_filesystem/src/scoped.rs:331/275/296`). Read+list permissions
   only — use the read-only mount view.

**HTTP** — `ironclaw_webui_v2`:
- Routes (`descriptors.rs`): two read-policy descriptors —
  - `GET /api/webchat/v2/threads/{thread_id}/files?path=…` → `Json<Vec<ProjectFsEntry>>`
    (directory listing; the navigation surface).
  - `GET /api/webchat/v2/threads/{thread_id}/files/content?path=…` → raw bytes.
  Add route constants + patterns + `*_descriptor()` (use `read_policy`), append to
  `webui_v2_routes()`, **update `tests/webui_v2_descriptors_contract.rs`** (it locks
  the table).
- Handlers (`handlers.rs`): `Extension<WebUiAuthenticatedCaller>` + `Path(thread_id)`
  + `Query{ path }`, call `state.services()`. The content handler returns a streamed
  body:
  ```rust
  Response::builder()
      .header(CONTENT_TYPE, safe_content_type(&file.mime_type))
      .header(CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", sanitized))
      .header("X-Content-Type-Options", "nosniff")
      .body(Body::from(file.bytes))
  ```
- Mount in `router.rs`.

**Security (this is a file-read endpoint — get it right):**
- `Content-Disposition: attachment` **always** (never inline) + `nosniff` → a
  generated `.html`/`.svg` can't execute in the app origin.
- Confine to `/workspace`; rely on `ScopedPath` + mount resolver. Defense in depth:
  the production resolver (`invocation_mount_view`) rewrites `/workspace` to
  `/tenants/{tenant}/users/{user}/workspace`, so bytes resolve under the *caller's*
  tenant/user regardless of the path — a guessed `thread_id` still can't cross
  tenants. Still verify thread ownership so cross-user reads in the same tenant are
  denied.
- Deny `stat.sensitive` files (secrets) and symlinks that escape the mount
  (`LocalFilesystem` already canonicalizes + containment-checks).
- Enforce a max download size; consider HTTP range support later.
- Apply the standard read rate limit from the descriptor policy.

### 3.5 Frontend

**Crate:** `ironclaw_webui_v2_static` (no build step; `node --check`).
1. Extend the message mapping (the SSE/`useChat` event handler) to copy projected
   `attachments` onto `message.attachments`, computing a download URL per ref:
   `` `/api/webchat/v2/threads/${threadId}/files/content?path=${encodeURIComponent(att.storage_key)}` ``.
2. `message-bubble.js:165` — add a download link to the existing chip:
   ```js
   <a href=${att.downloadUrl} download=${att.filename || "file"}
      className="ml-auto inline-flex items-center gap-1 …">
     <${Icon} name="download" className="h-3 w-3" /> Download
   </a>
   ```
3. (Later, out of scope) a file-browser panel consuming the `…/files?path=` listing
   endpoint — already enabled by §3.4.

---

## 4. Build / verify per crate

```bash
cargo build -p ironclaw_first_party_extensions
cargo build -p ironclaw_host_runtime
cargo build -p ironclaw_turns
cargo build -p ironclaw_product_workflow --all-features
cargo build -p ironclaw_product_adapters
cargo build -p ironclaw_webui_v2 --features webui-v2-beta
cargo build -p ironclaw_reborn_composition --features "webui-v2-beta libsql"
cargo build -p ironclaw_reborn_cli            # full serve graph
cargo test  -p ironclaw_webui_v2              # descriptors contract test
node --check crates/ironclaw_webui_v2_static/static/js/pages/chat/components/message-bubble.js
```

Test through the caller, not just the helper (per CLAUDE.md): cover the
`ProjectFilesystemReader` impl and the webui handler (ownership denial, path-escape
denial, sensitive-file denial, dir listing, byte download), and a turn-level test
that an `attach_file` call results in `ThreadMessageRecord.attachments` populated and
projected.

---

## 5. Scope forks (price them before building)

| Fork | Cheap | Expensive |
|------|-------|-----------|
| Download endpoint | file `content` + `stat` only | + `list_dir` navigation (small extra; the user asked for it — include) |
| attach_file source | path must already exist (agent wrote it) | tool also accepts inline content and writes the file itself |
| Extraction | none for outbound | run text extractors to populate `extracted_text` for previews |
| History | live `final_reply` frame only | also project onto replayed timeline items (recommended; cheap once mapping exists) |

Recommended slice: **`list_dir` + `content` + `stat`**, attach existing paths only,
no extraction, project onto both live and replayed items. This delivers the user's
ask (generate file → attach → download) and the generic navigation substrate in one
pass.

---

## 6. Files touched (summary)

| Layer | File(s) |
|-------|---------|
| Tool | `crates/ironclaw_first_party_extensions/src/coding/{file.rs,mod.rs}`, `crates/ironclaw_host_runtime/src/first_party_tools/mod.rs` |
| Harvest | `crates/ironclaw_turns/src/run_profile/host.rs`, the `LoopTranscriptPort` impl in `ironclaw_reborn`/composition |
| Persist | reuses `crates/ironclaw_threads/src/contract.rs` (`attachments`, `MessageContent::with_attachments`) — no change |
| Projection | `crates/ironclaw_product_adapters/src/outbound.rs`, `crates/ironclaw_webui_v2/src/schema.rs` |
| FS read port | `crates/ironclaw_product_workflow/src/reborn_services/project_fs.rs` (+ `reborn_services.rs`, `lib.rs`) |
| FS read impl | `crates/ironclaw_reborn_composition/src/project_filesystem_reader.rs` (+ `lib.rs`, `factory.rs`/`webui.rs` wiring) |
| HTTP | `crates/ironclaw_webui_v2/src/{descriptors.rs,handlers.rs,router.rs}`, `tests/webui_v2_descriptors_contract.rs` |
| Frontend | `crates/ironclaw_webui_v2_static/static/js/pages/chat/{components/message-bubble.js, hooks/*}` |
```
