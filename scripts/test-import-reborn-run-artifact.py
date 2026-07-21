from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import pathlib
import tempfile
import types
import unittest

SCRIPT = pathlib.Path(__file__).with_name("import-reborn-run-artifact.py")
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

    def test_thread_artifact_becomes_multiple_fixture_turns(self) -> None:
        artifact = {
            "schema": MODULE.THREAD_SCHEMA,
            "thread_id": "thread-1",
            "logs": {"complete": False},
            "redaction": {"pipeline": "deterministic-trace-redactor-v1"},
            "messages": [
                {"sequence": 1, "run_id": "run-1", "kind": "user", "content": "first"},
                {"sequence": 2, "run_id": "run-1", "kind": "assistant", "content": "one"},
                {"sequence": 3, "run_id": "run-2", "kind": "user", "content": "second"},
                self.tool_message(
                    4, "turn-b", "call-2", "builtin__two", "two result", run_id="run-2"
                ),
                {"sequence": 5, "run_id": "run-2", "kind": "assistant", "content": "two"},
            ],
        }

        candidate = MODULE.trace_candidate(artifact, None)

        self.assertEqual([turn["user_input"] for turn in candidate["turns"]], ["first", "second"])
        self.assertEqual(candidate["_review"]["source_schema"], MODULE.THREAD_SCHEMA)
        self.assertEqual(candidate["_review"]["source_thread_id"], "thread-1")

    def test_thread_artifact_reports_incomplete_run_without_discarding_completed_turns(
        self,
    ) -> None:
        artifact = self.thread_artifact(
            {"sequence": 1, "run_id": "run-1", "kind": "user", "content": "first"},
            {
                "sequence": 2,
                "run_id": "run-1",
                "kind": "assistant",
                "status": "finalized",
                "content": "done",
            },
            {
                "sequence": 3,
                "run_id": "run-2",
                "kind": "user",
                "status": "submitted",
                "content": "still running",
            },
        )

        candidate = MODULE.trace_candidate(artifact, None)

        self.assertEqual([turn["user_input"] for turn in candidate["turns"]], ["first"])
        self.assertEqual(
            candidate["_review"]["skipped_incomplete_runs"],
            [
                {
                    "run_id": "run-2",
                    "sequence": 3,
                    "reason": "run has no finalized assistant response",
                }
            ],
        )
        self.assertIn(
            "skipped_incomplete_runs",
            candidate["_review"]["required_actions"][0],
        )

    def test_thread_artifact_rejects_when_every_run_is_incomplete(self) -> None:
        artifact = self.thread_artifact(
            {"sequence": 1, "run_id": "run-1", "kind": "user", "content": "pending"}
        )

        with self.assertRaisesRegex(ValueError, "no complete run-scoped replayable turns"):
            MODULE.trace_candidate(artifact, None)

    def test_thread_artifact_reports_accepted_message_without_run_id(self) -> None:
        artifact = self.thread_artifact(
            {"sequence": 1, "kind": "user", "status": "accepted", "content": "failed"},
            {
                "sequence": 2,
                "run_id": "run-1",
                "kind": "user",
                "status": "submitted",
                "content": "try again",
            },
            {
                "sequence": 3,
                "run_id": "run-1",
                "kind": "assistant",
                "status": "finalized",
                "content": "done",
            },
        )

        candidate = MODULE.trace_candidate(artifact, None)

        self.assertEqual([turn["user_input"] for turn in candidate["turns"]], ["try again"])
        self.assertEqual(
            candidate["_review"]["skipped_unscoped_messages"],
            [{"sequence": 1, "kind": "user", "status": "accepted"}],
        )
        self.assertIn(
            "skipped_unscoped_messages",
            candidate["_review"]["required_actions"][0],
        )
        self.assertNotIn("failed", str(candidate))

    def test_thread_artifact_rejects_other_unscoped_messages(self) -> None:
        artifact = self.thread_artifact(
            {"sequence": 1, "kind": "system", "status": "finalized", "content": "prompt"},
            {"sequence": 2, "run_id": "run-1", "kind": "user", "content": "retry"},
            {"sequence": 3, "run_id": "run-1", "kind": "assistant", "content": "done"},
        )

        with self.assertRaisesRegex(ValueError, "no replayable user message"):
            MODULE.trace_candidate(artifact, None)

    def test_thread_artifact_rejects_when_every_message_is_unscoped(self) -> None:
        artifact = self.thread_artifact(
            {"sequence": 1, "kind": "user", "status": "accepted", "content": "failed"}
        )

        with self.assertRaisesRegex(ValueError, "no complete run-scoped replayable turns"):
            MODULE.trace_candidate(artifact, None)

    @staticmethod
    def thread_artifact(*messages: dict[str, object]) -> dict[str, object]:
        return {
            "schema": MODULE.THREAD_SCHEMA,
            "thread_id": "thread-1",
            "logs": {"complete": False},
            "redaction": {"pipeline": "deterministic-trace-redactor-v1"},
            "messages": list(messages),
        }

    @staticmethod
    def tool_message(
        sequence: int,
        provider_turn_id: str,
        call_id: str,
        name: str,
        content: str,
        run_id: str | None = None,
    ) -> dict[str, object]:
        message = {
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
        if run_id:
            message["run_id"] = run_id
        return message


if __name__ == "__main__":
    unittest.main()
