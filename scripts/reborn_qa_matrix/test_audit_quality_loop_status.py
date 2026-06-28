#!/usr/bin/env python3
"""Unit tests for scoped QA loop status aggregation."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from unittest import mock

import audit_quality_loop_status


def _surface_report(**overrides):
    report = {
        "workbook": "matrix.xlsx",
        "surface_count": 98,
        "surface_count_by_kind": {},
        "uncovered_surface_count": 0,
        "uncovered_surfaces": [],
    }
    report.update(overrides)
    return report


def _completeness_report(**overrides):
    report = {
        "workbook": "matrix.xlsx",
        "scoped_feature_count": 60,
        "scoped_test_count": 464,
        "missing_feature_field_count": 0,
        "missing_test_suite_count": 0,
        "missing_test_category_count": 0,
        "missing_feature_fields": [],
        "missing_test_suites": [],
        "missing_test_categories": [],
    }
    report.update(overrides)
    return report


def _coverage_report(**overrides):
    report = {
        "workbook": "matrix.xlsx",
        "scope_tokens": [],
        "all_feature_count": 99,
        "all_matrix_test_count": 747,
        "feature_count": 60,
        "matrix_test_count": 464,
        "hermetic_runner_coverage_pct": 94.2,
        "combined_runner_coverage_pct": 95.0,
        "matrix_only_or_new_combined_coverage_pct": 61.6,
        "actionable_gap_test_count": 0,
        "runner_ids_not_in_workbook": [],
        "covered_feature_count": 60,
        "matrix_only_or_new_feature_count": 40,
    }
    report.update(overrides)
    return report


def _execution_report(**overrides):
    report = {
        "workbook": "matrix.xlsx",
        "scoped_test_count": 464,
        "passed_test_count": 425,
        "external_existing_test_count": 37,
        "blocked_test_count": 2,
        "unknown_status_test_count": 0,
        "missing_execution_field_count": 0,
        "missing_external_reference_count": 0,
        "stale_external_reference_count": 0,
        "execution_evidence_test_count": 464,
        "missing_execution_fields": [],
        "missing_external_references": [],
        "stale_external_references": [],
        "blocked_tests": [
            {
                "test_id": "REBCLI-055-TC-18",
                "feature_id": "REBCLI-055",
                "status": "Blocked - external credential preflight",
                "category": "Live",
                "notes": "waived",
            },
            {
                "test_id": "REBCLI-055-TC-19",
                "feature_id": "REBCLI-055",
                "status": "Blocked - external credential preflight",
                "category": "Live",
                "notes": "waived",
            },
        ],
        "unknown_status_tests": [],
    }
    report.update(overrides)
    return report


def _defect_report(**overrides):
    report = {
        "workbook": "matrix.xlsx",
        "scoped_defect_count": 3,
        "resolved_defect_count": 1,
        "waived_defect_count": 2,
        "open_defect_count": 0,
        "scoped_non_passing_test_count": 2,
        "documented_non_passing_test_count": 2,
        "undocumented_non_passing_test_count": 0,
        "missing_defect_field_count": 0,
        "open_high_critical_defect_count": 0,
        "undocumented_non_passing_tests": [],
        "missing_defect_fields": [],
        "open_defects": [],
        "open_high_critical_defects": [],
    }
    report.update(overrides)
    return report


class AuditQualityLoopStatusTests(unittest.TestCase):
    def _patch_reports(
        self,
        *,
        surface=None,
        completeness=None,
        coverage=None,
        execution=None,
        defects=None,
    ):
        return mock.patch.multiple(
            audit_quality_loop_status,
            audit_surface_inventory=mock.Mock(
                build_audit=mock.Mock(return_value=surface or _surface_report())
            ),
            audit_workbook_completeness=mock.Mock(
                build_audit=mock.Mock(return_value=completeness or _completeness_report())
            ),
            report_coverage=mock.Mock(
                build_report=mock.Mock(return_value=coverage or _coverage_report())
            ),
            audit_execution_evidence=mock.Mock(
                build_audit=mock.Mock(return_value=execution or _execution_report())
            ),
            audit_defect_traceability=mock.Mock(
                build_audit=mock.Mock(return_value=defects or _defect_report())
            ),
        )

    def test_documented_blockers_pass_default_gate_but_remain_risks(self):
        with tempfile.TemporaryDirectory() as tmpdir, self._patch_reports():
            report = audit_quality_loop_status.build_status(
                Path(tmpdir) / "matrix.xlsx",
                Path(tmpdir),
            )

            self.assertTrue(report["loop_gate_passed"])
            self.assertEqual(report["blocking_gap_count"], 0)
            self.assertEqual(report["remaining_risks"]["blocked_test_count"], 2)
            self.assertEqual(report["remaining_risks"]["open_defect_count"], 0)
            self.assertEqual(report["defects_found"], 3)
            self.assertEqual(report["defects_fixed"], 1)
            self.assertEqual(report["defects_documented_or_waived"], 2)
            self.assertEqual(report["confidence_score"], "95.0%")

    def test_strict_no_blocked_fails_until_live_blockers_execute(self):
        with tempfile.TemporaryDirectory() as tmpdir, self._patch_reports():
            report = audit_quality_loop_status.build_status(
                Path(tmpdir) / "matrix.xlsx",
                Path(tmpdir),
                strict_no_blocked=True,
            )

            self.assertFalse(report["loop_gate_passed"])
            self.assertEqual(report["blocking_gap_count"], 2)

    def test_surface_coverage_execution_and_defect_gaps_are_blocking(self):
        with tempfile.TemporaryDirectory() as tmpdir, self._patch_reports(
            surface=_surface_report(uncovered_surface_count=1),
            completeness=_completeness_report(missing_test_suite_count=1),
            coverage=_coverage_report(
                actionable_gap_test_count=1,
                runner_ids_not_in_workbook=["REBCLI-999-TC-01"],
            ),
            execution=_execution_report(
                missing_execution_field_count=1,
                unknown_status_test_count=1,
                missing_external_reference_count=1,
                stale_external_reference_count=1,
                blocked_test_count=0,
                blocked_tests=[],
            ),
            defects=_defect_report(
                undocumented_non_passing_test_count=1,
                missing_defect_field_count=1,
                open_defect_count=1,
                open_defects=[
                    {
                        "defect_id": "DEF-999",
                        "feature_id": "REBCLI-055",
                        "severity": "Low",
                        "status": "Open",
                        "title": "Open defect",
                    }
                ],
                open_high_critical_defect_count=1,
            ),
        ):
            report = audit_quality_loop_status.build_status(
                Path(tmpdir) / "matrix.xlsx",
                Path(tmpdir),
            )

            self.assertFalse(report["loop_gate_passed"])
            self.assertEqual(report["blocking_gap_count"], 11)

    def test_main_exit_code_tracks_gate_state(self):
        with tempfile.TemporaryDirectory() as tmpdir, self._patch_reports():
            workbook = Path(tmpdir) / "matrix.xlsx"

            self.assertEqual(
                audit_quality_loop_status.main(
                    ["--workbook", str(workbook), "--repo-root", tmpdir]
                ),
                0,
            )
            self.assertEqual(
                audit_quality_loop_status.main(
                    [
                        "--workbook",
                        str(workbook),
                        "--repo-root",
                        tmpdir,
                        "--strict-no-blocked",
                    ]
                ),
                1,
            )


if __name__ == "__main__":
    unittest.main()
