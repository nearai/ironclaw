from __future__ import annotations

import hashlib
import importlib.util
import io
import os
import sys
import tarfile
import tempfile
import textwrap
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/ci/release-upgrade-canary.py"
SPEC = importlib.util.spec_from_file_location("release_upgrade_canary", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
CANARY = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = CANARY
SPEC.loader.exec_module(CANARY)


FAKE_BINARY = r'''#!/usr/bin/env python3
import argparse
import json
import os
import signal
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, unquote, urlparse

VERSION = "__VERSION__"
DROP_STATE = __DROP_STATE__
DROP_ROUTINES = __DROP_ROUTINES__

if sys.argv[1:] == ["--version"]:
    print(f"ironclaw {VERSION}")
    raise SystemExit(0)

parser = argparse.ArgumentParser()
subparsers = parser.add_subparsers(dest="command", required=True)
serve = subparsers.add_parser("serve")
serve.add_argument("--host", required=True)
serve.add_argument("--port", type=int, required=True)
args = parser.parse_args()

home = Path(os.environ["IRONCLAW_REBORN_HOME"])
home.mkdir(parents=True, exist_ok=True)
state_path = home / "fake-release-state.json"
if DROP_STATE:
    state_path.unlink(missing_ok=True)


def load_state():
    if not state_path.exists():
        return {"threads": [], "timelines": {}, "automations": []}
    return json.loads(state_path.read_text(encoding="utf-8"))


def save_state(state):
    state_path.write_text(json.dumps(state), encoding="utf-8")


if DROP_ROUTINES and state_path.exists():
    state = load_state()
    state["automations"] = []
    save_state(state)


def send_json(handler, payload, status=200):
    body = json.dumps(payload).encode("utf-8")
    handler.send_response(status)
    handler.send_header("Content-Type", "application/json")
    handler.send_header("Content-Length", str(len(body)))
    handler.end_headers()
    handler.wfile.write(body)


class Handler(BaseHTTPRequestHandler):
    def log_message(self, _format, *_args):
        return

    def do_GET(self):
        parsed = urlparse(self.path)
        if parsed.path == "/api/health":
            send_json(self, {"status": "ok"})
            return
        state = load_state()
        if parsed.path == "/api/webchat/v2/threads":
            send_json(self, {"threads": state["threads"]})
            return
        if parsed.path == "/api/webchat/v2/automations":
            send_json(self, {"automations": state.get("automations", [])})
            return
        prefix = "/api/webchat/v2/threads/"
        suffix = "/timeline"
        if parsed.path.startswith(prefix) and parsed.path.endswith(suffix):
            thread_id = unquote(parsed.path[len(prefix):-len(suffix)])
            send_json(
                self,
                {"messages": state["timelines"].get(thread_id, [])},
            )
            return
        if parsed.path == "/api/webchat/v2/fs/content":
            query = parse_qs(parsed.query)
            requested = query.get("path", [""])[0]
            snapshot = Path(os.environ["IRONCLAW_REBORN_LEGACY_WORKSPACE_SNAPSHOT"])
            body = (snapshot / requested).read_bytes()
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        self.send_error(404)

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        payload = json.loads(self.rfile.read(length) or b"{}")
        state = load_state()
        if self.path == "/api/webchat/v2/threads":
            thread_id = f"thread-{len(state['threads']) + 1}"
            thread = {"thread_id": thread_id}
            state["threads"].append(thread)
            state["timelines"][thread_id] = []
            save_state(state)
            send_json(self, {"thread": thread})
            return
        prefix = "/api/webchat/v2/threads/"
        suffix = "/messages"
        if self.path.startswith(prefix) and self.path.endswith(suffix):
            thread_id = unquote(self.path[len(prefix):-len(suffix)])
            if "scheduled routine" in payload["content"]:
                name = "release-upgrade-canary-scheduled"
                expression = "0 0 1 1 *"
            elif "paused routine" in payload["content"]:
                name = "release-upgrade-canary-paused"
                expression = "0 0 2 1 *"
            else:
                name = None
                expression = None
            if name is not None:
                state.setdefault("automations", []).append(
                    {
                        "automation_id": f"automation-{len(state['automations']) + 1}",
                        "name": name,
                        "source": {
                            "type": "schedule",
                            "cron": expression,
                            "timezone": "UTC",
                        },
                        "state": "scheduled",
                        "next_run_at": "2999-01-01T00:00:00Z",
                        "created_at": "2026-08-05T00:00:00Z",
                    }
                )
            state["timelines"][thread_id] = [
                {
                    "message_id": f"{thread_id}-user",
                    "kind": "user",
                    "content": payload["content"],
                    "sequence": 1,
                    "status": "accepted",
                    "turn_run_id": f"{thread_id}-run",
                },
                {
                    "message_id": f"{thread_id}-assistant",
                    "kind": "assistant",
                    "content": "release upgrade canary deterministic reply",
                    "sequence": 2,
                    "status": "finalized",
                    "turn_run_id": f"{thread_id}-run",
                },
            ]
            save_state(state)
            send_json(self, {"outcome": "submitted"}, status=202)
            return
        automation_prefix = "/api/webchat/v2/automations/"
        pause_suffix = "/pause"
        if self.path.startswith(automation_prefix) and self.path.endswith(pause_suffix):
            automation_id = unquote(
                self.path[len(automation_prefix):-len(pause_suffix)]
            )
            for automation in state.get("automations", []):
                if automation["automation_id"] == automation_id:
                    automation["state"] = "paused"
                    save_state(state)
                    send_json(self, {"automation": automation})
                    return
            send_json(self, {"error": "not found"}, status=404)
            return
        self.send_error(404)


server = ThreadingHTTPServer((args.host, args.port), Handler)
signal.signal(signal.SIGINT, lambda *_args: sys.exit(0))
server.serve_forever()
'''


class ReleaseUpgradeCanaryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def _archive(
        self,
        name: str,
        version: str,
        *,
        drop_state: bool = False,
        drop_routines: bool = False,
        duplicate: bool = False,
    ) -> tuple[Path, Path]:
        archive = self.root / f"{name}.tar.gz"
        executable = textwrap.dedent(FAKE_BINARY).replace("__VERSION__", version)
        executable = executable.replace(
            "__DROP_STATE__", "True" if drop_state else "False"
        )
        executable = executable.replace(
            "__DROP_ROUTINES__", "True" if drop_routines else "False"
        ).encode("utf-8")
        with tarfile.open(archive, "w:gz") as package:
            paths = ("package/ironclaw", "duplicate/ironclaw") if duplicate else ("package/ironclaw",)
            for path in paths:
                member = tarfile.TarInfo(path)
                member.mode = 0o755
                member.size = len(executable)
                package.addfile(member, io.BytesIO(executable))
        checksum = archive.with_suffix(archive.suffix + ".sha256")
        checksum.write_text(
            f"{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n",
            encoding="utf-8",
        )
        return archive, checksum

    def _run(
        self,
        *,
        drop_candidate_state: bool = False,
        drop_candidate_routines: bool = False,
    ) -> set[str]:
        previous, previous_checksum = self._archive("previous", "1.0.0-rc.1")
        candidate, candidate_checksum = self._archive(
            "candidate",
            "1.1.0-rc.1",
            drop_state=drop_candidate_state,
            drop_routines=drop_candidate_routines,
        )
        return CANARY.run_upgrade_canary(
            previous_archive=previous,
            previous_checksum=previous_checksum,
            previous_version="1.0.0-rc.1",
            candidate_archive=candidate,
            candidate_checksum=candidate_checksum,
            candidate_version="1.1.0-rc.1",
            binary_name="ironclaw",
            artifact_dir=self.root / "artifacts",
        )

    @unittest.skipIf(os.name == "nt", "the fake release binary uses POSIX signals")
    def test_exact_artifacts_cover_upgrade_restart_rollback_and_reupgrade(self) -> None:
        evidence = self._run()

        self.assertEqual(
            evidence,
            {
                "checksums",
                "artifact_versions",
                "previous_release_state",
                "routine_state",
                "upgrade",
                "restart_idempotence",
                "rollback",
                "reupgrade",
            },
        )
        self.assertIn(
            '"status": "passed"',
            (self.root / "artifacts/result.json").read_text(encoding="utf-8"),
        )

    @unittest.skipIf(os.name == "nt", "the fake release binary uses POSIX signals")
    def test_candidate_state_loss_fails_the_observable_upgrade_gate(self) -> None:
        with self.assertRaisesRegex(CANARY.CanaryFailure, "first candidate boot"):
            self._run(drop_candidate_state=True)

        self.assertIn(
            '"status": "failed"',
            (self.root / "artifacts/result.json").read_text(encoding="utf-8"),
        )

    @unittest.skipIf(os.name == "nt", "the fake release binary uses POSIX signals")
    def test_candidate_routine_loss_fails_the_observable_upgrade_gate(self) -> None:
        with self.assertRaisesRegex(
            CANARY.CanaryFailure, "omitted the seeded release routines"
        ):
            self._run(drop_candidate_routines=True)

        self.assertIn(
            '"status": "failed"',
            (self.root / "artifacts/result.json").read_text(encoding="utf-8"),
        )

    def test_checksum_mismatch_is_rejected_before_execution(self) -> None:
        archive, checksum = self._archive("previous", "1.0.0-rc.1")
        checksum.write_text(f"{'0' * 64}  {archive.name}\n", encoding="utf-8")

        with self.assertRaisesRegex(CANARY.CanaryFailure, "SHA-256 mismatch"):
            CANARY.verify_checksum(archive, checksum)

    def test_duplicate_shipping_binary_is_rejected(self) -> None:
        archive, _ = self._archive("duplicate", "1.0.0-rc.1", duplicate=True)

        with self.assertRaisesRegex(CANARY.CanaryFailure, "found 2"):
            CANARY.extract_binary(archive, self.root / "extracted", "ironclaw")


if __name__ == "__main__":
    unittest.main()
