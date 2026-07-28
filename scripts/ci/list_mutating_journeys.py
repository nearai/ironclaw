#!/usr/bin/env python3
"""Print the mutating provider journeys, one id per line.

Used by the nightly "each journey passes alone" lane, which runs one pytest
process per id. That loop is only as good as this list: if it ever printed
nothing, the loop would iterate zero times, the step would exit 0, and the
proof would be retired without a single failing check. So an empty list is an
error here rather than an empty stdout for the caller to notice.
"""

from __future__ import annotations

import sys
from pathlib import Path

E2E_ROOT = Path(__file__).resolve().parents[2] / "tests/e2e"


def mutating_journey_ids() -> list[str]:
    sys.path.insert(0, str(E2E_ROOT))
    from journey_cases import PROVIDER_JOURNEY_CASES

    return [
        case.case_id
        for case in PROVIDER_JOURNEY_CASES
        if case.mutable_provider_worlds
    ]


def main() -> int:
    ids = mutating_journey_ids()
    if not ids:
        print(
            "no mutating provider journeys found; the alone-lane would test "
            "nothing and pass",
            file=sys.stderr,
        )
        return 1
    print("\n".join(ids))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
