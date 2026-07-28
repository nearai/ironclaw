# IronClaw Mobile App Plan

**Status:** Milestone 1 implementation in progress
**Date:** 2026-07-24  
**Owners:** Product, mobile, WebUI host, product workflow  
**Target:** iOS and Android access to a user's hosted or dedicated IronClaw agent

## 1. Decision summary

Build a dedicated cross-platform mobile client that connects to the existing
Reborn product surface. Keep conversation, turn, authorization, projection, and
runtime policy in the server. The mobile app is a product adapter and
presentation layer; it must not become another source of truth or execute agent
capabilities locally.

Use React Native with TypeScript and Expo as the client stack. It fits
the existing React/TypeScript UI skills while still providing native secure
credential storage, deep links, background notification handling, and
App Store/Play distribution.

The primary audience is people who want to access their agent on the go for
asking questions, knowledge work, and coding. The app must make previously
viewed information available when it opens offline; network access is required
only to refresh data or submit work.

The primary service is the hosted IronClaw deployment at
`https://agent.near.ai`. Non-production builds use
`https://agent-stg.near.ai`. Users sign in with Google, Apple, GitHub, or a
NEAR account and land in their hosted agent. Dedicated deployments remain
supported through pairing and a saved custom HTTPS URL, but that path is
secondary in the interface.

The first release is a secure remote companion focused on:

- signing in to the hosted IronClaw service with Google, Apple, GitHub, or a
  NEAR account;
- pairing with and saving a dedicated deployment;
- listing, creating, renaming, and deleting threads;
- reading durable timelines and sending messages with attachments;
- following live run progress and reconnecting without losing durable state;
- resolving approval and authentication gates;
- cancelling and retrying runs;
- receiving privacy-safe notifications for actionable or terminal events.

Operator setup, provider configuration, extension installation, filesystem
administration, and other broad-authority surfaces remain in the WebUI for the
first release.

### Implementation checkpoint — 2026-07-24

The Expo application shell is now present under `app/` with:

- native iOS and Android configuration at the selected OS floors;
- hosted account OAuth bootstrap and hosted IronClaw instance discovery;
- advanced direct connection for a dedicated WebChat v2 deployment;
- SecureStore credentials and a SQLCipher-backed durable read cache;
- offline thread, timeline, automation, and draft rendering;
- thread creation, text messaging, and foreground timeline reconciliation;
- automation pause, resume, rename, and delete controls;
- tool settings and global auto-approval controls.

TypeScript checking, unit tests, Expo web export, Expo Doctor, native Expo
prebuild, and an Xcode iOS simulator build pass. The current staging account
frontend and control plane are reachable, but its deployed agent image must
expose the Reborn WebChat v2 contract at the instance `dashboard_url` before
hosted end-to-end chat can pass.
The client detects and reports an HTML frontend fallback rather than
misclassifying it as an authenticated API response.

## 2. Why this shape

The current Reborn stack already supplies the important server foundations:

- `ironclaw_webui` hosts authenticated REST routes plus SSE and WebSocket event
  streams;
- handlers consume the `ProductSurface` boundary instead of runtime or storage
  internals;
- durable threads and timelines are distinct from live projection streams;
- stream cursors support replay and explicit rebase after retention gaps;
- authenticated caller identity scopes access to tenant, user, agent, and
  project resources;
- the frontend is responsive and already includes an installable web manifest.

A native app should reuse those contracts. Reimplementing orchestration,
conversation state, projections, or capability execution in a mobile-specific
backend would create competing authority paths and violate Reborn boundaries.

## 3. Product goals and non-goals

### Goals

1. Let a user ask questions, perform knowledge work, and supervise coding work
   through their agent away from a desktop.
2. Open reliably without a network connection and show previously viewed
   threads and conversation information from a durable, encrypted local cache.
3. Make approvals, authentication gates, failures, and completed work easy to
   notice and act on.
4. Survive app suspension, network changes, and reconnects without duplicating
   messages or losing durable conversation state.
5. Make the hosted service the fastest path while retaining secure pairing for
   dedicated deployments.
6. Preserve server-side authorization, redaction, rate limits, body limits, and
   side-effect evidence.
7. Share generated API types and behavioral fixtures where practical, without
   coupling mobile presentation code to the WebUI SPA.

