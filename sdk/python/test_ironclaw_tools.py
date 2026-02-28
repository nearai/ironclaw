"""Tests for the IronClaw Programmatic Tool Calling Python SDK."""

import json
import os
import sys
import unittest
from unittest.mock import patch, MagicMock
import urllib.error

# Ensure ironclaw_tools is importable regardless of working directory.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))


class TestEnvMissing(unittest.TestCase):
    """Test that missing env vars produce clear errors."""

    def setUp(self):
        # Clear all relevant env vars
        for var in ["IRONCLAW_ORCHESTRATOR_URL", "IRONCLAW_JOB_ID", "IRONCLAW_WORKER_TOKEN"]:
            os.environ.pop(var, None)

    def test_env_missing(self):
        from ironclaw_tools import call_tool
        with self.assertRaises(RuntimeError) as ctx:
            call_tool("echo", {"message": "hello"})
        # Should mention the missing variable
        self.assertIn("IRONCLAW_ORCHESTRATOR_URL", str(ctx.exception))


class TestCallToolRequestFormat(unittest.TestCase):
    """Test that call_tool sends correctly formatted requests."""

    def setUp(self):
        os.environ["IRONCLAW_ORCHESTRATOR_URL"] = "http://localhost:50051"
        os.environ["IRONCLAW_JOB_ID"] = "550e8400-e29b-41d4-a716-446655440000"
        os.environ["IRONCLAW_WORKER_TOKEN"] = "test-token-123"

    def tearDown(self):
        for var in ["IRONCLAW_ORCHESTRATOR_URL", "IRONCLAW_JOB_ID", "IRONCLAW_WORKER_TOKEN"]:
            os.environ.pop(var, None)

    @patch("ironclaw_tools.urllib.request.urlopen")
    def test_call_tool_request_format(self, mock_urlopen):
        from ironclaw_tools import call_tool

        # Mock successful response
        mock_response = MagicMock()
        mock_response.read.return_value = json.dumps({
            "success": True,
            "output": "hello",
            "duration_ms": 5,
            "was_sanitized": False,
        }).encode("utf-8")
        mock_response.__enter__ = lambda s: s
        mock_response.__exit__ = MagicMock(return_value=False)
        mock_urlopen.return_value = mock_response

        result = call_tool("echo", {"message": "hello"}, timeout_secs=30)

        # Verify the request was made
        mock_urlopen.assert_called_once()
        call_args = mock_urlopen.call_args
        req = call_args[0][0]  # First positional arg is the Request object

        # Check URL
        self.assertIn("/worker/550e8400-e29b-41d4-a716-446655440000/tools/call", req.full_url)

        # Check headers
        self.assertEqual(req.get_header("Content-type"), "application/json")
        self.assertEqual(req.get_header("Authorization"), "Bearer test-token-123")

        # Check body
        body = json.loads(req.data.decode("utf-8"))
        self.assertEqual(body["tool_name"], "echo")
        self.assertEqual(body["parameters"], {"message": "hello"})
        self.assertEqual(body["timeout_secs"], 30)

        # Check return value
        self.assertEqual(result, "hello")


class TestCallToolHttpError(unittest.TestCase):
    """Test HTTP error handling."""

    def setUp(self):
        os.environ["IRONCLAW_ORCHESTRATOR_URL"] = "http://localhost:50051"
        os.environ["IRONCLAW_JOB_ID"] = "550e8400-e29b-41d4-a716-446655440000"
        os.environ["IRONCLAW_WORKER_TOKEN"] = "test-token-123"

    def tearDown(self):
        for var in ["IRONCLAW_ORCHESTRATOR_URL", "IRONCLAW_JOB_ID", "IRONCLAW_WORKER_TOKEN"]:
            os.environ.pop(var, None)

    @patch("ironclaw_tools.urllib.request.urlopen")
    def test_call_tool_http_error(self, mock_urlopen):
        from ironclaw_tools import call_tool

        mock_urlopen.side_effect = urllib.error.HTTPError(
            url="http://localhost:50051/worker/test/tools/call",
            code=500,
            msg="Internal Server Error",
            hdrs=None,
            fp=None,
        )

        with self.assertRaises(RuntimeError) as ctx:
            call_tool("echo", {"message": "hello"})

        self.assertIn("500", str(ctx.exception))


class TestConvenienceWrappers(unittest.TestCase):
    """Test that convenience wrappers call call_tool correctly."""

    def setUp(self):
        os.environ["IRONCLAW_ORCHESTRATOR_URL"] = "http://localhost:50051"
        os.environ["IRONCLAW_JOB_ID"] = "550e8400-e29b-41d4-a716-446655440000"
        os.environ["IRONCLAW_WORKER_TOKEN"] = "test-token-123"

    def tearDown(self):
        for var in ["IRONCLAW_ORCHESTRATOR_URL", "IRONCLAW_JOB_ID", "IRONCLAW_WORKER_TOKEN"]:
            os.environ.pop(var, None)

    @patch("ironclaw_tools.call_tool")
    def test_convenience_wrappers(self, mock_call_tool):
        from ironclaw_tools import shell, read_file, write_file, http_get

        mock_call_tool.return_value = "output"

        # Test shell
        shell("ls -la")
        mock_call_tool.assert_called_with("shell", {"command": "ls -la"}, timeout_secs=60)

        # Test read_file
        read_file("/workspace/README.md")
        mock_call_tool.assert_called_with("read_file", {"path": "/workspace/README.md"})

        # Test write_file
        write_file("/workspace/out.txt", "content")
        mock_call_tool.assert_called_with("write_file", {"path": "/workspace/out.txt", "content": "content"})

        # Test http_get
        http_get("https://api.example.com/data")
        mock_call_tool.assert_called_with("http", {"url": "https://api.example.com/data", "method": "GET"}, timeout_secs=30)


if __name__ == "__main__":
    unittest.main()
