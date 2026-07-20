#!/usr/bin/env python3
"""Audit WebUI v2 and OpenAI-compatible surfaces against the QA workbook."""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from zipfile import ZipFile

import report_coverage

ROOT = Path(__file__).resolve().parents[2]


@dataclass(frozen=True)
class Surface:
    kind: str
    identifier: str
    source: str
    keywords: tuple[str, ...]


def _read(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def _workbook_feature_text(workbook_path: Path) -> str:
    with ZipFile(workbook_path) as xlsx:
        rows = report_coverage.sheet_rows(xlsx, "Feature Inventory")
    return "\n".join(" ".join(cell for cell in row if cell) for row in rows[1:]).lower()


def _route_keywords(path: str) -> tuple[str, ...]:
    normalized = path.lower().strip("/")
    if not normalized:
        return ("chat",)
    first = normalized.split("/", 1)[0].replace(":threadid", "chat")
    aliases = {
        "welcome": ("onboarding", "provider"),
        "chat": ("chat",),
        "workspace": ("workspace",),
        "projects": ("projects",),
        "missions": ("missions",),
        "jobs": ("jobs",),
        "routines": ("routines",),
        "automations": ("automations",),
        "extensions": ("extensions",),
        "logs": ("logs",),
        "settings": ("settings",),
        "admin": ("admin",),
        "login": ("login",),
    }
    return aliases.get(first, (first,))


def browser_routes(repo_root: Path) -> list[Surface]:
    app_js = repo_root / "crates/ironclaw_webui/frontend/src/app/app.tsx"
    route_re = re.compile(r"<Route\s+path=\"([^\"]+)\"")
    surfaces: list[Surface] = []
    for route in route_re.findall(_read(app_js)):
        if route in {"*", "/"}:
            continue
        surfaces.append(
            Surface(
                kind="webui_browser_route",
                identifier=f"/{route.lstrip('/')}",
                source=str(app_js.relative_to(repo_root)),
                keywords=_route_keywords(route),
            )
        )
    if not surfaces:
        raise RuntimeError(f"no WebUI browser routes extracted from {app_js}")
    return surfaces


def _descriptor_patterns(path: Path, *, kind: str, repo_root: Path) -> list[Surface]:
    constant_re = re.compile(
        r"pub const (?P<name>[A-Z0-9_]+):\s*&str\s*=\s*\"(?P<pattern>[^\"]+)\""
    )
    surfaces: list[Surface] = []
    for match in constant_re.finditer(_read(path)):
        name = match.group("name")
        pattern = match.group("pattern")
        if "PATTERN" not in name:
            continue
        surfaces.append(
            Surface(
                kind=kind,
                identifier=pattern,
                source=str(path.relative_to(repo_root)),
                keywords=_api_keywords(name, pattern),
            )
        )
    if not surfaces:
        raise RuntimeError(f"no {kind} descriptors extracted from {path}")
    return surfaces


def _api_keywords(name: str, pattern: str) -> tuple[str, ...]:
    lower_name = name.lower()
    lower_pattern = pattern.lower()
    if "responses" in lower_name or "/responses" in lower_pattern:
        return ("responses",)
    if "chat_completions" in lower_name or "chat/completions" in lower_pattern:
        return ("chat completions", "openai")
    if "models" in lower_name or lower_pattern.endswith("/models"):
        return ("models", "openai")
    if "thread" in lower_name or "/threads" in lower_pattern:
        if "file" in lower_name or "/files" in lower_pattern:
            return ("workspace", "filesystem")
        return ("thread", "message")
    if "project" in lower_name or "/projects" in lower_pattern:
        return ("project",)
    if "fs_" in lower_name or "/fs/" in lower_pattern:
        return ("filesystem", "workspace")
    if "automation" in lower_name or "/automations" in lower_pattern:
        return ("automations",)
    if "trace" in lower_name or "/traces" in lower_pattern:
        return ("trace",)
    if "outbound" in lower_name or "/outbound" in lower_pattern:
        return ("outbound",)
    if "channel" in lower_name or "/channels" in lower_pattern:
        return ("channel",)
    if "extension" in lower_name or "/extensions" in lower_pattern:
        return ("extension",)
    if "skill" in lower_name or "/skills" in lower_pattern:
        return ("skill",)
    if "settings" in lower_name or "/settings" in lower_pattern:
        return ("settings",)
    if "llm" in lower_name or "/llm/" in lower_pattern:
        return ("llm", "provider")
    if "operator" in lower_name or "/operator" in lower_pattern:
        return ("operator",)
    if "sso" in lower_name or "oauth" in lower_name:
        return ("sso", "oauth")
    return tuple(part for part in re.split(r"[_/\-{}]+", lower_name) if part)


def api_surfaces(repo_root: Path) -> list[Surface]:
    return [
        *_descriptor_patterns(
            repo_root / "crates/ironclaw_webui/src/webui_v2/descriptors.rs",
            kind="webui_api_pattern",
            repo_root=repo_root,
        ),
        *_descriptor_patterns(
            repo_root / "crates/ironclaw_reborn_openai_compat/src/descriptors.rs",
            kind="openai_compat_api_pattern",
            repo_root=repo_root,
        ),
    ]


def build_audit(workbook_path: Path, repo_root: Path = ROOT) -> dict[str, object]:
    feature_text = _workbook_feature_text(workbook_path)
    surfaces = [*browser_routes(repo_root), *api_surfaces(repo_root)]
    uncovered = [
        surface
        for surface in surfaces
        if not any(keyword.lower() in feature_text for keyword in surface.keywords)
    ]
    by_kind: dict[str, int] = {}
    for surface in surfaces:
        by_kind[surface.kind] = by_kind.get(surface.kind, 0) + 1
    return {
        "workbook": str(workbook_path),
        "surface_count": len(surfaces),
        "surface_count_by_kind": by_kind,
        "uncovered_surface_count": len(uncovered),
        "uncovered_surfaces": [
            {
                "kind": surface.kind,
                "identifier": surface.identifier,
                "source": surface.source,
                "keywords": list(surface.keywords),
            }
            for surface in uncovered
        ],
    }


def print_report(report: dict[str, object]) -> None:
    print(f"Workbook: {report['workbook']}")
    print(f"Surface count: {report['surface_count']}")
    for kind, count in sorted(report["surface_count_by_kind"].items()):
        print(f"  {kind}: {count}")
    print(f"Uncovered surfaces: {report['uncovered_surface_count']}")
    for surface in report["uncovered_surfaces"]:
        print(
            f"- {surface['kind']} {surface['identifier']} "
            f"keywords={','.join(surface['keywords'])} source={surface['source']}"
        )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workbook", type=Path, required=True)
    parser.add_argument("--repo-root", type=Path, default=ROOT)
    parser.add_argument("--json", action="store_true")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    report = build_audit(args.workbook, args.repo_root)
    if args.json:
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        print_report(report)
    return 0 if report["uncovered_surface_count"] == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
