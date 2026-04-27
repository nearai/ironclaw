# Ubiquitous Language — IronClaw Reborn

Domain glossary covering the full Reborn surface: the host architecture, the host API contracts in `crates/ironclaw_host_api`, the system-service crates (`ironclaw_extensions`, `ironclaw_filesystem`, `ironclaw_resources`, `ironclaw_capabilities`, `ironclaw_authorization`, `ironclaw_approvals`, `ironclaw_run_state`, `ironclaw_processes`, `ironclaw_secrets`, `ironclaw_network`, `ironclaw_events`, `ironclaw_dispatcher`, `ironclaw_host_runtime`, `ironclaw_wasm`, `ironclaw_scripts`, `ironclaw_mcp`, `ironclaw_memory`), and the architecture FAQ/decision log.

---

## Architecture and ownership

| Term | Definition | Aliases to avoid |
| --- | --- | --- |
| **Reborn** | The in-progress reboot of IronClaw with an OS-like host, narrow contracts, and explicit system services. | rewrite, v2, next |
| **Host** | The privileged process that composes system services and brokers all authority; runs userland extensions. | core, server, app |
| **Userland** | The non-privileged side of the host that runs extensions; first-party agent/gateway/TUI live here as privileged extensions, not in the kernel. | userspace, plugin layer |
| **Kernel (concept)** | The architectural notion of the small composition layer that wires system services. | dispatcher (the dispatcher is *not* the kernel) |
| **`ironclaw_host_runtime` (crate)** | The concrete composition crate that fills the kernel role today; there is no live `ironclaw_kernel` crate. | host runtime (avoid in casual prose; ambiguous with “runtime lane”) |
| **System service** | A host crate behind a narrow contract (filesystem, resources, secrets, network, events, processes, etc.). | subsystem, layer |
| **Extension** | A packaged provider of capabilities; lives under `/system/extensions/{id}/` with a manifest. *Not* a live runtime instance. | plugin (overlapping but vendor-flavored), addon |
| **Process** | A live runtime instance of a declared capability tracked by `ironclaw_processes`. *Not* a host OS process and *not* an extension. | task, worker |
| **Thread** | A durable logical-work record; identity flows through `ThreadId`. *Not* a process and *not* a conversation. | conversation (use thread for the ID; conversation for product-level UX) |
| **Architecture law** | One of the eleven invariants in `2026-04-24-os-like-architecture-design.md` (e.g. *Kernel wires; it does not become a product runtime*). | rule, policy |

## Identity, scope, and execution context

| Term | Definition | Aliases to avoid |
| --- | --- | --- |
| **Principal** | Typed enum of authority subjects: `Tenant`, `User`, `Agent`, `Project`, `Mission`, `Thread`, `Extension`, `System`. | actor (used loosely), identity |
| **`Principal::System`** | The narrow, explicit system authority; **not** a wildcard grant target. | admin, root |
| **`TenantId` / `UserId` / `AgentId` / `ProjectId` / `MissionId` / `ThreadId`** | Validated string newtypes used everywhere as scope keys. | string id, uuid (these are not UUIDs) |
| **`InvocationId` / `ProcessId` / `CapabilityGrantId` / `ResourceReservationId` / `ApprovalRequestId` / `AuditEventId` / `CorrelationId`** | UUID-backed identifiers for one-shot or per-record entities. | run id, request id (when ambiguous) |
| **`CorrelationId`** | An ID that ties together all events/audit records for one logical user-visible operation across multiple invocations. | trace id |
| **`ResourceScope`** | The cascade `tenant -> user -> agent? -> project? -> mission? -> thread? -> invocation` used for all costed work, accounting, and audit. | scope (bare) |
| **`ExecutionContext`** | The authority envelope handed to every host-API call: identities, runtime, trust, grants, mounts, and resource scope. Single source of truth per invocation. | request, ctx |
| **Local default scope** | The canonical local single-user scope: tenant `default`, agent `default`, project `bootstrap`. | dev scope |
| **Scope cascade** | The fixed precedence order `tenant -> user -> project -> mission -> thread -> invocation` used by limits and accounting. | hierarchy |

## Authority: capabilities, grants, leases, decisions

