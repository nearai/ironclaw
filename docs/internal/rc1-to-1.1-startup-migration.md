# 1.0.0-rc.1 to 1.1.0-rc.1 startup migration

This runbook covers only the exact release pair `ironclaw-v1.0.0-rc.1` to
`ironclaw-v1.1.0-rc.1`. The database v33/v34 migrations are necessary to make
the 1.1 storage substrate readable, but they are not sufficient: most affected
state is encoded as versioned records inside `RootFilesystem`, not as SQL
columns.

## Migration matrix

| Domain | rc1 authority | 1.1 authority | Startup action | Rollback authority |
| --- | --- | --- | --- | --- |
| SQL substrate | libSQL/PostgreSQL schema through v32 | schema through v34 | Backend-transactional, additive v33/v34 DDL | Existing rows and additive columns remain readable by rc1 |
| Thread headers | scoped `thread.json` rows and legacy index projection | canonical scoped headers and projection v2 | Discover every tenant/scope, validate identity and scope, rewrite projection | Same backward-readable header rows |
| Thread messages | transcript files plus rc1 append-event tail | canonical transcript plus projection | Page the complete tail; file transcript wins; append-only messages are materialized exactly once and projected in order | Original transcript and append events are retained |
| Channel conversations | rc1 conversation roots | canonical conversation state | Merge roots collision-safely and verify every referenced canonical thread | rc1 roots are retained |
| Idempotency | rc1 action records and transient leases | canonical filesystem ledger | Page all actions, copy durable outcomes, expire transient leases | rc1 action records are retained |
| Processes | legacy process journal, locks, checkpoints, reservations | canonical process journal | Page all records; import journal; explicitly expire or supersede non-replayable control state | Legacy records are retained |
| Slack OAuth | `slack_personal` provider/account/flow rows | `slack` provider rows | Copy exact account identities, handles, and tokens; expire incomplete flows; verify readback | Versioned backup plus original provider rows |
| Extension installations | per-tenant `.installations/state.json` snapshots | global compatibility rows plus normalized v2 records | Discover every hosted snapshot, import manifests/installations, preserve owner, bindings, and activation state | Monolithic snapshots are retained |
| Extension activation | rc1 `installed`/`disabled`/`enabled` state | normalized installation activation state | Preserve state; restore must never widen installed/disabled to enabled | rc1 snapshot plus compatibility row |
| Slack setup | setup, identities, routes, DM targets, connection rows | admin configuration, generic identities/routes/DM targets | Import usable setup; active connections are superseded by canonical identity/OAuth state; stale connections expire; interrupted disconnect fails closed | All provider-specific rows are retained |
| Telegram setup | setup, identities, DM targets, pairing rows | admin configuration, generic identities/DM targets | Import usable setup; one-time pairing challenges and pending completions expire explicitly | All provider-specific rows are retained |
| Triggers | backend trigger repository | same backend repository | Run existing backend schema migration and verify repository access; no record transform | Same trigger rows |
| User/system skills | scoped filesystem skill roots | same roots through 1.1 mount catalog | Inventory and verify paths remain reachable; no content transform | Same skill files |

Each transformed domain writes its own versioned completion marker. The final
release-pair completion is written only after all readback and cross-domain
checks pass. Thread and channel domains include one count-only entry per
discovered scope; scope identities are deliberately omitted. Reports contain
counts and dispositions only; they must not contain record paths, actor
identifiers, message contents, credentials, or secret handles.

## Failure and restart

- The startup lease is acquired before record transforms. A second writer
  fails closed. The owner heartbeats the lease every minute while composition
  and readback are still running; losing the lease fails the completion CAS.
- Malformed, ambiguous, or conflicting P0/P1 state fails startup and does not
  publish completion.
- A normal error path marks the attempt failed immediately. A dropped lease has
  a best-effort fail guard so an ordinary builder failure does not impose the
  full lease timeout.
- Every transform is CAS-guarded and repeatable. A restart revalidates retained
  source authorities and produces a zero-change aggregate report.
- Non-replayable ephemeral state is never guessed: active process locks,
  reservations, incomplete OAuth flows, stale connection attempts, and pairing
  challenges receive explicit expiration/supersession dispositions.

## Rollback procedure

1. Stop every 1.1 process before starting an rc1 process. Never run the two
   binaries concurrently against the same database.
2. Preserve a database-native snapshot when the deployment platform provides
   one. For a local libSQL file, copy the database and its WAL/SHM companions
   while IronClaw is stopped.
3. Confirm the completion report says `old_authorities_retained=true` and
   `in_place_rows_backward_readable=true`.
4. Start the exact `ironclaw-v1.0.0-rc.1` binary against the retained database.
   Verify thread listing and transcript reads before admitting writes.
5. If rc1 verification fails, stop it and restore the pre-upgrade
   database-native snapshot. Do not delete 1.1 completion markers or normalized
   rows by hand.

The application migration intentionally does not delete rc1 authorities. This
is the rollback snapshot for record transforms; a database-native snapshot is
still recommended because it also protects the substrate DDL and unrelated
state.
