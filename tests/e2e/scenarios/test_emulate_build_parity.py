"""Local runs must not quietly cover less than CI (#6524 workstream 1).

CI builds the pinned `serrrfirat/emulate` fork and points
`IRONCLAW_EMULATE_CLI` at it. Without that variable `conftest.py` falls back
to the `emulate@0.7.0` npm package, which is not the same build: measured on
`test_emulate_reborn_provider_contracts.py` against unmodified main, the
fallback gave 1 failed / 10 passed / 3 skipped where the pinned fork gave 14
passed.

So a local green could mean "14 checks passed" or "10 passed and 3 quietly
didn't run", with nothing distinguishing them. These tests pin the two things
that keep that honest: the run says which build it used, and a missing
capability is a failure on the build that is supposed to have it.
"""

from __future__ import annotations

import conftest
import pytest
from _pytest.outcomes import Failed, Skipped


def test_report_header_names_the_emulate_build():
    header = conftest.pytest_report_header()
    assert header.startswith("emulate: ")
    assert conftest.emulate_build_label() in header


def test_fallback_label_says_it_is_not_what_ci_runs(monkeypatch):
    """The label has to be readable as a warning, not trivia."""
    monkeypatch.setattr(conftest, "EMULATE_CLI_PATH", None)
    label = conftest.emulate_build_label()
    assert conftest.EMULATE_NPM_PACKAGE in label
    assert "NOT what CI runs" in label


def test_pinned_label_names_the_cli_path(monkeypatch):
    monkeypatch.setattr(conftest, "EMULATE_CLI_PATH", "/somewhere/dist/index.js")
    assert "/somewhere/dist/index.js" in conftest.emulate_build_label()


def test_missing_capability_fails_on_the_pinned_build(monkeypatch):
    """The pinned fork is expected to have these endpoints.

    A skip here would report a capability regression in the build CI depends
    on as "skipped", which is how a coverage loss survives a green run.
    """
    monkeypatch.setattr(conftest, "EMULATE_CLI_PATH", "/somewhere/dist/index.js")
    with pytest.raises(Failed, match="capability regression"):
        conftest.skip_if_emulate_capability_absent("Docs API missing")


def test_missing_capability_only_skips_on_the_npm_fallback(monkeypatch):
    """On the fallback the endpoint genuinely is absent, so a skip is honest."""
    monkeypatch.setattr(conftest, "EMULATE_CLI_PATH", None)
    with pytest.raises(Skipped):
        conftest.skip_if_emulate_capability_absent("Docs API missing")