| Term | Definition | Aliases to avoid |
| --- | --- | --- |
| **Capability** | A declared, callable action on the host (e.g. `github.search_issues`). The narrow word for *what extensions expose*. Not authority by itself. | tool, function |
| **`CapabilityId`** | The `provider.action` string identifier for a capability. | tool name |
| **`CapabilityDescriptor`** | The host-API record describing one capability: id, provider, runtime, trust ceiling, parameters schema, declared effects, default permission, optional resource profile. | tool spec |
| **`EffectKind`** | The typed enum of side-effect classes a capability declares (e.g. `ReadFile`, `WriteFile`, `Network`, `UseSecret`, `SpawnProcess`). | permission flag |
| **`PermissionMode`** | Per-capability default approval posture (always-allow, always-ask, allow-on-approval). | trust mode |
| **`CapabilityGrant`** | An authority binding from a `Principal` to a `CapabilityId` with `GrantConstraints` (effects, mounts, network, secrets, ceilings, expiry, max invocations). | permission, allowance |
| **`CapabilitySet`** | The collection of `CapabilityGrant`s carried by an `ExecutionContext`. | grant bag |
| **`GrantConstraints`** | The fence around a grant: allowed effects, scoped mount view, network policy, secret handles, optional resource ceiling, expiry, max invocations. | grant scope |
| **`CapabilityLease`** | A short-lived, one-shot authority handle issued by approval resolution; carries an `InvocationFingerprint`. | approval token |
| **`InvocationFingerprint`** | The hash over scope + capability + estimate + JSON input that ties a `CapabilityLease` to one specific replay. | request hash |
| **`Decision`** | The authorization verdict: `Allow { obligations }`, `Deny { reason }`, `RequireApproval { request }`. | result, verdict |
| **`DenyReason`** | Closed set: `MissingGrant`, `InvalidPath`, `PathOutsideMount`, `UnknownCapability`, `UnknownSecret`, `NetworkDenied`, `BudgetDenied`, `ApprovalDenied`, `PolicyDenied`, `ResourceLimitExceeded`, `InternalInvariantViolation`. | error code |
| **`Obligation`** | A post-decision task the host must perform: `AuditBefore`, `AuditAfter`, `RedactOutput`, `ReserveResources`, `UseScopedMounts`, `InjectSecretOnce`, `ApplyNetworkPolicy`, `EnforceOutputLimit`. | hook (overloaded), side-task |
| **`TrustClass`** | The execution trust tier: `Sandbox`, `UserTrusted`, `FirstParty`, `System`. Tags an `ExecutionContext`. | tier (used informally in sandbox tiers — keep distinct) |
| **Trust ceiling** | The maximum `TrustClass` a `CapabilityDescriptor` will run under; runtime trust must be `<= ceiling`. | trust limit |
| **Default-deny** | The architectural rule that registration is not authority; missing grant/lease → `Deny::MissingGrant`. | implicit deny |

## Paths, mounts, and the filesystem boundary

| Term | Definition | Aliases to avoid |
| --- | --- | --- |
| **`HostPath`** | The physical backend path; intentionally non-serializable; only backend code may hold one. | abs path, real path |
| **`VirtualPath`** | A canonical durable address rooted at a system root (`/engine`, `/system/extensions`, `/users`, `/projects`, `/memory`). | absolute path |
| **`ScopedPath`** | The view a runtime sees through a `MountView`, like `/workspace/README.md`; the only path type runtimes receive. | relative path |
| **Virtual root** | One of the canonical `VirtualPath` prefixes: `/engine`, `/system/extensions`, `/users`, `/projects`, `/memory`. | mountpoint root |
| **`MountAlias`** | The local name a runtime uses for one mount entry (e.g. `workspace`). | mount name |
| **`MountGrant`** | A single `(alias, virtual target, MountPermissions)` binding; one entry of a mount view. | mount, mountpoint |
| **`MountView`** | The full set of `MountGrant`s available to one `ExecutionContext`; resolves `ScopedPath` -> `VirtualPath`. | mount table |
| **`MountPermissions`** | Per-mount `read/write/delete/list/execute` flags; a child view must be a subset of its parent. | acl |
| **Containment invariant** | The rule that `runtime sees ScopedPath; host policy reasons over MountView; filesystem resolves to VirtualPath; backend alone touches HostPath`. | sandbox boundary |
| **`RootFilesystem`** | Trusted host-service interface that operates on canonical `VirtualPath` values. | root fs (avoid lowercase; collides with the `/` directory) |
| **Backend (filesystem)** | A pluggable storage implementation behind a virtual root (local, libSQL, Postgres). | driver |