### Non-goals for v1

- running the agent, models, MCP servers, WASM, scripts, or other capabilities
  on the phone;
- complete parity with operator/admin settings;
- offline agent execution or offline mutation of server state;
- backgrounding a permanent socket;
- rendering raw tool arguments, raw tool output, secrets, or host paths;
- silently discovering or connecting to arbitrary hosts on a local network;
- supporting multiple simultaneous signed-in users within one deployment.

## 4. User experience scope

### Release 1: companion core

| Area | Included |
| --- | --- |
| Hosted service | Production routes to `agent.near.ai`; internal/staging builds route to `agent-stg.near.ai` |
| Identity | Google, Apple, GitHub, and NEAR account sign-in; logout and remote session revocation |
| Dedicated deployment | Pair by QR code, one-time code, or verified link; save, switch, rename, or remove the custom HTTPS deployment |
| Threads | List, paginate, create, rename, delete, unread/action-needed state |
| Chat | Offline-first durable timeline, markdown/code rendering, text input, attachment upload, send retry with idempotency |
| Runs | Live status, capability-safe summaries, cancel, retry, reconnect/rebase |
| Gates | Approve/deny exact invocation, open OAuth authorization, submit supported manual interactions |
| Notifications | Approval needed, auth needed, run completed, run failed; content hidden by default |
| Settings | Theme, language, notification preferences, diagnostics export, deployment management |

### Later releases

- projects and project switching;
- automations and read-focused run monitoring;
- share-sheet input, camera/photo capture, and document scanning;
- tablet and split-view layouts;
- opt-in richer notification previews;
- passkeys/device-bound login if the server identity contract supports them;
- carefully selected settings or extension-management tasks;
- accessibility refinements based on production audits and user testing.

## 5. Client architecture

Create a new top-level mobile workspace under `app/` rather than placing native
code in a Rust crate or inside the WebUI bundle. A proposed layout is:

```text
app/
  package.json
  app.config.ts
  src/
    api/             generated wire types, transport, error mapping
    auth/            deployment sessions, pairing, OAuth deep links
    deployments/     local deployment profiles and trust prompts
    features/        threads, chat, runs, gates, settings
    notifications/   device registration and notification routing
    storage/         secure credentials, encrypted SQLite, migrations, retention
    navigation/
    design-system/
  ios/
  android/
  tests/
```

Use the current stable Expo SDK with development builds. Adopt custom native
modules only when a measured requirement cannot be met through maintained Expo
modules. Do not eject merely for hypothetical future needs.

The supported OS floor is:

- **iOS 16.4 and newer**;
- **Android 13 and newer (API level 33 minimum)**.

Compile and target the current store-required SDKs independently of the install
floor; the initial Android build targets API level 36. Revisit the floor
annually so supported operating systems remain no more than approximately four
years old.

The app uses three client-side data classes:

1. **Credentials and deployment trust metadata** live only in platform secure
   storage.
2. **Server-owned durable data** is cached locally for fast rendering but is
   always reconciled with the server.
3. **Ephemeral UI state** may live in normal application storage and must be
   namespaced by deployment plus authenticated user.

No raw capability result, secret value, access token, or sensitive attachment
is written to analytics, crash reports, notification payloads, clipboard
history, or unencrypted application storage.

### Offline durability

Use `expo-sqlite` with SQLCipher in Expo development and production builds.
Store the randomly generated database key in `expo-secure-store`; do not derive
it from a user password or place it in application configuration. The durable
cache is an explicitly versioned read model, not an HTTP-response cache.

Maintain a separate encrypted database per deployment and authenticated user.
Each database contains:

- deployment and authenticated-user display metadata;
- thread summaries and pagination positions;
- previously fetched durable timeline items;
- locally authored composer drafts;
- projection snapshots and the last fully committed scoped cursor;
- mutation receipts and client action IDs needed to reconcile uncertain sends;
- cache metadata, schema version, and last successful synchronization time.

Durability rules:

1. Open and migrate the database before mounting authenticated application
   navigation. A failed migration preserves the old database for diagnostics
   and presents a recoverable local-data error; it does not silently reset.
