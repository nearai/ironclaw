"""Simple file statistics helper."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path


def count_file_stats(file_path):
    """Return line, word, and character counts for a text file.

    If the file cannot be read because it does not exist or permissions deny
    access, the returned dictionary includes an ``error`` key with details.
    """
    path = Path(file_path)

    try:
        text = path.read_text(encoding="utf-8")
    except FileNotFoundError as exc:
        return {
            "lines": 0,
            "words": 0,
            "characters": 0,
            "error": f"File not found: {exc.filename}",
        }
    except PermissionError as exc:
        return {
            "lines": 0,
            "words": 0,
            "characters": 0,
            "error": f"Permission denied: {exc.filename}",
        }

    return {
        "lines": len(text.splitlines()),
        "words": len(text.split()),
        "characters": len(text),
    }


def _main():
    parser = argparse.ArgumentParser(
        description="Count lines, words, and characters in a text file."
    )
    parser.add_argument("file_path", help="Path to the text file to inspect.")
    args = parser.parse_args()

    stats = count_file_stats(args.file_path)
    for key, value in stats.items():
        print(f"{key}: {value}")

    return 1 if "error" in stats else 0


if __name__ == "__main__":
    sys.exit(_main())