## Runtime lanes and dispatch

| Term | Definition | Aliases to avoid |
| --- | --- | --- |
| **`RuntimeKind`** | Closed enum of runtime lanes: `Wasm`, `Mcp`, `Script`, `FirstParty`, `System`. | runtime, kind |
| **Runtime lane** | One concrete execution path matching a `RuntimeKind` (the WASM lane, the Script lane, etc.). | runtime backend (overloaded) |
| **`RuntimeAdapter`** | The narrow port a runtime crate (`ironclaw_wasm`, `ironclaw_scripts`, `ironclaw_mcp`) implements so the dispatcher can route to it. | runtime impl |
| **`CapabilityDispatcher` (trait)** | The host-API trait `RuntimeDispatcher` implements; the only neutral dispatch port that `ironclaw_capabilities` may depend on. | dispatch interface |
| **`RuntimeDispatcher` (struct)** | The composition-only routing layer in `ironclaw_dispatcher`; selects a `RuntimeAdapter` by `RuntimeKind`. | dispatcher (acceptable in context, but disambiguate from `ToolDispatcher` in legacy code) |
| **`CapabilityDispatchRequest`** | An already-authorized request: `CapabilityId`, `ResourceScope`, `ResourceEstimate`, JSON input. The dispatcher does not authorize. | dispatch input |
| **`CapabilityDispatchResult`** | The normalized lane result with `RuntimeKind` and JSON output; same shape across WASM/Script/MCP. | dispatch output |
| **`DispatchError` / `RuntimeDispatchErrorKind`** | The structured failure modes for routing/consistency vs lane-internal errors. | dispatch failure |
| **dispatch (verb)** | Request/response invocation; caller does not manage a `ProcessId`; result is returned synchronously. | call (overloaded) |
| **spawn (verb)** | Background/long-lived/streaming start; returns a `ProcessId`; caller observes via `ProcessHost`. | run (overloaded), launch |

## Capability invocation workflow

| Term | Definition | Aliases to avoid |
| --- | --- | --- |
| **`CapabilityHost`** | The caller-facing invocation service in `ironclaw_capabilities`; entry points `invoke_json`, `resume_json`, `spawn_json`. | invoker, façade |
| **`invoke_json`** | The synchronous workflow: validate scope → fingerprint → authorize → maybe approval → dispatch → mark completed. | call, exec |
| **`resume_json`** | The post-approval workflow: load record + matching lease, recompute fingerprint, claim lease, dispatch, consume lease. | resume, replay |
| **`spawn_json`** | The capability-backed process-start workflow; requires `EffectKind::SpawnProcess` plus declared effects. | start |
| **`CapabilityDispatchAuthorizer`** | The authorization port `ironclaw_authorization` implements (`authorize_dispatch`, `authorize_spawn`). | authorizer |
| **`GrantAuthorizer`** | Default grant-matching authorizer: matches `grant.capability == descriptor.id`, grantee matches principal, allowed effects cover declared effects. | grant matcher |
| **`LeaseBackedAuthorizer`** | Authorizer that combines grants with active non-fingerprinted leases; used before dispatch. | lease checker |
| **`CapabilityObligationHandler`** | The seam invoked when a `Decision::Allow` carries non-empty obligations; failure aborts before dispatch. | obligation runner |
| **`BuiltinObligationHandler`** | The default handler in `ironclaw_host_runtime`: metadata audit, network policy preflight, and direct `InjectSecretOnce` lease/consume. | host obligations |

## Approvals and run state

