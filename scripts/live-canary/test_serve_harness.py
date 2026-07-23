#!/usr/bin/env python3
"""Unit tests for the shared ``ironclaw serve`` live-canary launcher."""

from __future__ import annotations

import asyncio
import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch


ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

_SPEC = importlib.util.spec_from_file_location(
    "live_canary_serve_harness",
    ROOT / "scripts" / "live_canary" / "serve_harness.py",
)
serve_harness = importlib.util.module_from_spec(_SPEC)
sys.modules[_SPEC.name] = serve_harness
_SPEC.loader.exec_module(serve_harness)


class ServeHarnessTests(unittest.TestCase):
    def test_command_uses_shipping_serve_cli_on_loopback(self):
        self.assertEqual(
            serve_harness.serve_command(Path("/tmp/ironclaw"), 43121),
            [
                "/tmp/ironclaw",
                "serve",
                "--host",
                "127.0.0.1",
                "--port",
                "43121",
            ],
        )

    def test_launcher_captures_logs_and_waits_for_health(self):
        proc = Mock(spec=subprocess.Popen)
        proc.poll.return_value = None

        async def run_test() -> tuple[object, str, list[str], str, bool, bool]:
            with tempfile.TemporaryDirectory() as tmpdir:
                output_dir = Path(tmpdir)
                with (
                    patch.object(
                        serve_harness.subprocess,
                        "Popen",
                        return_value=proc,
                    ) as popen,
                    patch.object(
                        serve_harness,
                        "wait_for_ready",
                    ) as wait_for_ready,
                ):
                    launched_proc, base_url = await serve_harness.start_serve(
                        binary=Path("/opt/ironclaw"),
                        port=43122,
                        env={"PATH": "/usr/bin:/bin"},
                        output_dir=output_dir,
                    )
                    command = popen.call_args.args[0]
                    readiness_url = wait_for_ready.call_args.args[0]
                    stdout_exists = (output_dir / "ironclaw-serve.stdout.log").exists()
                    stderr_exists = (output_dir / "ironclaw-serve.stderr.log").exists()
                return (
                    launched_proc,
                    base_url,
                    command,
                    readiness_url,
                    stdout_exists,
                    stderr_exists,
                )

        (
            launched_proc,
            base_url,
            command,
            readiness_url,
            stdout_exists,
            stderr_exists,
        ) = asyncio.run(run_test())
        self.assertIs(launched_proc, proc)
        self.assertEqual(base_url, "http://127.0.0.1:43122")
        self.assertIn("serve", command)
        self.assertEqual(readiness_url, "http://127.0.0.1:43122/api/health")
        self.assertTrue(stdout_exists)
        self.assertTrue(stderr_exists)


if __name__ == "__main__":
    unittest.main()
