#!/usr/bin/env python3
"""Report QA matrix workbook coverage for Reborn WebUI v2/OpenAI rows."""

from __future__ import annotations

import argparse
import json
import re
import sys
import xml.etree.ElementTree as ET
from pathlib import Path, PurePosixPath
from zipfile import ZipFile

import run_hermetic_qa

ROOT = Path(__file__).resolve().parents[2]
SHEET_NS = {"x": "http://schemas.openxmlformats.org/spreadsheetml/2006/main"}
REL_NS = {"r": "http://schemas.openxmlformats.org/package/2006/relationships"}
OFFICE_REL = "{http://schemas.openxmlformats.org/officeDocument/2006/relationships}id"
TEST_ID_RE = re.compile(r"REBCLI-\d{3}-TC-\d{2}")
FEATURE_ID_RE = re.compile(r"REBCLI-\d{3}")


def _column_index(cell_ref: str) -> int:
    letters = "".join(ch for ch in cell_ref if ch.isalpha())
    index = 0
    for letter in letters:
        index = index * 26 + (ord(letter.upper()) - ord("A") + 1)
    return index - 1


def _shared_strings(xlsx: ZipFile) -> list[str]:
    try:
        root = ET.fromstring(xlsx.read("xl/sharedStrings.xml"))
    except KeyError:
        return []
    strings: list[str] = []
    for item in root.findall("x:si", SHEET_NS):
        strings.append("".join(node.text or "" for node in item.findall(".//x:t", SHEET_NS)))
    return strings


def _cell_text(cell: ET.Element, shared_strings: list[str]) -> str:
    inline = cell.find("x:is/x:t", SHEET_NS)
    if inline is not None:
        return inline.text or ""
    value = cell.find("x:v", SHEET_NS)
    if value is None:
        return ""
    text = value.text or ""
    if cell.attrib.get("t") == "s":
        try:
            return shared_strings[int(text)]
        except (IndexError, ValueError):
            return ""
    return text


def _sheet_paths(xlsx: ZipFile) -> dict[str, str]:
    workbook = ET.fromstring(xlsx.read("xl/workbook.xml"))
    rels = ET.fromstring(xlsx.read("xl/_rels/workbook.xml.rels"))
    targets = {
        rel.attrib["Id"]: rel.attrib["Target"]
        for rel in rels.findall("r:Relationship", REL_NS)
    }
    paths: dict[str, str] = {}
    for sheet in workbook.findall(".//x:sheet", SHEET_NS):
        name = sheet.attrib["name"]
        rel_id = sheet.attrib[OFFICE_REL]
        target = PurePosixPath(targets[rel_id].lstrip("/"))
        if not str(target).startswith("xl/"):
            target = PurePosixPath("xl") / target
        paths[name] = str(target)
    return paths


def _sheet_rows(xlsx: ZipFile, sheet_name: str) -> list[list[str]]:
    path = _sheet_paths(xlsx)[sheet_name]
    shared_strings = _shared_strings(xlsx)
    root = ET.fromstring(xlsx.read(path))
    rows: list[list[str]] = []
    for row in root.findall(".//x:sheetData/x:row", SHEET_NS):
        values: list[str] = []
        for cell in row.findall("x:c", SHEET_NS):
            ref = cell.attrib.get("r", "")
            column = _column_index(ref) if ref else len(values)
            while len(values) <= column:
                values.append("")
            values[column] = _cell_text(cell, shared_strings)
        rows.append(values)
    return rows


def workbook_ids(workbook_path: Path) -> tuple[set[str], set[str]]:
    with ZipFile(workbook_path) as xlsx:
        feature_rows = _sheet_rows(xlsx, "Feature Inventory")
        test_rows = _sheet_rows(xlsx, "Test Cases")
    feature_ids = {
        row[0]
        for row in feature_rows[1:]
        if row and FEATURE_ID_RE.fullmatch(row[0] or "")
    }
    test_ids = {
        row[0]
        for row in test_rows[1:]
        if row and TEST_ID_RE.fullmatch(row[0] or "")
    }
    return feature_ids, test_ids


def hermetic_runner_ids(default_only: bool = False) -> set[str]:
    ids: set[str] = set()
    for case in run_hermetic_qa.CASES.values():
        if default_only and not case.default_enabled:
            continue
        ids.update(case.qa_matrix_test_ids)
    return ids


def matrix_only_runner_ids(default_only: bool = False) -> set[str]:
    ids: set[str] = set()
    for case in run_hermetic_qa.CASES.values():
        if default_only and not case.default_enabled:
            continue
        if run_hermetic_qa._case_has_matrix_only_command(case):
            ids.update(case.qa_matrix_test_ids)
    return ids


def existing_ci_only_ids(default_only: bool = False) -> set[str]:
    ids: set[str] = set()
    for case in run_hermetic_qa.CASES.values():
        if default_only and not case.default_enabled:
            continue
        if run_hermetic_qa._case_existing_ci_only(case):
            ids.update(case.qa_matrix_test_ids)
    return ids


def live_runner_ids(repo_root: Path = ROOT) -> set[str]:
    live_runner = repo_root / "scripts/reborn_webui_v2_live_qa/run_live_qa.py"
    if not live_runner.exists():
        return set()
    return set(TEST_ID_RE.findall(live_runner.read_text(encoding="utf-8")))


def _pct(numerator: int, denominator: int) -> float:
    return round((numerator / denominator * 100.0), 1) if denominator else 0.0


