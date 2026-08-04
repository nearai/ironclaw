#!/usr/bin/env bash
# End-to-end: can an agent create a skill and can a LATER conversation use it?
#
# This is the loop #6941 item 4 (originally #7168) is about, and no hermetic test can cover it: it
# needs a live server, a real model, a restart, and two separate conversations. Every hermetic guard
# around it passed while the product was broken, because each one tested a layer in isolation:
#
#   * the mount views agreed in the shape the test constructed
#   * the install reported success
#   * discovery listed the tree the test had seeded
#
# The failure only appears when the writer and the reader are the real ones, chosen by the real
# composition, with a restart in between so nothing in memory can carry the result.
#
#   Phase A  a skill the agent authors from prose
#     A1  prompt 1 asks it to derive something and save a reusable skill
#     A2  the skill is listed in Settings -> Skills          (was: absent)
#     A3  the skill's SKILL.md is in the DATABASE, not on disk
#     A4  restart the server
#     A5  the skill is still listed                          (was: absent)
#     A6  a NEW conversation activates it                    (was: unactivatable, forever)
#
#   Phase B  a skill that carries a runnable script (the #6745 feature)
#     B1  prompt 1 asks for a skill including a Python script
#     B2  the bundle holds SKILL.md AND scripts/*.py in the database
#     B3  Settings reports has_scripts                       (was: hardcoded false)
#     B4  a NEW conversation activates it
#     B5  the script is EXECUTED from the bundle             <-- KNOWN GAP, see below
#
# B5 is expected to fail today and is reported, not asserted, unless
# E2E_REQUIRE_SCRIPT_EXEC=1. `builtin.shell` spawns a host process and the script exists only in the
# DB-backed virtual filesystem, so there is no host path to run. Observed: the agent tries
# `ls -la skills/<name>/<script>.py … || echo "NOT FOUND"` and then falls back to
# `python3 -c "<the algorithm re-typed inline>"`. Right answers, wrong mechanism -- and it defeats the
# argument for shipping a script, which is that prose lets the next run re-derive and drift. Set the
# env var once a fix lands, and this becomes a real assertion.
#
# Usage:
#   OPENROUTER_API_KEY=... scripts/e2e-skill-self-creation.sh
#   E2E_PORT=3200 E2E_MODEL=deepseek/deepseek-v4-flash scripts/e2e-skill-self-creation.sh
#
# Runs on its own port and its own IRONCLAW_REBORN_HOME, so it never disturbs a running dev server.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

PORT="${E2E_PORT:-3100}"
MODEL="${E2E_MODEL:-deepseek/deepseek-v4-flash}"
HOME_DIR="${E2E_HOME:-$HOME/.ironclaw-reborn-e2e-skills}"
TOKEN="${E2E_TOKEN:-e2eskillstokene2eskillstokene2eskills}"
BIN="${E2E_BIN:-$REPO_ROOT/target/debug/ironclaw}"
API="http://127.0.0.1:${PORT}/api/webchat/v2"
WORK="$(mktemp -d)"
LOG_DIR="${E2E_LOG_DIR:-$WORK}"
# Must exist before the first redirect: `nohup … > "$LOG_DIR/server-1.log"` fails silently on a
# missing directory, so the server never launches and the log that would say why is never created.
mkdir -p "$LOG_DIR"

PASS=0
FAIL=0
GAP=0

pass() { PASS=$((PASS + 1)); echo "  PASS  $1"; }
fail() { FAIL=$((FAIL + 1)); echo "  FAIL  $1"; }
gap()  { GAP=$((GAP + 1));  echo "  GAP   $1"; }
step() { echo; echo "== $1"; }

cleanup() {
  pkill -f "ironclaw serve --port ${PORT}" 2>/dev/null
  rm -rf "$WORK"
}
trap cleanup EXIT

if [ ! -x "$BIN" ]; then
  echo "no ironclaw binary at $BIN -- build it first: cargo build -p ironclaw --bin ironclaw"
  exit 2
fi
if [ -z "${OPENROUTER_API_KEY:-}" ] && [ -z "${E2E_SKIP_PROVIDER_CHECK:-}" ]; then
  echo "OPENROUTER_API_KEY is unset; this test drives a real model. Set it, or set"
  echo "E2E_SKIP_PROVIDER_CHECK=1 if the configured provider needs no key."
  exit 2
fi

