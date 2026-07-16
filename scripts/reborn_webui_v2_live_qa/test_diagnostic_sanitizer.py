import unittest

from scripts.reborn_webui_v2_live_qa.diagnostic_sanitizer import sanitize_diagnostic


class DiagnosticSanitizerTests(unittest.TestCase):
    def test_redacts_escaped_json_secret_value_completely(self):
        secret = 'prefix\\"TOPSECRET'
        value = '{"token":"' + secret + '"}'

        sanitized = str(sanitize_diagnostic(value))

        self.assertNotIn("TOPSECRET", sanitized)
        self.assertNotIn("prefix", sanitized)
        self.assertIn("REDACTED", sanitized)

    def test_handles_cycles_and_bounds_depth_without_recursing(self):
        mapping: dict[str, object] = {}
        sequence: list[object] = [mapping]
        mapping["cycle"] = sequence
        mapping["deep"] = {"one": {"two": {"three": "still bounded"}}}

        sanitized = sanitize_diagnostic(mapping, max_depth=2)

        self.assertIn("CYCLE", str(sanitized))
        self.assertIn("MAX_DEPTH", str(sanitized))

    def test_redacts_bytes_keys_authorization_and_enterprise_slack_ids(self):
        value = {
            b"apiKey": b"TOPSECRET-API-KEY",
            b"authorization": b"Basic dXNlcjp0b3BzZWNyZXQ= trailing",
            "message": "enterprise E0123456789 user U0123456789 team T0123456789",
        }

        sanitized = str(sanitize_diagnostic(value))

        for leaked in (
            "TOPSECRET",
            "dXNlcjp0b3BzZWNyZXQ",
            "trailing",
            "E0123456789",
            "U0123456789",
            "T0123456789",
        ):
            self.assertNotIn(leaked, sanitized)
        self.assertIn("REDACTED", sanitized)

    def test_bounds_mapping_items_before_stringifying_values(self):
        class Explosive:
            def __str__(self):
                raise AssertionError("truncated value must not be stringified")

        value = {"first": "kept", "second": Explosive()}

        sanitized = sanitize_diagnostic(value, max_items=1)

        self.assertEqual(sanitized["first"], "kept")
        self.assertEqual(sanitized["<TRUNCATED>"], "1 item(s) omitted")


if __name__ == "__main__":
    unittest.main()