| Term | Definition | Aliases to avoid |
| --- | --- | --- |
| **`ApprovalRequest`** | The host-API request shape: id, correlation, requester, action, optional fingerprint, reason, optional reusable scope. | approval |
| **`ApprovalRecord`** | The persisted store row carrying `ApprovalStatus`. | approval row |
| **`ApprovalStatus`** | `Pending`, `Approved`, `Denied`, `Expired`. | approval state |
| **`ApprovalScope`** | Optional reusable scope attached to an approval to allow lease reuse across compatible invocations. | reusable scope |
| **`ApprovalRequestStore`** | The durable store API (tenant/user-scoped reads, scoped `approve`/`deny`). | approval db |
| **`ApprovalResolver`** | The `ironclaw_approvals` service that reads `Pending` records and produces fingerprinted `CapabilityLease`s on approve. | approver, decider |
| **`CapabilityLeaseStore`** | The durable store of issued leases keyed by tenant/user/invocation/capability/fingerprint. | lease db |
| **`RunRecord` / `RunStatus`** | The current lifecycle row of an invocation: `Running`, `BlockedApproval`, `BlockedAuth`, `Completed`, `Failed`. | run state row |
| **`RunStateStore`** | The durable run-state API; tenant/user scoped; partitioned by `ResourceScope`. | run db |
| **Control-plane vs runtime event distinction** | Approval resolution is *control-plane audit* (`AuditEnvelope { stage: ApprovalResolved }`), not a `RuntimeEvent`. | event (always qualify) |

## Processes (live runtime instances)

| Term | Definition | Aliases to avoid |
| --- | --- | --- |
| **`ProcessRecord`** | The tracked tuple `(ProcessId, parent?, InvocationId, ResourceScope, ExtensionId, CapabilityId, RuntimeKind, ProcessStatus, grants, mounts, estimate, reservation?, error_kind?)`. Always tenant/user-scoped. | process row |
| **`ProcessStatus`** | The lifecycle enum (started/running/exited/failed/killed). | state |
| **`ProcessExecutor`** | The narrow trait that actually runs a capability lane in the background. | runner |
| **`ProcessManager`** | The trait that owns the process *table*: create/get/list/transition/cancel. | task manager |
| **`ProcessHost`** | The host-facing facade for spawn/await/subscribe/kill workflows. | process api |
| **`ProcessServices`** | The composition struct bundling store + executor + cancellation registry. | process container |
| **`BackgroundProcessManager`** | The default `ProcessManager` used by `HostRuntimeServices` for detached execution. | bg manager |
| **`ResourceManagedProcessStore` / `EventingProcessStore`** | Decorator stores that own resource reservation and event emission around a base process store. | wrapper |
| **`ProcessCancellationToken` / `ProcessCancellationRegistry`** | Cooperative cancellation primitives shared between `ProcessManager` and `ProcessExecutor`. | cancel handle |
| **`DispatchProcessExecutor`** | The composition path that lets a capability-backed process run through the same `RuntimeDispatcher` without leaking borrowed state. | spawn executor |

## Extensions and packages

| Term | Definition | Aliases to avoid |
| --- | --- | --- |
| **`ExtensionId`** | The validated lowercase identifier for one extension package. | plugin id |
| **`ExtensionPackage`** | The on-disk extension at `/system/extensions/{id}/`: manifest plus assets (skills, scripts, wasm). | extension folder |
| **`ExtensionManifest`** | The validated `manifest.toml` declaring runtime kind, capabilities, and authority metadata. | manifest |
| **`CapabilityManifest`** | The per-capability section of an `ExtensionManifest`. | capability spec |
| **`ExtensionAssetPath`** | A validated relative path inside an extension package. | asset |
| **`ExtensionRuntime`** | The manifest-level runtime tag mapped onto `RuntimeKind`. | runtime decl |
| **`ExtensionRegistry`** | The in-memory registry of installed extensions and their declared capabilities; *knows what can run, never what is running*. | catalog |
| **`ExtensionDiscovery`** | The filesystem scan that loads `ExtensionPackage`s from `/system/extensions`. | loader |
| **`ExtensionLifecycleOperation`** | Typed lifecycle action: install/activate/deactivate/uninstall, used in audit. | lifecycle event |

## Resources, budget, and quotas

