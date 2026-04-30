# /// script
# dependencies = ["requests", "openai"]
# ///
"""Reproduce: the full flow of abound_send_wire sends notify_thread_id, causing the Abound mobile client to never receive it. Because
of an issue on IronClaw end (the tool is never called or/and the success text returned to the chat)
the approval notification.

Drives through the full SKILL.md workflow automatically:
  auto-picks the first option from every choice_set until action="initiate" fires,
  then sends "Send now." to trigger action="send".

After action="send" actually checks the tool was called what were the parameters used for execution, they compared against account info and params for "Inititaete" action

Required env vars:
    BASE_URL            IronClaw deployment URL (e.g. http://localhost:3000)
    ADMIN_TOKEN         Admin bearer token
    ABOUND_READ_TOKEN   Abound read bearer token
    ABOUND_WRITE_TOKEN  Abound write bearer token
    ABOUND_API_KEY      Abound X-API-KEY header value

Optional:
    MASSIVE_API_KEY     Enables exchange rate analysis (forex timing)
    N                   Number of parallel pipeline runs per scenario (default: 2)
    RUNS_PER_THREAD     Sequential sub-runs per thread (default: 5)
    SCENARIO            Which pipeline to run: choice_set | specific | all (default: specific)

Usage:
    BASE_URL=http://localhost:3000 ADMIN_TOKEN=... ABOUND_READ_TOKEN=... \\
    ABOUND_WRITE_TOKEN=... ABOUND_API_KEY=... N=2 SCENARIO=all \\
        uv run integrations/abound/tests/test_send_wire_notify_id.py

Verbose logs are written to per-thread files in the current directory.
Only the final stats table is printed to the terminal.
"""

import atexit
import os
import time
import uuid
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime

import requests
from openai import OpenAI

from pipeline_choice_set import run_pipeline
from pipeline_specific import run_pipeline_specific
from pipeline_replay import run_pipeline_replay
from pipeline_replay_guided import run_pipeline_replay_guided

BASE_URL = os.environ["BASE_URL"].rstrip("/")
ADMIN_TOKEN = os.environ["ADMIN_TOKEN"]
ABOUND_READ_TOKEN = os.environ["ABOUND_READ_TOKEN"]
ABOUND_WRITE_TOKEN = os.environ["ABOUND_WRITE_TOKEN"]
ABOUND_API_KEY = os.environ["ABOUND_API_KEY"]
MASSIVE_API_KEY = os.environ.get("MASSIVE_API_KEY", "")
N = int(os.environ.get("N", "3"))
RUNS_PER_THREAD = int(os.environ.get("RUNS_PER_THREAD", "4"))
SCENARIO = os.environ.get("SCENARIO", "replay_guided").lower()
_temp_env = os.environ.get("TEMPERATURE")
TEMPERATURE = None if os.environ.get("NO_TEMPERATURE") else (float(_temp_env) if _temp_env else 1.5)

if SCENARIO not in ("choice_set", "specific", "replay", "replay_guided", "all"):
    print(f"FATAL: SCENARIO must be choice_set, specific, replay, replay_guided, or all — got {SCENARIO!r}")
    raise SystemExit(1)

admin = requests.Session()
admin.headers.update({"Authorization": f"Bearer {ADMIN_TOKEN}", "Content-Type": "application/json"})

user_id = ""
run_ts = datetime.now().strftime("%Y%m%d_%H%M%S")


def cleanup():
    if user_id:
        admin.delete(f"{BASE_URL}/api/admin/users/{user_id}")
        print(f"  Deleted test user {user_id}")


atexit.register(cleanup)

# ---------------------------------------------------------------------------
# Setup
# ---------------------------------------------------------------------------
print("=== send_wire notify_thread_id regression test ===")
print(f"Target: {BASE_URL}  N={N}  RUNS_PER_THREAD={RUNS_PER_THREAD}  SCENARIO={SCENARIO}")
print(f"Logs: {run_ts}_<label>.log\n")

r = admin.post(f"{BASE_URL}/api/admin/users", json={
    "display_name": "Wire Notify Test",
    "email": f"wire-notify-{uuid.uuid4().hex[:8]}@example.com",
    "role": "member",
})
if r.status_code != 200:
    print(f"FATAL: create user {r.status_code} {r.text}")
    raise SystemExit(1)

data = r.json()
user_id = data["id"]
user_token = data["token"]
print(f"Created user {user_id}")