2. Commit a fetched timeline page, its pagination metadata, and its cursor in
   one SQLite transaction. Never advance a cursor before its corresponding
   rows are durable.
3. Apply stream updates transactionally and idempotently by stable event or
   projection identity.
4. Render cached data immediately at launch, including when the network is
   absent. Label it with offline/staleness state and its last sync time.
5. Reconcile cached state against the server on foreground, connectivity
   restoration, account switch, and notification open. Explicit rebase replaces
   derived projection state but preserves durable drafts.
6. Keep message submission as an outbox record until the server returns a
   durable receipt or the subsequent read confirms the message by client action
   ID. An uncertain send is shown as such and is never automatically duplicated.
7. Do not automatically queue approvals, denials, authentication submissions,
   cancellations, or other time-sensitive side effects while offline.
8. Cache attachments only when explicitly required for an in-progress draft.
   Store them in protected application storage, record their lifecycle in the
   database, and remove them after send, discard, logout, or expiry.
9. Start with a 30-day, 250 MiB per-account budget and least-recently-viewed
   eviction, while always retaining drafts and mutation receipts. Validate and
   tune these limits with beta data.
10. On logout or deployment removal, close the database, delete its files and
    attachment cache, and remove its secure-store key. Exclude cached content
    from platform backups unless a reviewed encrypted-backup design is added.

Repository and database migrations must be crash-tested. The app must be able
to launch into cached read-only mode after process termination at every
transaction boundary.

## 6. Server contract and ownership

The mobile client should consume the existing WebChat v2 product contract for
threads, timelines, messages, runs, gates, attachments, and projections.
Before app implementation, publish a versioned, machine-readable mobile
contract derived from the same route descriptors and wire DTO owners. Generated
client models must not become a hand-maintained mirror of Rust domain types.

Ownership remains:

| Concern | Owner |
| --- | --- |
| Mobile screens, navigation, secure local cache | `app/` |
| HTTP routes, transport framing, host authentication | `ironclaw_webui` |
| Product-facing commands, views, and wire DTOs | `ironclaw_product` / existing product surface owner |
| Product facade wiring | `ironclaw_reborn_composition` |
| Threads, turns, gates, projections, runtime | Existing Reborn domain crates |
| Durable events and read models | Existing event/projection crates |
| External notification preference/attempt metadata | `ironclaw_outbound` |
| APNs/FCM delivery adapter | A host-side product adapter, not composition or a projection crate |

Any new endpoint must follow the existing vertical:

```text
mobile app
  -> ironclaw_webui route, policy, and handler
  -> ProductSurface
  -> product command/view service
  -> owning Reborn domain
```

Mobile routes must not directly call a dispatcher, runtime lane, database,
filesystem, secret store, or event store.

### Contract gaps to close during M1

1. **Capability negotiation:** expose server/API version, supported mobile
   features, attachment limits, stream modes, and authentication methods.
2. **Hosted mobile login:** support Google, Apple, GitHub, and NEAR account
   authorization for `agent.near.ai`, using the staging host only in
   non-production builds.
3. **Dedicated deployment pairing:** add a short-lived, single-use exchange
   that binds a verified custom HTTPS origin and returns a scoped device
   session. Never ask users to copy a long-lived operator bearer into the app.
4. **Session lifecycle:** support short-lived access credentials, renewable
   sessions, explicit logout/revocation, and device/session naming.
5. **Idempotent mutations:** require a client action ID for message submission
   and every retryable side effect.
6. **Background resync:** provide a bounded “changes since cursor” or equivalent
   projection read so the app does not depend on a continuously open socket.
7. **Push registration:** add authenticated register, rotate, list, and revoke
   operations for opaque device delivery handles.
8. **Deployment identity:** define what the user verifies when first connecting
   and how a changed origin or certificate is presented.

Do not add mobile-only domain DTOs where an existing product DTO is sufficient.
When an app constraint requires a new wire shape, keep it at the product/host
boundary and map to the canonical domain types.

## 7. Authentication and deployment connection

Support two deployment classes with deliberately different prominence:

- **Hosted, primary:** `agent.near.ai`, with Google, Apple, GitHub, and NEAR
  account choices presented on the first login screen. Development and internal
  beta builds use `agent-stg.near.ai`; production builds must not silently
  fall back to staging.