| Term | Definition | Aliases to avoid |
| --- | --- | --- |
| **`ResourceGovernor`** | The host service that owns reservation/reconcile/release for all costed work. | budget service |
| **`ResourceEstimate`** | Caller-provided pre-execution estimate (USD, tokens, wall-clock, output bytes, egress, processes, concurrency). | budget hint |
| **`ResourceUsage`** | Actual post-execution consumption returned by a runtime. | actuals |
| **`ResourceProfile`** | Per-capability default estimate plus optional hard ceiling. | budget profile |
| **`ResourceCeiling`** | Hard upper bounds on a single invocation; deny when exceeded. | hard limit |
| **`SandboxQuota`** | Runtime-enforced limits on sandbox execution (memory, fuel, time, output bytes). | sandbox limits |
| **`ResourceReservation`** | The held capacity created by `reserve(scope, estimate)`; must be `reconcile`d or `release`d. | budget hold |
| **`ReservationStatus`** | Lifecycle states (Reserved, Reconciled, Released, …). | reservation state |
| **`ResourceReceipt`** | The post-call record returned by lanes carrying actual usage. | usage record |
| **Sandbox tier** | One of *Tier 0/1/2/3*: host-mediated → stateless micro-worker → project-scoped warm runtime → strong per-job isolation. | sandbox level |

## Secrets and credentials

| Term | Definition | Aliases to avoid |
| --- | --- | --- |
| **`SecretMaterial`** | The raw secret value, wrapped in `secrecy::SecretString`; only revealed via `ExposeSecret`. | secret value, password |
| **`SecretHandle`** | The opaque host-API pointer to a secret; never the value. | secret name |
| **`SecretId`** | The UUID identity of one stored secret row. | |
| **`SecretMetadata`** | Redacted descriptor (kind, scope, created/last-used, expiry); never carries material. | secret info |
| **`SecretLease`** | A one-shot redemption right for one `SecretHandle`; status `Active` → `Consumed`/`Failed`. | access token |
| **`SecretLeaseStatus`** | The lease lifecycle enum. | lease state |
| **`SecretsCrypto`** | The HKDF-SHA256 + AES-256-GCM crypto port; redacts master keys/ciphertext/salts in `Debug`. | crypto |
| **`EncryptedSecretRecord` / `EncryptedSecretRepository` / `EncryptedSecretStore`** | The encrypted-row layer plus its repository contract and `SecretStore` impl. | secret table |
| **`FilesystemEncryptedSecretRepository`** | The durable repository implemented over `RootFilesystem`. | fs secrets |
| **`CredentialMapping`** | The runtime-facing mapping from logical credential location to one or more `SecretHandle`s. | credential, env mapping |
| **`CredentialAccount`** | A logical account record (e.g. one Gmail OAuth identity) referenced by capability-side credential lookups. | account |
| **`CredentialSlot`** | A named slot within an account (e.g. `oauth_refresh_token`). | env var, key |
| **`CredentialSecretRef`** | A typed reference from a credential slot to a stored secret. | secret pointer |
| **`CredentialLocation`** | The semantic location a credential is delivered to (env var, header, query param, etc.). | injection point |

## Network policy and egress

| Term | Definition | Aliases to avoid |
| --- | --- | --- |
| **`NetworkPolicy`** | The host-API description of allowed targets, egress limits, and private-IP posture; lives on `GrantConstraints`. | allowlist |
| **`NetworkTarget` / `NetworkTargetPattern`** | Concrete target (`scheme://host:port`) vs glob pattern matched against allowed targets. | URL, host pattern |
| **`NetworkScheme` / `NetworkMethod`** | Closed enums for `https/http/...` and `GET/POST/...`. | protocol |
| **`NetworkRequest` / `NetworkPermit`** | The scoped policy-evaluation input/output; permits are *metadata-only* (no I/O). | policy result |
| **`NetworkPolicyEnforcer`** | The trait that turns `NetworkRequest` into `NetworkPermit` or `NetworkPolicyError`. | policy engine |
| **`HardenedHttpEgressClient`** | The egress client that performs HTTP only after policy + DNS + private-IP rejection + redirect re-validation + timeouts + size limits. | http client |
| **`HttpEgressRequest` / `HttpEgressResponse` / `HttpEgressError`** | The narrow egress request/response/error shapes. | http req/res |

## Events and audit

