#!/usr/bin/env python3
"""Structural ownership tests for canary and Reborn GitHub workflows."""

from __future__ import annotations

import re
import textwrap
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
LIVE_WORKFLOW = ROOT / ".github" / "workflows" / "live-canary.yml"
REBORN_WORKFLOW = ROOT / ".github" / "workflows" / "reborn-tests.yml"
UPGRADE_WORKFLOW = ROOT / ".github" / "workflows" / "upgrade-compatibility.yml"
REPLAY_WORKFLOW = ROOT / ".github" / "workflows" / "replay-gate.yml"
RUNNER = ROOT / "scripts" / "live-canary" / "run.sh"

LIVE_LANES = {
    "public-smoke",
    "persona-rotating",
    "private-oauth",
    "provider-matrix",
    "release-public-full",
    "auth-live-seeded",
    "auth-browser-consent",
    "reborn-webui-v2-live-qa",
}
LIVE_HELPER_JOBS = {
    "approve-reborn-webui-v2-pr-live-qa",
    "preflight-reborn-webui-v2-google-oauth",
    "prepare-reborn-webui-v2-live-qa",
    "canary-report",
}
NON_LIVE_LANES = {
    "deterministic-replay",
    "auth-smoke",
    "auth-full",
    "auth-channels",
    "workflow-canary",
    "upgrade-canary",
}
REBORN_SCOPE_IF = (
    "needs.changes.outputs.docs_only != 'true' && "
    "needs.changes.outputs.has_reborn_tests == 'true'"
)


def read_optional(path: Path) -> str:
    return path.read_text(encoding="utf-8") if path.exists() else ""


def active_text(text: str) -> str:
    """Drop comment-only lines so disabled YAML/shell cannot satisfy a test."""

    return "\n".join(
        line for line in text.splitlines() if not line.lstrip().startswith("#")
    )


def unquote(value: str) -> str:
    value = value.strip()
    if len(value) >= 2 and value[0] == value[-1] and value[0] in "'\"":
        return value[1:-1]
    return value


def mapping_block(text: str, key: str, indent: int) -> str:
    """Return one indentation-scoped YAML mapping entry, including its key."""

    lines = text.splitlines()
    spaces = " " * indent
    key_pattern = re.compile(rf"^{spaces}{re.escape(key)}:\s*(?:#.*)?$")
    sibling_pattern = re.compile(rf"^{spaces}[A-Za-z0-9_.-]+:\s*(?:.*)?$")
    start = next(
        (index for index, line in enumerate(lines) if key_pattern.match(line)), None
    )
    if start is None:
        return ""

    end = len(lines)
    for index in range(start + 1, len(lines)):
        if sibling_pattern.match(lines[index]):
            end = index
            break
    return "\n".join(lines[start:end])


def mapping_keys(text: str, indent: int) -> set[str]:
    spaces = " " * indent
    pattern = re.compile(rf"^{spaces}([A-Za-z0-9_.-]+):(?:\s|$)", re.MULTILINE)
    return set(pattern.findall(text))


def list_values(text: str, indent: int) -> list[str]:
    spaces = " " * indent
    pattern = re.compile(rf"^{spaces}-\s+([^#\n]+?)(?:\s+#.*)?$", re.MULTILINE)
    return [unquote(value) for value in pattern.findall(text)]


def scalar_value(text: str, key: str, indent: int) -> str | None:
    spaces = " " * indent
    match = re.search(
        rf"^{spaces}{re.escape(key)}:\s+([^#\n]+?)(?:\s+#.*)?$",
        text,
        re.MULTILINE,
    )
    if not match:
        return None
    return unquote(match.group(1))


def sequence_blocks(text: str, indent: int) -> list[str]:
    """Return active YAML sequence items at one indentation level."""

    lines = text.splitlines()
    spaces = " " * indent
    item_pattern = re.compile(rf"^{spaces}-\s+[^#\s]")
    starts = [
        index
        for index, line in enumerate(lines)
        if not line.lstrip().startswith("#") and item_pattern.match(line)
    ]
    return [
        "\n".join(lines[start : starts[position + 1] if position + 1 < len(starts) else len(lines)])
        for position, start in enumerate(starts)
    ]