- **Dedicated, secondary:** a custom HTTPS deployment added from an Advanced
  action through QR, one-time code, or verified universal/app link pairing. The
  successful pairing saves the deployment URL and display name so users do not
  need to enter it again.

Plain HTTP is allowed only for an explicit debug build pointed at loopback.
Production builds reject it.

Hosted login sequence:

1. User chooses Google, Apple, GitHub, or NEAR account.
2. App opens the hosted service authorization endpoint in a system
   authorization session, using PKCE and an exact allowlisted callback.
3. The hosted service completes the selected identity flow and exchanges the
   authorization code for a scoped mobile device session.
4. The app accepts the callback only for its pending provider, state, PKCE
   verifier, build environment, and hosted origin.
5. App stores credentials in Keychain/Keystore-backed secure storage.
6. App fetches the authenticated session and verifies the returned identity and
   hosted deployment match the pending login.

Dedicated pairing follows the same final device-session contract, but begins
from a short-lived pairing artifact minted by that deployment. The artifact
must bind the exact HTTPS origin, expire quickly, and be single-use. The app
shows the resolved origin and deployment identity for confirmation before
saving it.

The app must never inherit operator configuration privileges merely because the
deployment also has an operator token. A mobile device session receives only
the authenticated user's scoped capabilities.

Deep links must accept opaque references only. Resolve the referenced thread,
run, or gate through an authenticated server read before navigation; never
derive authority from link contents.

## 8. Realtime, suspension, and offline behavior

Use the durable timeline and projection snapshot as the display source of truth.
Use a live stream only while the app is foregrounded and a relevant screen is
active.

Connection algorithm:

1. Fetch the current durable snapshot/timeline.
2. Open the supported event stream with its last scoped cursor.
3. Apply ordered updates after schema and scope validation.
4. On a lag/rebase response, discard derived live state and refetch the durable
   snapshot.
5. On foreground, network restoration, or push open, repeat reconciliation.
6. Close the live stream when the app backgrounds.

Prefer the existing WebSocket projection stream for foreground updates if its
authentication mechanism is proven reliable on both native platforms.
Otherwise add a short-lived, scoped stream ticket obtained through an
authenticated POST. Do not place a durable bearer in a URL. SSE remains a
server-supported browser transport and need not be the mobile transport.

Offline v1 behavior is durable and deliberately read-oriented:

- launch without waiting for discovery, login refresh, or a network timeout;
- render saved deployments, cached thread lists, and previously viewed
  timelines with a clear offline marker and last-sync time;
- retain unsent composer text locally;
- queue an attachment only while its local file still exists and the user can
  review it;
- require explicit user action to retry a message after connectivity returns;
- never queue approval, denial, auth submission, cancellation, or another
  time-sensitive side effect for automatic later execution.

## 9. Push notifications

Push is an alert channel, not a durable event stream or source of truth.

The server selects an authorized, redacted outbound candidate, records the
delivery attempt, and sends a minimal payload through APNs or FCM. The payload
contains an opaque notification reference, deployment identifier, coarse event
kind, and no prompt, response, tool input/output, host path, secret, or
attachment content. After the user opens it, the app authenticates and fetches
current state from the source deployment.

Notification preference defaults:

- approval/auth required: on;
- run completed/failed: on;
- message previews: off;
- sound and badges: platform defaults, user configurable.

The design must document token rotation, uninstall/revocation, expired devices,
multi-device fan-out, delivery deduplication, provider retry behavior, and an
operator-visible kill switch. If notification delivery requires a third-party
relay, make that deployment choice explicit and opt-in; the default
self-hosted path must not leak conversation data to the relay.

## 10. Mobile security checklist

- HTTPS only in production; use platform trust validation and do not ship an
  “accept any certificate” switch.
- Store session secrets only in secure storage and redact them from all logs.
- Use system browser authorization with PKCE; do not embed provider login pages
  in an unrestricted WebView.
- Allowlist callback schemes, universal links, and outbound authorization URLs.
- Require biometric/app-lock support as an opt-in local privacy control.
- Blur sensitive screens in the app-switcher snapshot.
- Clear credentials and sensitive cache on logout or deployment removal.
- Namespace every cache, cursor, draft, and notification by deployment and
  authenticated user.