for name, value, provider in [
    ("abound_read_token", ABOUND_READ_TOKEN, "abound"),
    ("abound_write_token", ABOUND_WRITE_TOKEN, "abound"),
    ("abound_api_key", ABOUND_API_KEY, "abound"),
    *([("massive_api_key", MASSIVE_API_KEY, "massive")] if MASSIVE_API_KEY else []),
]:
    r = admin.put(f"{BASE_URL}/api/admin/users/{user_id}/secrets/{name}",
                  json={"value": value, "provider": provider})
    if r.status_code != 200:
        print(f"FATAL: inject {name}: {r.status_code} {r.text[:200]}")
        raise SystemExit(1)

print("Secrets injected. Waiting 5s for workspace bootstrap...")
time.sleep(5)

client = OpenAI(api_key=user_token, base_url=f"{BASE_URL}/v1")


def write_log(label: str, lines: list[str]) -> str:
    path = f"{run_ts}_{label}.log"
    with open(path, "w") as f:
        f.write("\n".join(lines))
    return path


# ---------------------------------------------------------------------------
# Run pipelines in parallel
# ---------------------------------------------------------------------------
run_choice_set = SCENARIO in ("choice_set", "all")
run_specific = SCENARIO in ("specific", "all")
run_replay = SCENARIO in ("replay", "all")
run_replay_guided = SCENARIO in ("replay_guided", "all")
max_workers = (N if run_choice_set else 0) + (N if run_specific else 0) + (N if run_replay else 0) + (N if run_replay_guided else 0)

all_results: list[tuple[str, list[tuple[str, bool, str]]]] = []

print(f"\nRunning {max_workers} parallel thread(s)...\n")

with ThreadPoolExecutor(max_workers=max(max_workers, 1)) as executor:
    futures: dict = {}
    if run_choice_set:
        futures.update({
            executor.submit(run_pipeline, i + 1, client, TEMPERATURE, RUNS_PER_THREAD): f"run-{i + 1}"
            for i in range(N)
        })
    if run_specific:
        futures.update({
            executor.submit(run_pipeline_specific, i + 1, client, ABOUND_READ_TOKEN, ABOUND_API_KEY, TEMPERATURE, RUNS_PER_THREAD): f"specific-{i + 1}"
            for i in range(N)
        })
    if run_replay:
        futures.update({
            executor.submit(run_pipeline_replay, i + 1, client, ABOUND_READ_TOKEN, ABOUND_API_KEY, TEMPERATURE, RUNS_PER_THREAD): f"replay-{i + 1}"
            for i in range(N)
        })
    if run_replay_guided:
        futures.update({
            executor.submit(run_pipeline_replay_guided, i + 1, client, ABOUND_READ_TOKEN, ABOUND_API_KEY, TEMPERATURE, RUNS_PER_THREAD): f"replay_guided-{i + 1}"
            for i in range(N)
        })

    for future in as_completed(futures):
        label = futures[future]
        try:
            checks, log_lines = future.result()
        except Exception as exc:
            checks = [("completed", False, str(exc))]
            log_lines = [f"FATAL: {exc}"]
        log_path = write_log(label, log_lines)
        all_results.append((label, checks))
        passed = sum(1 for _, ok, _ in checks if ok)
        failed = sum(1 for _, ok, _ in checks if not ok)
        print(f"  [{label}] done — {passed} passed, {failed} failed  →  {log_path}")

# ---------------------------------------------------------------------------
# Stats table
# ---------------------------------------------------------------------------
print()
print("=== Results ===")
print(f"{'Thread':<18} {'Pass':>5} {'Fail':>5}  Failures")
print("-" * 70)

total_passed = 0
total_failed = 0

all_results.sort(key=lambda x: x[0])
for label, checks in all_results:
    passed = sum(1 for _, ok, _ in checks if ok)
    failed = sum(1 for _, ok, _ in checks if not ok)
    total_passed += passed
    total_failed += failed
    failures = [name for name, ok, _ in checks if not ok]
    failure_str = ", ".join(failures[:3]) + ("…" if len(failures) > 3 else "")
    print(f"  {label:<16} {passed:>5} {failed:>5}  {failure_str}")

print("-" * 70)
print(f"  {'TOTAL':<16} {total_passed:>5} {total_failed:>5}")
print()

raise SystemExit(0 if total_failed == 0 else 1)