# The request body is built by a python FILE on purpose. Inline `python3 -c` loses the dict literal's
# braces and colons to shell quoting, which silently produces a malformed request.
cat > "$WORK/body.py" <<'PY'
import json, sys
json.dump({"client_action_id": sys.argv[1], "content": sys.argv[2]}, open(sys.argv[3], "w"))
PY

cat > "$WORK/skills.py" <<'PY'
import json, sys
d = json.load(sys.stdin)
want = sys.argv[1] if len(sys.argv) > 1 else ""
for s in d.get("skills") or []:
    if s.get("source") == "system" and not want:
        continue
    if want and s.get("name") != want:
        continue
    print("%s\t%s\t%s" % (s.get("name"), s.get("source"), s.get("has_scripts")))
PY

cat > "$WORK/timeline.py" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
blob = json.dumps(d)
shell_cmds = []
for m in d.get("messages") or []:
    if m.get("kind") != "capability_display_preview":
        continue
    try:
        p = json.loads(m.get("content") or "{}")
    except ValueError:
        continue
    if "shell" in (p.get("capability_id") or ""):
        shell_cmds.append(p.get("subtitle") or "")
answer = ""
for m in d.get("messages") or []:
    if m.get("kind") == "assistant":
        answer = (m.get("content") or "").replace("\n", " ")[:200]
print(json.dumps({
    "activated": "skill_activate" in blob,
    "names_skill": sys.argv[2] in blob,
    "shell_cmds": shell_cmds,
    "answer": answer,
}))
PY

start_server() {
  rm -f "$LOG_DIR/server-$1.log"
  IRONCLAW_REBORN_HOME="$HOME_DIR" \
  IRONCLAW_REBORN_WEBUI_TOKEN="$TOKEN" \
  IRONCLAW_REBORN_WEBUI_USER_ID=reborn-cli \
  RUST_MIN_STACK=8388608 \
    nohup "$BIN" serve --port "$PORT" > "$LOG_DIR/server-$1.log" 2>&1 &
  # A debug build seeds bundled skills and runs migrations before it binds; be generous.
  for _ in $(seq 1 60); do
    sleep 10
    if [ "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:${PORT}/")" = "200" ]; then
      echo "  server up ($1)"
      return 0
    fi
  done
  echo "  server failed to start ($1); see $LOG_DIR/server-$1.log"
  return 1
}

new_thread() {
  curl -s -X POST -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
    -d "{\"client_action_id\":\"$1\"}" "$API/threads" \
    | python3 -c "import json,sys; print(json.load(sys.stdin)['thread']['thread_id'])"
}

send() {
  python3 "$WORK/body.py" "$1" "$2" "$WORK/req.json"
  curl -s -X POST -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
    --data-binary @"$WORK/req.json" "$API/threads/$3/messages" > /dev/null
}

user_skills() {
  curl -s -H "Authorization: Bearer $TOKEN" "$API/skills" | python3 "$WORK/skills.py"
}

# Wait for a named skill to appear, or give up.
await_skill() {
  local name="$1"
  for _ in $(seq 1 40); do
    sleep 15
    if user_skills | grep -q "^${name}	"; then return 0; fi
  done
  return 1
}

await_answer() {
  local thread="$1"
  for _ in $(seq 1 40); do
    sleep 15
    if curl -s -H "Authorization: Bearer $TOKEN" "$API/threads/$thread/timeline" \
        | grep -q '"kind": *"assistant"'; then
      return 0
    fi
  done
  return 1
}

db_paths() {
  # Local-dev keeps the virtual filesystem in libSQL under the profile subdir.
  python3 - "$HOME_DIR" "$1" <<'PY'
import glob, os, sqlite3, sys
home, needle = sys.argv[1], sys.argv[2]
found = []
for db in glob.glob(os.path.join(home, "**", "*.db"), recursive=True):
    try:
        con = sqlite3.connect("file:%s?mode=ro" % db, uri=True)
        rows = con.execute(
            "select path from root_filesystem_entries where path like ? order by path",
            ("%%%s%%" % needle,),
        ).fetchall()
        found.extend(r[0] for r in rows)
    except Exception:
        continue
print("\n".join(found))
PY
}

echo "e2e skill self-creation"
echo "  port   : $PORT"
echo "  model  : $MODEL"
echo "  home   : $HOME_DIR (recreated)"
echo "  logs   : $LOG_DIR"