- Preserve server body limits, attachment validation, rate limits,
  authorization checks, approval leases, and redacted error vocabulary.
- Treat rooted/jailbroken-device detection as a signal, not an authority
  boundary or a reason to weaken server checks.
- Establish dependency, SBOM, signing-key, provenance, and secret-scanning
  procedures before store submission.

Threat-model review is a beta gate. It must cover malicious deployment URLs,
SSRF-like client behavior, compromised deep links, token theft, notification
privacy, cross-account cache bleed, stale approval actions, replayed mutations,
attachment exfiltration, screenshots, clipboard use, and diagnostic exports.

## 11. Delivery milestones

### M1 — app shell, hosted login, offline durability, and durable chat

- Scaffold `app/`, CI checks, environment separation, and signed development
  builds with Expo.
- Configure iOS 16.4+ and Android 13/API 33+ install floors, with Android
  targeting the current Play-required API.
- Add hosted Google, Apple, GitHub, and NEAR account login against
  `agent-stg.near.ai` for internal builds and `agent.near.ai` for production.
- Add secure login/logout, session refresh, revocation, capability discovery,
  and build-environment host pinning.
- Add the secondary dedicated-deployment pairing flow and saved deployment
  profiles.
- Build navigation, design tokens, localization plumbing, error handling,
  diagnostics, and secure credential storage.
- Add contract-generation and compatibility checks.
- Add the encrypted SQLite read model, migrations, per-account isolation,
  transactional cursor checkpoints, bounded eviction, and deterministic purge.
- Implement thread list/detail/create/rename/delete.
- Implement timeline pagination, composer drafts, message submission
  idempotency, attachment selection/upload, and retry UX.
- Render cached threads and timelines immediately after cold offline launch.
- Reconcile uncertain sends without duplication after restart or reconnect.
- Meet baseline screen-reader, dynamic-type, contrast, keyboard, and reduced
  motion requirements.

**Exit:** signed internal iOS and Android builds can use every hosted login
method, pair a dedicated deployment, complete core chat flows, and cold-launch
offline into previously viewed information. App termination during migration,
pagination, cache eviction, upload, or message submission does not corrupt the
cache, duplicate a send, or expose another account's data.

### M2 — live supervision and gates

- Add foreground projection streaming, cursor resume, rebase, and lifecycle
  handling.
- Render safe run/capability progress.
- Add cancel, retry, approval, denial, and auth-gate flows.
- Add background/foreground reconciliation and stale-gate protection.

**Exit:** scripted whole-turn tests prove message-to-terminal-state,
disconnect/reconnect, approval, auth, cancellation, and retry paths.

### M3 — notifications and external beta

- Add device registration/revocation and minimal APNs/FCM payloads.
- Implement notification preferences, deep-link resolution, and fetch-on-open.
- Complete privacy review, penetration testing, accessibility audit, crash and
  performance budgets, and store beta distribution.
- Publish deployment compatibility and support documentation.

**Exit:** external beta meets reliability, privacy, and support-readiness gates.

### M4 — store launch

- Close beta-blocking defects and validate upgrade/rollback paths.
- Complete App Store/Play privacy disclosures and review artifacts.
- Exercise server rollback, API compatibility, credential revocation, push kill
  switch, and mobile release rollback.
- Publish release notes, support runbooks, and a compatibility matrix.

**Exit:** phased production rollout with monitored crash-free sessions,
successful reconnects, message send success, and notification-to-current-state
resolution.

## 12. Test strategy

Use the narrowest tier that proves each boundary, with caller-level coverage
whenever a helper gates a side effect.

| Tier | Required coverage |
| --- | --- |
| Mobile unit/component | reducers, cursor handling, URL validation, redaction, secure-store wrappers, SQL migrations, cache namespacing/eviction, outbox reconciliation, drafts, accessibility semantics |
| Mobile integration | all hosted auth callbacks, dedicated pairing, session expiry, cold offline launch, process termination during writes, offline/online transitions, attachment retry, notification open |
| Rust crate contract | new DTOs, route descriptors, capability discovery, session/device lifecycle, push registration, redacted errors |
| Reborn integration | real product facade through threads/turns/projections for send, gate resolution, cancellation, retry, reconnect/rebase |
| Browser/WebUI regression | existing WebUI behavior remains unchanged where contracts are shared |
| Device E2E | iOS and Android against a hermetic server with scripted model/provider doubles |
| Live canary | supplemental OAuth and push-provider drift checks; never the sole gate |

