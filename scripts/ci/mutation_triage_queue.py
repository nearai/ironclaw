#!/usr/bin/env python3
"""Turn a cargo-mutants report into a self-contained triage queue.

Each surviving mutant becomes one entry carrying the sabotage diff and the
enclosing source, so whoever works the queue never has to go find the code.
That matters at scale: the queue is the unit of work, and an entry that
requires hunting is an entry that gets skipped or guessed at.

Verdict taxonomy is documented in docs/internal/mutation-audit.md. The
`needs-product-decision` verdict is load-bearing, not a cop-out: without it,
whoever works the queue will invent an intended contract and write a test
asserting it, which manufactures exactly the decorative coverage this audit
exists to find.
"""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path

MUTANT_LINE = re.compile(r"^(?P<file>[^:]+):(?P<line>\d+):(?P<col>\d+): (?P<what>.*)$")


def _enclosing_rust_item(source_path: Path, line_number: int) -> tuple[str, int]:
    """Source of the fn/impl containing `line_number`, plus its start line.

    Deliberately simple: scan back to the nearest column-0 or 4-space `fn`, then
    forward to the next one. A mutation report is read by a human or an agent,
    not parsed, so an approximate window beats a dependency on a Rust parser.
    """
    try:
        lines = source_path.read_text().splitlines()
    except OSError:
        return ("<source unavailable>", 0)

    start = 0
    opener = re.compile(r"^(\s{0,4})(pub(\([^)]*\))?\s+)?(async\s+)?(unsafe\s+)?fn\s")
    for index in range(min(line_number, len(lines)) - 1, -1, -1):
        if opener.match(lines[index]):
            start = index
            break

    end = len(lines)
    for index in range(start + 1, len(lines)):
        if opener.match(lines[index]):
            end = index
            break

    window = lines[start:end]
    # Keep entries readable; a very long function is a smell worth seeing, but
    # the queue should not become a source dump.
    if len(window) > 60:
        window = window[:60] + ["    // … truncated, see the file for the rest"]
    return ("\n".join(window), start + 1)


def _diff_for(report_dir: Path, outcomes: dict, mutant: str) -> str:
    """The sabotage cargo-mutants applied, as a diff, when it recorded one."""
    for outcome in outcomes.get("outcomes", []):
        scenario = outcome.get("scenario")
        if not isinstance(scenario, dict):
            continue
        described = scenario.get("Mutant", {})
        if not isinstance(described, dict):
            continue
        # `name` is already the exact string cargo-mutants writes to
        # missed.txt, so match on it rather than rebuilding it from parts.
        if str(described.get("name", "")).strip() != mutant.strip():
            continue
        diff_path = outcome.get("diff_path")
        if not diff_path:
            continue
        candidate = report_dir / diff_path
        if candidate.is_file():
            return candidate.read_text()
    return ""


def build_queue(report_dir: Path) -> str:
    missed_path = report_dir / "missed.txt"
    if not missed_path.is_file():
        raise SystemExit(f"no missed.txt in {report_dir} — was the audit run?")

    survivors = [line for line in missed_path.read_text().splitlines() if line.strip()]

    outcomes: dict = {}
    outcomes_path = report_dir / "outcomes.json"
    if outcomes_path.is_file():
        try:
            outcomes = json.loads(outcomes_path.read_text())
        except json.JSONDecodeError:
            outcomes = {}

    caught = len(
        [
            line
            for line in (report_dir / "caught.txt").read_text().splitlines()
            if line.strip()
        ]
    ) if (report_dir / "caught.txt").is_file() else 0
    unviable = len(
        [
            line
            for line in (report_dir / "unviable.txt").read_text().splitlines()
            if line.strip()
        ]
    ) if (report_dir / "unviable.txt").is_file() else 0

    viable = caught + len(survivors)
    out: list[str] = []
    out.append("# Mutation audit triage queue")
    out.append("")
    out.append(
        f"- **{len(survivors)} survivors** to triage "
        f"({caught} caught, {viable} viable, {unviable} unviable/ignored)"
    )
    if viable:
        out.append(f"- Caught rate over viable mutants: **{caught}/{viable}**")
    out.append(
        "- Score the caught rate over *viable* mutants only. Unviable mutants "
        "failed to compile and carry no signal."
    )
    out.append("")
    out.append(
        "Assign every entry exactly one verdict. See "
        "`docs/internal/mutation-audit.md`."
    )
    out.append("")
    out.append("| verdict | meaning | next step |")
    out.append("|---|---|---|")
    out.append(
        "| `real-gap` | behaviour genuinely unasserted | write a test, then "
        "`scripts/mutation-verify-fix.sh` must accept it |"
    )
    out.append(
        "| `equivalent-mutant` | no test *can* catch it — the change cannot "
        "alter observable behaviour | record the reasoning; no test |"
    )
    out.append(
        "| `needs-product-decision` | the intended contract is unclear | route "
        "to an owner; **do not invent one** |"
    )
    out.append("")
    out.append("---")
    out.append("")

    for index, mutant in enumerate(survivors, start=1):
        match = MUTANT_LINE.match(mutant)
        out.append(f"## {index}. `{mutant}`")
        out.append("")
        out.append("- verdict: `TODO`")
        out.append("- reasoning: _TODO_")
        out.append("")
        if match:
            source_path = Path(match.group("file"))
            body, start_line = _enclosing_rust_item(
                source_path, int(match.group("line"))
            )
            out.append(f"Enclosing item (`{source_path}:{start_line}`):")
            out.append("")
            out.append("```rust")
            out.append(body)
            out.append("```")
            out.append("")
        diff = _diff_for(report_dir, outcomes, mutant)
        if diff:
            out.append("Sabotage applied:")
            out.append("")
            out.append("```diff")
            out.append(diff.strip())
            out.append("```")
            out.append("")
        out.append("---")
        out.append("")

    if not survivors:
        out.append("No surviving mutants. Nothing to triage.")
        out.append("")

    return "\n".join(out)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report-dir", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    queue = build_queue(args.report_dir)
    args.output.write_text(queue)
    print(f"wrote {args.output}")


if __name__ == "__main__":
    main()