pkill -f "ironclaw serve --port ${PORT}" 2>/dev/null
sleep 2
rm -rf "$HOME_DIR"
IRONCLAW_REBORN_HOME="$HOME_DIR" "$BIN" models set-provider openrouter --model "$MODEL" >/dev/null 2>&1

start_server 1 || exit 1

# Bundled skills must be present from the start: production shipped zero of them, and an empty
# system root also makes every later count meaningless.
BUNDLED=$(curl -s -H "Authorization: Bearer $TOKEN" "$API/skills" \
  | python3 -c "import json,sys; print(len([x for x in (json.load(sys.stdin).get('skills') or []) if x.get('source')=='system']))")
step "bundled skills are seeded"
if [ "${BUNDLED:-0}" -gt 0 ]; then pass "$BUNDLED built-in skills listed"; else fail "no built-in skills listed"; fi

# ---------------------------------------------------------------- Phase A: prose skill
PROSE_SKILL=lab-unit-si-conversion
step "Phase A1: the agent authors a skill"
TA1=$(new_thread e2e-a1)
send e2e-a1-msg "Work out the exact factors for converting clinical lab values from US conventional units to SI units for sodium (mEq/L to mmol/L), glucose (mg/dL to mmol/L), creatinine (mg/dL to umol/L) and total cholesterol (mg/dL to mmol/L), and the correct order of operations for rounding. Then save it as a reusable skill named ${PROSE_SKILL} so you never have to re-derive it." "$TA1"

if await_skill "$PROSE_SKILL"; then
  pass "A2 the authored skill is listed in Settings -> Skills"
else
  fail "A2 the authored skill never appeared (this is the reported bug)"
  echo "  skills seen: $(user_skills | tr '\n' ' ')"
  echo; echo "results: $PASS passed, $FAIL failed, $GAP known gaps"; exit 1
fi

step "Phase A3: it is stored in the database, not on the host disk"
if db_paths "$PROSE_SKILL" | grep -q "/skills/${PROSE_SKILL}/SKILL.md"; then
  pass "A3 SKILL.md is in the DB-backed virtual filesystem"
else
  fail "A3 SKILL.md is not in the database"
fi
if find "$HOME_DIR" -path "*/skills/${PROSE_SKILL}/SKILL.md" 2>/dev/null | grep -q .; then
  fail "A3 a copy was left on the host disk, where discovery does not read"
else
  pass "A3 nothing was written to the host skill tree"
fi

step "Phase A4: restart, so nothing in memory can carry the result"
pkill -f "ironclaw serve --port ${PORT}"; sleep 5
start_server 2 || exit 1
if user_skills | grep -q "^${PROSE_SKILL}	"; then
  pass "A5 the skill survives a restart"
else
  fail "A5 the skill is gone after a restart"
fi

step "Phase A6: a NEW conversation activates it"
TA2=$(new_thread e2e-a2)
send e2e-a2-msg "A colleague sent me lab results in US conventional units: sodium 140, glucose 105, creatinine 1.1, total cholesterol 190. Give me the SI equivalents and say which conversion factors you applied." "$TA2"
if await_answer "$TA2"; then
  curl -s -H "Authorization: Bearer $TOKEN" "$API/threads/$TA2/timeline" > "$WORK/ta2.json"
  A6=$(python3 "$WORK/timeline.py" "$WORK/ta2.json" "$PROSE_SKILL")
  if echo "$A6" | grep -q '"activated": true'; then
    pass "A6 skill_activate fired in the fresh conversation"
  else
    fail "A6 the fresh conversation never activated the skill"
  fi
else
  fail "A6 the fresh conversation never answered"
fi
# `grep -c` exits 1 when the count is zero, so `|| echo 0` appended a SECOND zero and the compare
# against "0" failed -- reporting a failure for the healthy case. Count without the fallback.
SKIPS=$(grep -ac "skipping skill bundle" "$LOG_DIR/server-2.log" 2>/dev/null)
SKIPS=${SKIPS:-0}
if [ "${SKIPS:-0}" = "0" ]; then
  pass "A6 no bundle was skipped as unvalidatable"
else
  fail "A6 $SKIPS 'skipping skill bundle' warnings -- a manifest discovery refuses"
fi

# ---------------------------------------------------------------- Phase B: scripted skill
SCRIPT_SKILL=ckd-epi-egfr
step "Phase B1: the agent authors a skill carrying a runnable script"
TB1=$(new_thread e2e-b1)
send e2e-b1-msg "I need to compute eGFR from serum creatinine using the 2021 race-free CKD-EPI equation, and assign a CKD stage (G1-G5) from the result. Work out the equation and the stage cutoffs precisely, then save it as a reusable skill named ${SCRIPT_SKILL} that includes a runnable Python script, so a future run executes the script instead of re-deriving the math." "$TB1"

