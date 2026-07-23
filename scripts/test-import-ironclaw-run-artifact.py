from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import pathlib
import tempfile
import types
import unittest

SCRIPT = pathlib.Path(__file__).with_name("import-ironclaw-run-artifact.py")
SPEC = importlib.util.spec_from_file_location("run_artifact_importer", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class RunArtifactImporterTest(unittest.TestCase):
    def test_groups_parallel_calls_and_attaches_results_to_next_step(self) -> None:
        artifact = {
            "schema": MODULE.SCHEMA,
            "run": {"run_id": "run-1"},
            "logs": {"complete": False},
            "redaction": {"pipeline": "deterministic-trace-redactor-v1"},
            "messages": [
                {"sequence": 1, "kind": "user", "content": "do it"},
                self.tool_message(2, "turn-a", "call-1", "builtin__one", "one result"),
                self.tool_message(3, "turn-a", "call-2", "builtin__two", "two result"),
                {"sequence": 4, "kind": "assistant", "content": "done"},
            ],
        }

        candidate = MODULE.trace_candidate(artifact, None)

        turn = candidate["turns"][0]
        self.assertEqual(turn["user_input"], "do it")
        self.assertEqual(len(turn["steps"]), 2)
        self.assertEqual(len(turn["steps"][0]["response"]["tool_calls"]), 2)
        self.assertNotIn("expected_tool_results", turn["steps"][0])
        self.assertEqual(len(turn["steps"][1]["expected_tool_results"]), 2)
        self.assertEqual(turn["expects"]["tools_used"], ["builtin__one", "builtin__two"])
        self.assertEqual(candidate["_review"]["status"], "candidate")

    def test_groups_sequential_calls_and_attaches_each_result_group_to_the_next_step(self) -> None:
        artifact = self.artifact(
            [
                {"sequence": 1, "kind": "user", "content": "do it"},
                self.tool_message(2, "turn-a", "call-1", "builtin__one", "one result"),
                self.tool_message(3, "turn-b", "call-2", "builtin__two", "two result"),
                {"sequence": 4, "kind": "assistant", "content": "done"},
            ]
        )

        steps = MODULE.trace_candidate(artifact, None)["turns"][0]["steps"]

        self.assertEqual(len(steps), 3)
        self.assertNotIn("expected_tool_results", steps[0])
        self.assertEqual(steps[1]["expected_tool_results"][0]["tool_call_id"], "call-1")
        self.assertEqual(steps[2]["expected_tool_results"][0]["tool_call_id"], "call-2")

    def test_rejects_tool_results_without_a_finalized_assistant_response(self) -> None:
        artifact = self.artifact(
            [
                {"sequence": 1, "kind": "user", "content": "do it"},
                self.tool_message(2, "turn-a", "call-1", "builtin__one", "one result"),
            ]
        )

        with self.assertRaisesRegex(ValueError, "ends with tool results"):
            MODULE.trace_candidate(artifact, None)

    def test_main_reports_malformed_nested_tool_calls_as_a_validation_error(self) -> None:
        artifact = self.artifact(
            [
                {"sequence": 1, "kind": "user", "content": "do it"},
                {
                    "sequence": 2,
                    "kind": "tool_result_reference",
                    "content": "one result",
                    "tool_call": {
                        "provider_turn_id": "turn-a",
                        "provider_model_id": "model-a",
                        "capability_id": "builtin__one",
                    },
                },
            ]
        )
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            artifact_path = root / "artifact.json"
            artifact_path.write_text(json.dumps(artifact), encoding="utf-8")
            original_parse_args = MODULE.parse_args
            MODULE.parse_args = lambda: types.SimpleNamespace(
                artifact=artifact_path,
                output=root / "candidate.json",
                model_name=None,
            )
            stderr = io.StringIO()
            try:
                with contextlib.redirect_stderr(stderr):
                    self.assertEqual(MODULE.main(), 2)
            finally:
                MODULE.parse_args = original_parse_args

        self.assertIn("provider_call_id", stderr.getvalue())

    @staticmethod
    def artifact(messages: list[dict[str, object]]) -> dict[str, object]:
        return {
            "schema": MODULE.SCHEMA,
            "run": {"run_id": "run-1"},
            "logs": {"complete": False},
            "redaction": {"pipeline": "deterministic-trace-redactor-v1"},
            "messages": messages,
        }

    @staticmethod
    def tool_message(
        sequence: int, provider_turn_id: str, call_id: str, name: str, content: str
    ) -> dict[str, object]:
        return {
            "sequence": sequence,
            "kind": "tool_result_reference",
            "content": content,
            "tool_call": {
                "provider_model_id": "model-a",
                "provider_turn_id": provider_turn_id,
                "provider_call_id": call_id,
                "capability_id": name,
                "arguments": {"value": sequence},
            },
        }


if __name__ == "__main__":
    unittest.main()
