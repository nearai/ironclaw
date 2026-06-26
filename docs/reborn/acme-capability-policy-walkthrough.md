# Acme — capability-policy hand-driven walkthrough (epic #5261)

Stand up a single hosted IronClaw Reborn as the company **Acme**, be the admin,
create users, grant per-user policy across all four dimensions over REST, watch
it enforced at dispatch, and confirm it survives a restart. This is the
**milestone manual test** — no seed script, no automated e2e.

> Everything here is gated behind the `capability-policy` Cargo feature **and**
> the `IRONCLAW_REBORN_CAPABILITY_POLICY` env flag. With the flag off the
> runtime keeps `AllowAll` behaviour (existing tests unchanged).

## Cast

| Who | Account | Role | Notes |
|---|---|---|---|
| operator | env bearer | Owner | bootstrap identity; mints the first admin |
| director@ | REST-created | Admin | the company admin who grants access |
| Bob | REST-created | Member | a personal user |
| Carol | REST-created | Member | a personal user |
| engineering@ | REST-created | Member | the shared "room-agent" account you SSO into |

## 0. Boot `serve` with the policy engine on

```bash
export IRONCLAW_REBORN_HOME="$HOME/.ironclaw-acme"          # durable libSQL DB lives here
export IRONCLAW_REBORN_WEBUI_TOKEN="operator-secret-please-change"
export IRONCLAW_REBORN_WEBUI_USER_ID="operator"
export IRONCLAW_REBORN_CAPABILITY_POLICY=1                  # activate the four-dimension policy

cargo run -p ironclaw_reborn_cli --features webui-v2-beta,capability-policy -- \
  serve --port 8787
```

Convenience env for the rest of this doc:

```bash
export BASE=http://127.0.0.1:8787/api/webchat/v2
export OP="Authorization: Bearer operator-secret-please-change"   # the operator (Owner)
```

The operator bearer authenticates as `Owner` and is the bootstrap admin — it is
layered *over* the REST user directory, so it works before any user exists.

## 1. Create the users (operator → POST /admin/users)

`POST /admin/users` mints a **one-time bearer** in the response `token`. Capture
each one.

```bash
DIRECTOR=$(curl -s -X POST "$BASE/admin/users" -H "$OP" -H 'content-type: application/json' \
  -d '{"user_id":"director@","role":"admin"}'  | tee /dev/stderr | jq -r .token)

BOB=$(curl -s -X POST "$BASE/admin/users" -H "$OP" -H 'content-type: application/json' \
  -d '{"user_id":"bob","role":"member"}'       | jq -r .token)

CAROL=$(curl -s -X POST "$BASE/admin/users" -H "$OP" -H 'content-type: application/json' \
  -d '{"user_id":"carol","role":"member"}'     | jq -r .token)

ENG=$(curl -s -X POST "$BASE/admin/users" -H "$OP" -H 'content-type: application/json' \
  -d '{"user_id":"engineering@","role":"member"}' | jq -r .token)

# director@ is an Admin, so from here on you can grant as director@ instead of operator:
export DIR="Authorization: Bearer $DIRECTOR"

curl -s "$BASE/admin/users" -H "$DIR" | jq            # list users (admin-gated)
```

`role` wire values: `owner` | `admin` | `member`. A non-admin caller gets `403`;
a token for the wrong tenant gets `404`.

## 2. Grant the worked example (§12) — per-user policy across four dimensions

`PUT /admin/users/{user_id}/capabilities/{capability_id}` writes ONE delta at
`PolicyScope::User` carrying any of the four optional dimensions. Capability ids
are dot-segmented (`<extension>.<capability>`); use the ids from your installed
catalog — the ones below are the §12 illustration.

Wire enums: `availability` = `available|hidden`; `identity` =
`none|user_keyed|admin_keyed`; `approval` = `allow|ask|deny`; `config_patch` =
any JSON object (deep-merged into the invocation, admin keys win).

**Availability — `nearai.web_search` available for Bob, hidden for Carol.**
The default is Available, so Bob needs nothing; hide it for Carol:

```bash
curl -s -X PUT "$BASE/admin/users/carol/capabilities/nearai.web_search" \
  -H "$DIR" -H 'content-type: application/json' \
  -d '{"availability":"hidden"}' | jq
```

**Identity — `slack` admin-keyed + config `{workspace: acme}`** (the shared key
is the company's, not the user's), and **`gmail.send_message` user-keyed** (each
user brings their own):