if await_skill "$SCRIPT_SKILL"; then
  pass "B1 the scripted skill is listed"
else
  fail "B1 the scripted skill never appeared"
  echo; echo "results: $PASS passed, $FAIL failed, $GAP known gaps"; exit 1
fi

step "Phase B2: the bundle really carries a script"
BUNDLE=$(db_paths "$SCRIPT_SKILL")
echo "$BUNDLE" | sed 's/^/    /'
if echo "$BUNDLE" | grep -qE "/skills/${SCRIPT_SKILL}/scripts/.*\.py$"; then
  pass "B2 scripts/*.py is in the database alongside SKILL.md"
else
  fail "B2 no script in the bundle -- the agent wrote prose only"
fi

step "Phase B3: Settings reports that it has scripts"
if user_skills "$SCRIPT_SKILL" | grep -q "	True$"; then
  pass "B3 has_scripts is reported to the UI"
else
  fail "B3 has_scripts is false for a bundle that has scripts"
fi

step "Phase B4/B5: a NEW conversation uses it"
TB2=$(new_thread e2e-b2)
send e2e-b2-msg "Three patients: 62-year-old female with creatinine 1.3 mg/dL, 45-year-old male with creatinine 0.9 mg/dL, 78-year-old male with creatinine 2.1 mg/dL. Give me each one's eGFR and CKD stage." "$TB2"
if await_answer "$TB2"; then
  curl -s -H "Authorization: Bearer $TOKEN" "$API/threads/$TB2/timeline" > "$WORK/tb2.json"
  B=$(python3 "$WORK/timeline.py" "$WORK/tb2.json" "$SCRIPT_SKILL")
  if echo "$B" | grep -q '"activated": true'; then
    pass "B4 skill_activate fired for the scripted skill"
  else
    fail "B4 the scripted skill was not activated"
  fi
  echo "  shell commands the agent ran:"
  python3 - "$WORK/tb2.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
for m in d.get("messages") or []:
    if m.get("kind") != "capability_display_preview":
        continue
    try:
        p = json.loads(m.get("content") or "{}")
    except ValueError:
        continue
    if "shell" in (p.get("capability_id") or ""):
        print("    $ " + (p.get("subtitle") or "")[:160].replace("\n", " "))
PY
  # Did it RUN the bundled script, or re-type the algorithm inline?
  #
  # Best-effort: display-preview subtitles REDACT paths, so `python3 <path>` cannot be attributed to
  # the bundle from the preview alone. The inline `python3 -c "<algorithm>"` is the reliable tell, so
  # a false "no" is possible and a false "yes" is not. Worth replacing with a check against the shell
  # capability's recorded input once that is readable from the timeline.
  RAN_FILE=$(python3 - "$WORK/tb2.json" "$SCRIPT_SKILL" <<'PY'
import json, sys
d = json.load(open(sys.argv[1])); skill = sys.argv[2]
ran = False
for m in d.get("messages") or []:
    if m.get("kind") != "capability_display_preview":
        continue
    try:
        p = json.loads(m.get("content") or "{}")
    except ValueError:
        continue
    if "shell" not in (p.get("capability_id") or ""):
        continue
    cmd = p.get("subtitle") or ""
    # Executing the bundled file, not inlining its contents.
    if ".py" in cmd and skill in cmd and "-c" not in cmd and "ls " not in cmd:
        ran = True
print("yes" if ran else "no")
PY
)
  if [ "$RAN_FILE" = "yes" ]; then
    pass "B5 the bundled script was executed from the skill bundle"
  elif [ "${E2E_REQUIRE_SCRIPT_EXEC:-0}" = "1" ]; then
    fail "B5 the bundled script was never executed; the agent inlined the algorithm instead"
  else
    gap "B5 the bundled script was NOT executed -- no host path exists for a script that lives only in the virtual filesystem, so the agent re-typed it inline (#6941 item 4, open)"
  fi
else
  fail "B4 the scripted conversation never answered"
fi

echo
echo "================ results ================"
echo "  passed      : $PASS"
echo "  failed      : $FAIL"
echo "  known gaps  : $GAP"
echo "  logs        : $LOG_DIR"
[ "$FAIL" -eq 0 ] || exit 1