def yaml_steps(job: str) -> list[str]:
    return sequence_blocks(mapping_block(job, "steps", 4), 6)


def step_scalar(step: str, key: str) -> str | None:
    active = active_text(step)
    match = re.search(
        rf"^(?:      -\s+|        ){re.escape(key)}:\s+([^#\n]+?)(?:\s+#.*)?$",
        active,
        re.MULTILINE,
    )
    return unquote(match.group(1)) if match else None


def step_with_scalar(step: str, key: str) -> str | None:
    return scalar_value(mapping_block(active_text(step), "with", 8), key, 10)


def step_run_body(step: str) -> str | None:
    """Return an active scalar or block `run` body from one YAML step."""

    lines = step.splitlines()
    header_pattern = re.compile(r"^(?:      -\s+|        )run:\s*(.*?)\s*$")
    for index, line in enumerate(lines):
        if line.lstrip().startswith("#"):
            continue
        match = header_pattern.match(line)
        if not match:
            continue
        header = match.group(1)
        if header not in {"|", "|-", ">", ">-"}:
            return unquote(header)
        body = active_text("\n".join(lines[index + 1 :]))
        return textwrap.dedent(body).strip()
    return None


def active_step_uses(job: str) -> set[str]:
    return {value for step in yaml_steps(job) if (value := step_scalar(step, "uses"))}


def active_run_text(job: str) -> str:
    return "\n".join(
        body for step in yaml_steps(job) if (body := step_run_body(step))
    )


def step_using(job: str, action_prefix: str) -> str:
    return next(
        (
            step
            for step in yaml_steps(job)
            if (step_scalar(step, "uses") or "").startswith(action_prefix)
        ),
        "",
    )


def jobs(text: str) -> set[str]:
    return mapping_keys(mapping_block(text, "jobs", 0), 2)


def job_block(text: str, job: str) -> str:
    return mapping_block(mapping_block(text, "jobs", 0), job, 2)


def workflow_dispatch_lane_options(text: str) -> set[str]:
    on_block = mapping_block(text, "on", 0)
    dispatch_block = mapping_block(on_block, "workflow_dispatch", 2)
    inputs_block = mapping_block(dispatch_block, "inputs", 4)
    lane_block = mapping_block(inputs_block, "lane", 6)
    options_block = mapping_block(lane_block, "options", 8)
    return set(list_values(options_block, 10))


def job_needs(text: str, job: str) -> set[str]:
    needs_block = mapping_block(job_block(text, job), "needs", 4)
    return set(list_values(needs_block, 6))


class WorkflowTieringTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.live = read_optional(LIVE_WORKFLOW)
        cls.reborn = read_optional(REBORN_WORKFLOW)
        cls.upgrade = read_optional(UPGRADE_WORKFLOW)
        cls.replay = read_optional(REPLAY_WORKFLOW)
        cls.runner = read_optional(RUNNER)

    def test_live_dispatch_choices_and_jobs_are_live_only(self) -> None:
        self.assertEqual(
            workflow_dispatch_lane_options(self.live),
            LIVE_LANES | {"all"},
        )
        self.assertEqual(jobs(self.live), LIVE_LANES | LIVE_HELPER_JOBS)
        self.assertTrue(NON_LIVE_LANES.isdisjoint(jobs(self.live)))

    def test_live_report_needs_only_live_lanes_and_live_preflight(self) -> None:
        self.assertEqual(
            job_needs(self.live, "canary-report"),
            LIVE_LANES | {"preflight-reborn-webui-v2-google-oauth"},
        )

    def test_reborn_ci_owns_mock_auth_matrix(self) -> None:
        mock_auth = job_block(self.reborn, "mock-auth-e2e")
        self.assertTrue(mock_auth)
        matrix = mapping_block(
            mapping_block(mapping_block(mock_auth, "strategy", 4), "matrix", 6),
            "profile",
            8,
        )
        self.assertEqual(set(list_values(matrix, 10)), {"smoke", "full", "channels"})
        self.assertEqual(scalar_value(mock_auth, "needs", 4), "changes")
        self.assertEqual(scalar_value(mock_auth, "if", 4), REBORN_SCOPE_IF)
        self.assertRegex(mock_auth, r"(?m)^      LANE: auth-\$\{\{ matrix\.profile \}\}$")
        self.assertRegex(mock_auth, r"(?m)^      PROVIDER: mock$")
        self.assertRegex(mock_auth, r"(?m)^      PLAYWRIGHT_INSTALL: with-deps$")
        self.assertEqual(scalar_value(mock_auth, "ALLOW_LOCAL_TOOLS", 6), "true")
        self.assertEqual(scalar_value(mock_auth, "AGENT_AUTO_APPROVE_TOOLS", 6), "true")
        uses = active_step_uses(mock_auth)
        for action in (
            "actions/checkout@",
            "dtolnay/rust-toolchain@",
            "Swatinem/rust-cache@",
            "actions/setup-python@",
        ):
            self.assertTrue(any(value.startswith(action) for value in uses), action)
        self.assertEqual(
            step_with_scalar(step_using(mock_auth, "actions/setup-python@"), "python-version"),
            "3.12",
        )
        runs = active_run_text(mock_auth)
        self.assertIn("./scripts/build-wasm-extensions.sh", runs.splitlines())
        self.assertIn("scripts/live-canary/run.sh", runs.splitlines())

    def test_reborn_ci_owns_hermetic_workflow(self) -> None:
        workflow = job_block(self.reborn, "workflow-hermetic-e2e")
        self.assertTrue(workflow)
        self.assertEqual(scalar_value(workflow, "needs", 4), "changes")
        self.assertEqual(scalar_value(workflow, "if", 4), REBORN_SCOPE_IF)
        self.assertRegex(workflow, r"(?m)^      LANE: workflow-canary$")
        self.assertRegex(workflow, r"(?m)^      PROVIDER: mock$")
        self.assertRegex(workflow, r"(?m)^      PLAYWRIGHT_INSTALL: skip$")
        self.assertEqual(scalar_value(workflow, "ALLOW_LOCAL_TOOLS", 6), "true")
        self.assertEqual(scalar_value(workflow, "AGENT_AUTO_APPROVE_TOOLS", 6), "true")
        uses = active_step_uses(workflow)
        for action in (
            "actions/checkout@",
            "dtolnay/rust-toolchain@",
            "Swatinem/rust-cache@",
            "actions/setup-python@",
        ):
            self.assertTrue(any(value.startswith(action) for value in uses), action)
        self.assertEqual(
            step_with_scalar(step_using(workflow, "actions/setup-python@"), "python-version"),
            "3.12",
        )
        runs = active_run_text(workflow)
        self.assertIn("./scripts/build-wasm-extensions.sh --channels", runs.splitlines())
        self.assertIn("scripts/live-canary/run.sh", runs.splitlines())

    def test_reborn_changes_job_runs_ownership_test_for_every_event(self) -> None:
        changes = job_block(self.reborn, "changes")
        checkout = step_using(changes, "actions/checkout@")
        self.assertTrue(checkout)
        self.assertIsNone(step_scalar(checkout, "if"))
        self.assertEqual(step_with_scalar(checkout, "ref"), "${{ inputs.ref || github.sha }}")
        self.assertEqual(step_with_scalar(checkout, "fetch-depth"), "0")
        self.assertEqual(step_with_scalar(checkout, "persist-credentials"), "false")
        steps = yaml_steps(changes)
        ownership_index = next(
            (
                index
                for index, step in enumerate(steps)
                if "python3 scripts/live-canary/test_workflow_tiering.py -v"
                in (step_run_body(step) or "").splitlines()
            ),
            None,
        )
        non_diff_index = next(
            (index for index, step in enumerate(steps) if step_scalar(step, "id") == "non_diff"),
            None,
        )
        self.assertIsNotNone(ownership_index)
        self.assertIsNotNone(non_diff_index)
        self.assertLess(ownership_index, non_diff_index)

    def test_reborn_rollup_requires_both_hermetic_jobs(self) -> None:
        rollup = job_block(self.reborn, "reborn-tests")
        self.assertTrue(
            {"mock-auth-e2e", "workflow-hermetic-e2e"}.issubset(
                job_needs(self.reborn, "reborn-tests")
            )
        )
        for job in ("mock-auth-e2e", "workflow-hermetic-e2e"):
            self.assertIn(
                f'if ! job_result_ok "{job}" "${{{{ needs.{job}.result }}}}" false "allow"; then',
                active_run_text(rollup),
            )

    def test_upgrade_has_dedicated_manual_workflow(self) -> None:
        self.assertEqual(scalar_value(self.upgrade, "name", 0), "Upgrade Compatibility")
        on_block = mapping_block(self.upgrade, "on", 0)
        self.assertEqual(mapping_keys(on_block, 2), {"workflow_dispatch"})
        dispatch = mapping_block(on_block, "workflow_dispatch", 2)
        inputs = mapping_block(dispatch, "inputs", 4)
        self.assertEqual(mapping_keys(inputs, 6), {"previous_ref", "current_ref"})
        for input_name in ("previous_ref", "current_ref"):
            input_block = mapping_block(inputs, input_name, 6)
            self.assertEqual(scalar_value(input_block, "required", 8), "true")

        self.assertEqual(jobs(self.upgrade), {"upgrade-compatibility"})
        upgrade_job = job_block(self.upgrade, "upgrade-compatibility")
        checkout = step_using(upgrade_job, "actions/checkout@")
        self.assertTrue(checkout)
        self.assertEqual(step_with_scalar(checkout, "ref"), "${{ inputs.current_ref }}")
        self.assertEqual(step_with_scalar(checkout, "fetch-depth"), "0")
        self.assertEqual(step_with_scalar(checkout, "fetch-tags"), "true")
        self.assertEqual(step_with_scalar(checkout, "persist-credentials"), "false")
        self.assertRegex(upgrade_job, r"(?m)^          LANE: upgrade-canary$")
        self.assertRegex(upgrade_job, r"(?m)^          CURRENT_REF: HEAD$")
        self.assertRegex(
            upgrade_job,
            r"(?m)^          PREVIOUS_REF: \$\{\{ inputs\.previous_ref \}\}$",
        )
        uses = active_step_uses(upgrade_job)
        self.assertTrue(any(value.startswith("dtolnay/rust-toolchain@") for value in uses))
        self.assertTrue(any(value.startswith("Swatinem/rust-cache@") for value in uses))
        self.assertIn("scripts/live-canary/run.sh", active_run_text(upgrade_job).splitlines())
        upload = step_using(upgrade_job, "actions/upload-artifact@")
        self.assertEqual(step_with_scalar(upload, "retention-days"), "30")
        self.assertIn(
            "scripts/live-canary/scrub-artifacts.sh artifacts/live-canary",
            active_run_text(upgrade_job).splitlines(),
        )
        self.assertNotIn("${{ secrets.", self.upgrade)
        self.assertNotIn("${{ vars.", self.upgrade)

    def test_upgrade_workflow_uses_the_owned_upgrade_script_via_runner(self) -> None:
        upgrade_case = re.search(
            r"(?ms)^    upgrade-canary\)\n(?P<body>.*?)(?=^    [A-Za-z0-9*-]+\))",
            active_text(self.runner),
        )
        self.assertIsNotNone(upgrade_case)
        self.assertIn(
            "run_with_timeout scripts/live-canary/upgrade-canary.sh",
            upgrade_case.group("body"),
        )

    def test_replay_gate_owns_deterministic_replay(self) -> None:
        replay_job = job_block(self.replay, "replay-snapshots")
        self.assertTrue(replay_job)
        runs = active_run_text(replay_job)
        self.assertIn("cargo insta test", runs)
        self.assertIn('--features "libsql,replay"', runs)
        self.assertIn("--test e2e_recorded_trace", runs)
        self.assertIn("--test e2e_live", runs)

    def test_active_step_parser_ignores_commented_out_steps(self) -> None:
        job = """
  sample:
    steps:
      # - uses: actions/checkout@comment-only
      - name: Active shell
        # uses: dtolnay/rust-toolchain@comment-only
        run: echo active
      # - run: scripts/live-canary/run.sh
"""
        self.assertEqual(active_step_uses(job), set())
        self.assertEqual(active_run_text(job), "echo active")


if __name__ == "__main__":
    unittest.main()