Key adversarial scenarios:

- cold launch in airplane mode after process and device restart;
- termination before, during, and after every database migration and
  page/cursor transaction;
- corrupted or missing database key, failed migration, full disk, eviction, and
  logout purge;
- callback provider/origin/build mismatch across Google, Apple, GitHub, and
  NEAR account login;
- stale cursor, replay gap, duplicate event, reordered reconnect, and stream
  capacity exhaustion;
- duplicate send after timeout, app kill during upload, and attachment removal;
- expired or already-resolved approval and auth gates;
- logout during an in-flight request and user/deployment switching;
- malicious deep link, changed deployment origin, off-origin attachment URL,
  and untrusted OAuth URL;
- notification delivery after logout, token rotation, and multiple devices;
- server version older/newer than the client capability contract;
- accessibility at largest supported text size and with a screen reader.

New server routes require descriptor and handler contract coverage in
`ironclaw_webui`; cross-layer behavior requires an integration test at the
production facade seam. Dependency-edge changes also require
`cargo test -p ironclaw_architecture`.

## 13. Release, compatibility, and operations

Version the mobile API independently from the app release. The capability
response identifies supported features and minimum/maximum compatible client
contract versions. Additive fields are ignored safely; removed or semantically
changed behavior requires a new contract version and a documented migration
window.

Use separate development, beta, and production application identifiers,
signing credentials, OAuth clients, deep-link domains, and push environments.
No production secret belongs in the repository or JavaScript bundle.

Roll out by percentage and retain the ability to:

- stop a mobile release;
- disable push registration or delivery server-side;
- revoke a device session;
- disable a newly introduced route through configuration;
- fall back from live streaming to durable polling/reconciliation;
- keep the WebUI as the supported recovery/admin surface.

## 14. Success metrics

Track privacy-preserving aggregates for:

- successful sign-in and deployment connection;
- crash-free sessions and app startup latency;
- message submission success and duplicate-send prevention;
- foreground stream connection and cursor-resume success;
- time from actionable gate creation to resolution;
- notification open to successfully reconciled current state;
- logout/revocation success;
- support incidents caused by API incompatibility or connectivity.

Do not collect message bodies, prompts, responses, attachment names/content,
tool input/output, secrets, host paths, or full deployment URLs.

## 15. Open decisions

The product and platform choices are fixed. These remaining implementation
decisions must be resolved with ADRs before the named milestone:

| Decision | Deadline | Default proposal |
| --- | --- | --- |
| Stream authentication | M1 | Existing WS if cross-platform-safe; otherwise short-lived scoped ticket |
| API schema generation | M1 | Generate from product-owned versioned wire schema |
| Hosted identity protocol details | M1 | PKCE device session for Google, Apple, GitHub, and NEAR account |
| Cache retention tuning | M1 | SQLCipher; 30 days and 250 MiB per account; no raw capability results |
| Push topology | M2 | Direct deployment-to-provider where practical; relay only explicit/opt-in |
| Telemetry | M2 | Opt-in or operator-configured, metadata-only |
| Tablet support at launch | M1 | Responsive support, not a tablet-specialized information architecture |

## 16. Immediate next actions

1. Scaffold the Expo workspace and signed development builds for iOS and
   Android.
2. Assign mobile, hosted-auth, WebUI host, product-surface, security, and
   release owners.
3. Produce a route/DTO compatibility inventory from live
   `ironclaw_webui` descriptors and product wire types.
4. Implement the encrypted database bootstrap, migration harness, scoped cache,
   and offline cold-start skeleton before building screens on an in-memory
   cache.
5. Implement hosted Google, Apple, GitHub, and NEAR account login against
   staging, then add dedicated-deployment pairing.
6. Build the first durable vertical: sign in, list threads, open one cached
   timeline, send a message with a client action ID, terminate/reopen offline,
   then reconnect and reconcile.
7. Continue M1 feature work while validating the three target journeys—asking
   questions, knowledge work, and coding—with users on real devices.
