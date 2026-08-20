#!/usr/bin/env python3

import importlib.util
import pathlib
import sys
import unittest

SCRIPT = pathlib.Path(__file__).with_name("check_h2_advisory_exception.py")
_SPEC = importlib.util.spec_from_file_location("check_h2_advisory_exception", SCRIPT)
assert _SPEC and _SPEC.loader
GATE = importlib.util.module_from_spec(_SPEC)
sys.modules[_SPEC.name] = GATE
_SPEC.loader.exec_module(GATE)


class H2AdvisoryExceptionTests(unittest.TestCase):
    def test_allows_only_the_documented_legacy_and_patched_lines(self) -> None:
        self.assertEqual(
            GATE.validate(["0.3.27", "0.4.16"], {GATE.ADVISORY_ID}),
            [],
        )

    def test_rejects_another_vulnerable_version(self) -> None:
        errors = GATE.validate(["0.3.27", "0.4.15"], {GATE.ADVISORY_ID})
        self.assertTrue(any("0.4.15" in error for error in errors), errors)

    def test_requires_exception_removal_when_legacy_line_disappears(self) -> None:
        errors = GATE.validate(["0.4.16"], {GATE.ADVISORY_ID})
        self.assertTrue(any("remove" in error for error in errors), errors)

    def test_requires_exception_while_legacy_line_remains(self) -> None:
        errors = GATE.validate(["0.3.27", "0.4.16"], set())
        self.assertTrue(any("missing" in error for error in errors), errors)


if __name__ == "__main__":
    unittest.main()
