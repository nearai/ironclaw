#!/usr/bin/env python3
"""Sabotage tests for the WS12 shard policy."""

from __future__ import annotations

import copy
import importlib.util
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("ws12_suite_shards.py")
SPEC = importlib.util.spec_from_file_location("ws12_suite_shards", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ShardPolicySabotageTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        for name in ("a.py", "b.py", "evidence.txt"):
            (self.root / name).write_text("fixture\n", encoding="utf-8")
        self.policy = {
            "playwright": {
                "shard_count": 2,
                "max_shard_seconds": 20,
                "discovery_globs": ["*.py"],
            },
            "suite": [
                {
                    "path": "a.py",
                    "affinity": "provider-world",
                    "historical_seconds": 5,
                    "retry": "never",
                },
                {
                    "path": "b.py",
                    "affinity": "provider-world",
                    "historical_seconds": 6,
                    "retry": "never",
                },
            ],
        }

    def tearDown(self) -> None:
        self.temp.cleanup()

    def assert_rejected(self, policy: dict, message: str) -> None:
        with self.assertRaisesRegex(MODULE.PolicyError, message):
            MODULE.validate_and_generate(policy, self.root)

    def test_missing_discovered_suite_is_rejected(self) -> None:
        policy = copy.deepcopy(self.policy)
        policy["suite"].pop()
        self.assert_rejected(policy, "missing from policy")

    def test_duplicate_suite_is_rejected(self) -> None:
        policy = copy.deepcopy(self.policy)
        policy["suite"].append(copy.deepcopy(policy["suite"][0]))
        self.assert_rejected(policy, "duplicate suite")

    def test_affinity_group_is_never_split(self) -> None:
        matrix = MODULE.validate_and_generate(self.policy, self.root)
        provider_shards = [
            shard for shard in matrix if "provider-world" in shard["affinities"]
        ]
        self.assertEqual(len(provider_shards), 1)
        self.assertEqual(set(provider_shards[0]["files"].split()), {"a.py", "b.py"})

    def test_affinity_over_budget_is_rejected(self) -> None:
        policy = copy.deepcopy(self.policy)
        policy["playwright"]["max_shard_seconds"] = 10
        self.assert_rejected(policy, "affinity group provider-world")

    def test_retry_on_protected_suite_is_rejected(self) -> None:
        policy = copy.deepcopy(self.policy)
        policy["suite"][0]["retry"] = "on-failure"
        self.assert_rejected(policy, 'must set retry = "never"')

    def test_owned_waiver_requires_evidence(self) -> None:
        policy = copy.deepcopy(self.policy)
        policy["suite"].pop()
        policy["waiver"] = [
            {
                "path": "b.py",
                "owner": "owner",
                "reason": "covered elsewhere",
                "evidence": "missing.txt",
            }
        ]
        self.assert_rejected(policy, "waiver evidence does not exist")


if __name__ == "__main__":
    unittest.main()
