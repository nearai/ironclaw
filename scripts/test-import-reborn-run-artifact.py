from __future__ import annotations

import importlib.util
import pathlib
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