| Term | Definition | Aliases to avoid |
| --- | --- | --- |
| **`RuntimeEvent`** | The append-only runtime/dispatch/process event record; carries scope, capability, runtime, optional process, output bytes, error kind. | engine event |
| **`RuntimeEventKind`** | Closed enum: `DispatchRequested`, `RuntimeSelected`, `DispatchSucceeded`, `DispatchFailed`, `ProcessStarted`, `ProcessCompleted`, `ProcessFailed`, `ProcessKilled`. *No approval kinds.* | event type |
| **`EventSink`** | The trait runtime/process subsystems use to record `RuntimeEvent`s. | event bus (overloaded) |
| **`AuditEnvelope`** | The control-plane audit record: stage, scope, action summary, decision summary, optional result; *no raw payloads, paths, secrets, fingerprints, or lease contents*. | audit log |
| **`AuditStage`** | Enum naming the stage of the lifecycle being audited (e.g. `ApprovalResolved`, `ActionDenied`). | log stage |
| **`AuditSink`** | The trait the host uses to write `AuditEnvelope`s. | audit writer |
| **`ActionSummary` / `DecisionSummary` / `ActionResultSummary`** | The redacted summaries embedded in an `AuditEnvelope`. | summary |
| **Realtime event vs audit** | Architecture law: realtime events are not the audit log. The bus is for live updates; durable history is audit. | log (always qualify) |

## Memory and workspace (subsystem)

| Term | Definition | Aliases to avoid |
| --- | --- | --- |
| **Memory scope** | `tenant + user + agent? + project?`; the partition key for memory documents. | namespace |
| **`MemoryDocumentScope`** | The newtype carrying memory scope; `_none` is reserved for absent optional scopes. | memory key |
| **`MemoryDocumentPath`** | The relative path of one document inside a memory scope. | filename |
| **`MemoryBackend`** | Trait for memory plugins; today document-shaped, with a planned split (see flagged ambiguity). | memory adapter |
| **`MemoryBackendCapabilities`** | A *backend support declaration* (file ops/FTS/vector/embeddings); **not** an extension capability and **not** authority. Slated for rename to `MemoryBackendSupport`. | backend permissions |
| **`MemoryBackendFilesystemAdapter`** | Projects a `MemoryBackend` as virtual files under `/memory/...`. | memory mount |
| **`MemoryContext`** | Host-resolved per-call context handed to a backend; never widened by the backend. | request context |
| **`RepositoryMemoryBackend`** | Default backend over `MemoryDocumentRepository` (Postgres or libSQL). | db backend |
| **`MemoryDocumentService` / `MemorySearchService` / `MemoryVersionService` / `MemoryLayerService` / `MemoryPromptContextService` / `MemorySeedService` / `MemoryProfileService`** | The seven focused services that replace the monolithic production `Workspace`. | workspace service |
| **Primary / secondary / layer scope** | Write target / read-only merged scope / namespaced read-write surface (private/shared/team). | scope (bare) |
| **Identity files** | Primary-scope-only persona docs: `AGENTS.md`, `SOUL.md`, `USER.md`, `IDENTITY.md`. | persona docs |

## Composition (`ironclaw_host_runtime`)

| Term | Definition | Aliases to avoid |
| --- | --- | --- |
| **`HostRuntimeServices`** | The composition root that wires registry + filesystem + governor + authorizers + stores + runtimes + sinks + obligation handler. | host services |
| **Composition-only crate** | A crate that owns no new authority semantics or lifecycle state; only wiring. `ironclaw_host_runtime` and `ironclaw_dispatcher` are the canonical examples. | glue |
| **Borrowed vs Arc dispatcher** | `RuntimeDispatcher::new(&...)` for request-scoped composition vs `from_arcs(...)` for detached background execution. | dispatcher form |

---

## Relationships

