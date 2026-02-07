#!/usr/bin/env python3
"""Update auto-managed metadata block in FEATURE_PARITY.md.

This script updates the section wrapped by:
  <!-- parity:auto:start -->
  <!-- parity:auto:end -->

If the block does not exist, it is inserted below the top-level title.
"""

from __future__ import annotations

import os
import re
from pathlib import Path


START_MARKER = "<!-- parity:auto:start -->"
END_MARKER = "<!-- parity:auto:end -->"


def build_auto_block(sync_sha: str, pr_number: str) -> str:
    return (
        f"{START_MARKER}\n"
        f"> Auto-synced for PR #{pr_number} at commit `{sync_sha}`.\n"
        "> Managed by `.github/workflows/update-feature-parity.yml`.\n"
        f"{END_MARKER}\n"
    )


def insert_block_after_title(content: str, block: str) -> str:
    lines = content.splitlines(keepends=True)
    title_index = None

    for idx, line in enumerate(lines):
        if line.startswith("# "):
            title_index = idx
            break

    if title_index is None:
        return f"{block}\n{content}" if content else block

    insert_at = title_index + 1
    if insert_at < len(lines) and lines[insert_at].strip() == "":
        insert_at += 1

    block_with_spacing = f"\n{block}\n"
    lines.insert(insert_at, block_with_spacing)
    return "".join(lines)


def update_content(content: str, sync_sha: str, pr_number: str) -> str:
    block = build_auto_block(sync_sha, pr_number)
    pattern = re.compile(
        re.escape(START_MARKER) + r".*?" + re.escape(END_MARKER) + r"\n?",
        re.DOTALL,
    )

    if pattern.search(content):
        return pattern.sub(block, content, count=1)

    return insert_block_after_title(content, block)


def main() -> int:
    parity_path = Path(os.getenv("FEATURE_PARITY_PATH", "FEATURE_PARITY.md"))
    sync_sha = os.getenv("PARITY_SYNC_SHA", "unknown")
    pr_number = os.getenv("PARITY_PR_NUMBER", "unknown")

    content = parity_path.read_text(encoding="utf-8")
    updated = update_content(content, sync_sha, pr_number)

    if updated != content:
        parity_path.write_text(updated, encoding="utf-8")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