```bash
curl -s -X PUT "$BASE/admin/users/engineering@/capabilities/slack.post_message" \
  -H "$DIR" -H 'content-type: application/json' \
  -d '{"identity":"admin_keyed","config_patch":{"workspace":"acme"}}' | jq

curl -s -X PUT "$BASE/admin/users/bob/capabilities/gmail.send_message" \
  -H "$DIR" -H 'content-type: application/json' \
  -d '{"identity":"user_keyed"}' | jq
```

**Approval — admin auto-approves a tool for engineering@, hard-denies another:**

```bash
curl -s -X PUT "$BASE/admin/users/engineering@/capabilities/nearai.web_search" \
  -H "$DIR" -H 'content-type: application/json' -d '{"approval":"allow"}' | jq

curl -s -X PUT "$BASE/admin/users/carol/capabilities/shell.run" \
  -H "$DIR" -H 'content-type: application/json' \
  -d '{"availability":"hidden","approval":"deny"}' | jq
```

Read back what an admin has granted a user:

```bash
curl -s "$BASE/admin/users/bob/capabilities"   -H "$DIR" | jq
curl -s "$BASE/admin/users/carol/capabilities" -H "$DIR" | jq
```

Revoke (idempotent — a second DELETE is still `204`):

```bash
curl -s -o /dev/null -w '%{http_code}\n' -X DELETE \
  "$BASE/admin/users/carol/capabilities/nearai.web_search" -H "$DIR"
```

## 3. Observe enforcement at dispatch

Drive a turn **as each user** (their token in `Authorization: Bearer`) and watch
the policy bite. Enforcement happens live, once per turn, per the four seams:

| Dimension | What to do | Expected with the grants above |
|---|---|---|
| Availability | Ask Carol's agent to use `nearai.web_search` | the tool is **not in Carol's surface** (hidden); Bob's agent can use it |
| Configuration | Run `slack.post_message` as engineering@ | the invocation payload carries `{"workspace":"acme"}` merged in (admin keys win) |
| Identity | Run `gmail.send_message` as Bob with no Gmail key | **auth gate** (UserKeyed missing → re-auth prompt). For `slack` admin-keyed with no shared key provisioned → **unavailable** (see the known gap below) |
| Approval | Run `nearai.web_search` as engineering@ | **auto-approved** (admin Allow). A forced-approval (Financial) effect is still gated — admin Allow never bypasses the safety floor. `shell.run` for Carol is **denied** |

`director@` is an Admin; granting can be done as `$OP` (operator/Owner) or
`$DIR` (director@/Admin) interchangeably — both pass `is_admin()`.

## 4. Restart → durability

The deltas are written to the durable libSQL store under
`/tenants/capability_policy/policy_deltas` (the same store dispatch reads).
Stop `serve` (Ctrl-C), start it again with the **same `IRONCLAW_REBORN_HOME`**,
and re-read:

```bash
curl -s "$BASE/admin/users/carol/capabilities" -H "$DIR" | jq   # Carol's web_search hidden delta is still there
```

Re-run the step-3 observations: the grants enforce identically after restart.
Users themselves also persist (a token minted before the restart still
authenticates), because the user directory is durable on the same mount.

## Known gap (epic-flagged, not a milestone blocker)

**Admin-keyed credential provisioning.** The identity *filter* is fully enforced
(an `admin_keyed` capability with no `SharedAdminManaged` credential resolves to
*unavailable*; a `user_keyed` one with no key resolves to an *auth gate*). But
the two durable credential-creation flows still tag new credentials
`UserReusable` (`TODO(#5261 identity-provisioning)`), so there is no REST path
yet to *provision* the company's shared `slack` key as `SharedAdminManaged`.
Until that lands (the epic flagged it as a possible separate small PR), the
admin-keyed half demonstrates the "unavailable when unprovisioned" behaviour but
not the "works once the shared key exists" behaviour. The other three dimensions
are end-to-end.

## What is NOT in this milestone (per epic #5261)

Configuration-as-Code / declarative policy (#3036, #4120), live propagation to
running agents without restart (#3490, #5242), production SSO/OIDC role read, an
admin/role/grant UI, and Slack channel/room addressability are all out of scope
and revisited later. Grants here are imperative REST PUTs — the backend the UI
will later drive.