def build_report(workbook_path: Path, repo_root: Path = ROOT) -> dict[str, object]:
    feature_ids, matrix_ids = workbook_ids(workbook_path)
    hermetic_ids = hermetic_runner_ids()
    default_hermetic_ids = hermetic_runner_ids(default_only=True)
    matrix_only_ids = matrix_only_runner_ids(default_only=True)
    existing_ci_ids = existing_ci_only_ids(default_only=True)
    live_ids = live_runner_ids(repo_root)
    combined_ids = hermetic_ids | live_ids
    matrix_only_combined_ids = matrix_only_ids | live_ids
    covered_matrix_ids = matrix_ids & combined_ids
    matrix_only_covered_ids = matrix_ids & matrix_only_combined_ids
    covered_features = {test_id[:10] for test_id in covered_matrix_ids}
    matrix_only_covered_features = {test_id[:10] for test_id in matrix_only_covered_ids}
    return {
        "workbook": str(workbook_path),
        "feature_count": len(feature_ids),
        "matrix_test_count": len(matrix_ids),
        "hermetic_runner_test_count": len(matrix_ids & hermetic_ids),
        "default_hermetic_runner_test_count": len(matrix_ids & default_hermetic_ids),
        "live_runner_test_count": len(matrix_ids & live_ids),
        "combined_runner_test_count": len(covered_matrix_ids),
        "matrix_only_or_new_runner_test_count": len(matrix_ids & matrix_only_ids),
        "existing_ci_only_test_count": len(matrix_ids & existing_ci_ids),
        "matrix_only_or_new_combined_test_count": len(matrix_only_covered_ids),
        "hermetic_runner_coverage_pct": _pct(len(matrix_ids & hermetic_ids), len(matrix_ids)),
        "combined_runner_coverage_pct": _pct(len(covered_matrix_ids), len(matrix_ids)),
        "matrix_only_or_new_runner_coverage_pct": _pct(
            len(matrix_ids & matrix_only_ids), len(matrix_ids)
        ),
        "matrix_only_or_new_combined_coverage_pct": _pct(
            len(matrix_only_covered_ids), len(matrix_ids)
        ),
        "covered_feature_count": len(covered_features),
        "covered_feature_pct": _pct(len(covered_features), len(feature_ids)),
        "matrix_only_or_new_feature_count": len(matrix_only_covered_features),
        "matrix_only_or_new_feature_pct": _pct(
            len(matrix_only_covered_features), len(feature_ids)
        ),
        "runner_ids_not_in_workbook": sorted(combined_ids - matrix_ids),
        "workbook_ids_not_in_hermetic_runner": sorted(matrix_ids - hermetic_ids),
        "workbook_ids_not_in_combined_runner": sorted(matrix_ids - combined_ids),
    }


def _print_text(report: dict[str, object], include_missing: bool) -> None:
    print(f"Workbook: {report['workbook']}")
    print(f"Features: {report['feature_count']}")
    print(f"Matrix test cases: {report['matrix_test_count']}")
    print(
        "Hermetic runner coverage: "
        f"{report['hermetic_runner_test_count']} / {report['matrix_test_count']} "
        f"= {report['hermetic_runner_coverage_pct']}%"
    )
    print(
        "Matrix-only/new hermetic coverage: "
        f"{report['matrix_only_or_new_runner_test_count']} / "
        f"{report['matrix_test_count']} = "
        f"{report['matrix_only_or_new_runner_coverage_pct']}%"
    )
    print(
        "Already-existing CI-only traceability: "
        f"{report['existing_ci_only_test_count']} / {report['matrix_test_count']}"
    )
    print(
        "Hermetic + live runner coverage: "
        f"{report['combined_runner_test_count']} / {report['matrix_test_count']} "
        f"= {report['combined_runner_coverage_pct']}%"
    )
    print(
        "Matrix-only/new + live runner coverage: "
        f"{report['matrix_only_or_new_combined_test_count']} / "
        f"{report['matrix_test_count']} = "
        f"{report['matrix_only_or_new_combined_coverage_pct']}%"
    )
    print(
        "Executable feature coverage: "
        f"{report['covered_feature_count']} / {report['feature_count']} "
        f"= {report['covered_feature_pct']}%"
    )
    print(
        "Matrix-only/new feature coverage: "
        f"{report['matrix_only_or_new_feature_count']} / {report['feature_count']} "
        f"= {report['matrix_only_or_new_feature_pct']}%"
    )
    if report["runner_ids_not_in_workbook"]:
        print("Runner IDs not in workbook:")
        for test_id in report["runner_ids_not_in_workbook"]:
            print(f"  {test_id}")
    if include_missing:
        print("Workbook IDs not in hermetic runner:")
        for test_id in report["workbook_ids_not_in_hermetic_runner"]:
            print(f"  {test_id}")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workbook", type=Path, required=True)
    parser.add_argument("--repo-root", type=Path, default=ROOT)
    parser.add_argument("--json", action="store_true", help="emit JSON")
    parser.add_argument(
        "--include-missing",
        action="store_true",
        help="include workbook IDs not mapped by the hermetic runner in text output",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    report = build_report(args.workbook, args.repo_root)
    if args.json:
        json.dump(report, sys.stdout, indent=2)
        print()
    else:
        _print_text(report, args.include_missing)
    return 0 if not report["runner_ids_not_in_workbook"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