- An **`ExecutionContext`** carries one **`ResourceScope`**, one **`MountView`**, one **`CapabilitySet`**, one **`RuntimeKind`**, one **`TrustClass`**, and exactly one **`InvocationId`** that matches `resource_scope.invocation_id`.
- A **`Capability`** is *declared* by a **`CapabilityDescriptor`** (in an `ExtensionManifest`), *bound to a Principal* by a **`CapabilityGrant`**, *granted dynamically* by a **`CapabilityLease`**, and *invoked once* via a **`CapabilityDispatchRequest`** carrying an **`InvocationId`**.
- A **`Decision::Allow`** may carry **`Obligation`**s, which the **`CapabilityObligationHandler`** must satisfy *before* dispatch; obligation failure aborts before the runtime lane runs.
- An **`ApprovalRequest`** transitions through **`ApprovalStatus`**: `Pending → Approved (issues a fingerprinted CapabilityLease) | Denied | Expired`. Resolution is recorded as **`AuditEnvelope { stage: ApprovalResolved }`**, not a **`RuntimeEvent`**.
- A **`Process`** is created by `CapabilityHost::spawn_json` after authorization; its **`ProcessRecord`** carries the same scope/grants/mounts/estimate as the `ExecutionContext` that produced it; observers go through **`ProcessHost`**.
- A **`SecretHandle`** is opaque authority; **`SecretMaterial`** is value; **`SecretLease`** is one-shot redemption. The handler delivers material exactly once via the `InjectSecretOnce` obligation.
- **`MountView ⊆ MountView`** is the substitution rule: a child mount view must be a subset of its parent's permissions; runtimes never see `HostPath`.
- **`RuntimeDispatcher`** routes a `CapabilityDispatchRequest` to one **`RuntimeAdapter`** by **`RuntimeKind`**; the adapter returns a **`CapabilityDispatchResult`** plus a **`ResourceReceipt`** that the **`ResourceGovernor`** reconciles.
- An **`ExtensionPackage`** is *what can run*; a **`Process`** is *what is running*; a **`Thread`** is *the durable record of work* that may span many invocations. These three never collapse.
- **`ironclaw_host_api`** owns shapes only; it must not depend on `ironclaw_dispatcher`, `ironclaw_filesystem`, `ironclaw_resources`, `ironclaw_extensions`, `ironclaw_wasm`, `ironclaw_mcp`, `ironclaw_scripts`, `ironclaw_authorization`, or `ironclaw_network`. The dependency direction defines the architecture.

## Example dialogue

> **Dev:** "I want to add a `slack.post_message` capability. Where do I start?"
>
> **Domain expert:** "Add a `CapabilityDescriptor` to your extension's `ExtensionManifest` with `runtime = Mcp` (or whatever lane) and the declared `EffectKind`s — `Network` and `UseSecret` if it posts via Slack's API. The `ExtensionRegistry` will register it. That tells the host *what can run*."
>
> **Dev:** "And the user has to be allowed to actually call it?"
>
> **Domain expert:** "Yes. Registration is not authority. The caller's `ExecutionContext` needs a matching `CapabilityGrant` in its `CapabilitySet`, with `GrantConstraints.allowed_effects` covering the descriptor's effects. If there's no grant, `GrantAuthorizer` returns `Decision::Deny { reason: MissingGrant }` before the dispatcher ever sees the call."
>
> **Dev:** "What if the grant exists but Slack requires a one-time approval?"
>
> **Domain expert:** "Authorizer returns `Decision::RequireApproval`. `CapabilityHost::invoke_json` computes an `InvocationFingerprint`, saves a `Pending` `ApprovalRequest`, marks the run `BlockedApproval` in `RunStateStore`, and bubbles a typed approval-required error. When the user approves, `ApprovalResolver` issues a fingerprinted `CapabilityLease`. The caller calls `resume_json`, which recomputes the fingerprint, claims the lease, and dispatches. The lease is consumed on success."
>
> **Dev:** "And the Slack token?"
>
> **Domain expert:** "The grant references a `SecretHandle`; authorization checks that, and the obligation handler does an `InjectSecretOnce` against the `EncryptedSecretStore` to produce `SecretMaterial` exactly once for the runtime egress call. The `HardenedHttpEgressClient` enforces the `NetworkPolicy` from `GrantConstraints` independently — no destination outside the policy can be reached even if the token is correct."
>
> **Dev:** "If this is a long-running listener, not a one-shot post?"
>
> **Domain expert:** "Then it's `spawn_json`, not `invoke_json`. You need `EffectKind::SpawnProcess` plus the descriptor's effects. `ProcessManager` creates a `ProcessRecord` carrying scope, grants, mounts, and estimate; `ProcessHost` is how callers observe status. The `ProcessRecord` is *not* a host OS process and *not* a `Thread` — `Thread` is the durable logical work record that can span many invocations and processes."
>
> **Dev:** "Where does this all get wired together?"
>
> **Domain expert:** "`ironclaw_host_runtime`. Composition only — it doesn't define new authority. The architecture FAQ used to call this 'the kernel', but the live crate is `ironclaw_host_runtime`. There is no `ironclaw_kernel`. And `ironclaw_dispatcher` is a different thing: it routes already-authorized requests to runtime adapters, nothing more."

