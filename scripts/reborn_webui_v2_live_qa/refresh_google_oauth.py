#!/usr/bin/env python3
"""Refresh the live QA Google access token without exposing token material."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import stat
import sys
import urllib.error
import urllib.parse
import urllib.request
from typing import Optional, Tuple


TOKEN_URL = "https://oauth2.googleapis.com/token"


def _secret(name: str) -> str:
    path = os.environ.get(f"{name}_PATH", "").strip()
    if path:
        return Path(path).read_text(encoding="utf-8").strip()
    return os.environ.get(name, "").strip()


def refresh_access_token() -> Tuple[Optional[str], str]:
    client_id = _secret("IRONCLAW_REBORN_GOOGLE_CLIENT_ID")
    client_secret = _secret("IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET")
    refresh_token = _secret("AUTH_LIVE_GOOGLE_REFRESH_TOKEN")
    missing = [
        name
        for name, value in (
            ("IRONCLAW_REBORN_GOOGLE_CLIENT_ID", client_id),
            ("IRONCLAW_REBORN_GOOGLE_CLIENT_SECRET", client_secret),
            ("AUTH_LIVE_GOOGLE_REFRESH_TOKEN", refresh_token),
        )
        if not value
    ]
    if missing:
        return None, f"missing:{','.join(missing)}"

    request = urllib.request.Request(
        TOKEN_URL,
        data=urllib.parse.urlencode(
            {
                "client_id": client_id,
                "client_secret": client_secret,
                "refresh_token": refresh_token,
                "grant_type": "refresh_token",
            }
        ).encode(),
        headers={"Content-Type": "application/x-www-form-urlencoded"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=20) as response:
            payload = json.load(response)
    except urllib.error.HTTPError as exc:
        try:
            payload = json.loads(exc.read().decode("utf-8", errors="replace"))
        except (json.JSONDecodeError, UnicodeDecodeError):
            payload = {}
        return None, str(payload.get("error") or f"http_{exc.code}")
    except (OSError, urllib.error.URLError) as exc:
        return None, f"network:{type(exc).__name__}"

    access_token = str(payload.get("access_token") or "").strip()
    if not access_token:
        return None, "missing_access_token"
    return access_token, "healthy"


def _write_output(path: Path, value: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(value, encoding="utf-8")
    path.chmod(stat.S_IRUSR | stat.S_IWUSR)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--access-token-path", type=Path)
    parser.add_argument("--github-output", type=Path)
    args = parser.parse_args()

    access_token, status = refresh_access_token()
    if args.github_output:
        with args.github_output.open("a", encoding="utf-8") as output:
            output.write(f"status={status}\n")
    if access_token and args.access_token_path:
        _write_output(args.access_token_path, access_token)

    if access_token:
        print("Google OAuth refresh succeeded; minted a fresh access token.")
        return 0
    print(f"Google OAuth refresh failed: {status}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