## Flagged ambiguities

- **"Capability"** is overloaded across three layers: an extension *declaration* (`CapabilityDescriptor`), an authority *grant* (`CapabilityGrant`), and a one-shot *lease* (`CapabilityLease`). Always qualify which one. The shorthand "capability" usually means the descriptor; "the user has the capability" means the grant.
- **"Capabilities"** is *also* used for `MemoryBackendCapabilities`, which are **backend support declarations**, not authority. The memory contract notes a planned rename to `MemoryBackendSupport`. Until then: write *backend support* in prose; reserve *capability* for caller-visible actions.
- **"Kernel"** appears in older architecture docs. The concrete composition crate is **`ironclaw_host_runtime`**; there is no `ironclaw_kernel`. Treat *kernel* as architecture vocabulary, *`ironclaw_host_runtime`* as code vocabulary.
- **"Dispatcher"** has two meanings: **`RuntimeDispatcher`** in `ironclaw_dispatcher` (Reborn, narrow runtime-lane router) and **`ToolDispatcher`** in legacy production code (broad agent/tool dispatch). When discussing Reborn, prefer the full name or "the runtime dispatcher".
- **"Scope"** appears in five distinct senses — qualify on first use:
  - **`ResourceScope`** (tenant/user/agent/project/mission/thread/invocation cascade)
  - **`MemoryDocumentScope`** (memory partition key)
  - **`ApprovalScope`** (reusable scope on an approval)
  - **layer scope** (memory layer namespace: private/shared/team)
  - **search scope** (primary vs secondary read scope inside memory)
- **"Process"** is **`ProcessRecord`** in Reborn; *not* a host OS process and *not* an agent loop "session". A long conversation maps to a **`Thread`**, not a process.
- **"Thread"** is the durable logical work record (`ThreadId`); not an OS thread, not a Honcho session, not a chat message stream.
- **"Trust"** has two related but distinct uses: a runtime carries a **`TrustClass`** (its current tier) while a `CapabilityDescriptor` declares a **trust ceiling** (its maximum permitted tier). They are not interchangeable; the runtime class must be `<= ceiling`.
- **"System"** appears as **`Principal::System`** (narrow explicit authority, *not* a wildcard) and as **`RuntimeKind::System`** / **`TrustClass::System`** (system-runtime tier). Distinct concepts; always qualify.
- **"Action"** has two senses: the typed `Action` enum on `ApprovalRequest` (the thing being approved) and the colloquial "action" meaning a capability invocation. Prefer **invocation** for "one call" and reserve **`Action`** for the typed enum.
- **"Event"** is overloaded between **`RuntimeEvent`** (append-only runtime/process events) and **`AuditEnvelope`** (control-plane audit). Architecture law: *realtime events are not the audit log*. Always qualify.
- **"Backend"** drifts across filesystem (`RootFilesystem` storage backend), memory (`MemoryBackend`), script (`ScriptBackend`/`DockerScriptBackend`), and the encrypted-secret repository. The word is fine inside a subsystem; across subsystems, qualify (e.g. "filesystem backend", "memory backend").
- **"Plugin" / "Provider" / "Adapter"** drift in conversation. Recommendation: **provider** for the extension that *declares* a capability (`CapabilityDescriptor::provider`); **adapter** for the runtime impl behind a `RuntimeKind` (`RuntimeAdapter`); avoid bare *plugin* in code-facing prose.
- **"Authority" vs "Permission" vs "Policy"** — *authority* is what an `ExecutionContext` carries (grants + leases + mounts + scope); *policy* is what a host service evaluates (`NetworkPolicy`, `MountPermissions`); *permission* is a per-mount flag (`MountPermissions::read`). Don't say "permissions" when you mean "grant".
- **"Reservation"** vs **"Lease"** — both are time-bounded, both are one-shot-ish, but they live in different services: **`ResourceReservation`** is budget capacity; **`SecretLease`/`CapabilityLease`** are authority handles. Never write "secret reservation" or "budget lease".
- **"Manifest"** is fine inside extensions (`ExtensionManifest`), but in scripts and MCP it appears as "manifest-derived command contract" and "manifest-declared MCP tool". Always qualify or prefix when crossing crates.
