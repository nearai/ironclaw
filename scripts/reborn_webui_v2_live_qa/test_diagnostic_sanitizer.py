import unittest
from collections.abc import Iterator, Mapping

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

    def test_redacts_credential_suffix_keys_without_redacting_harmless_keys(self):
        value = {
            "webhook_secret": "secret-value",
            "session_token": "token-value",
            "encryption_key": "key-value",
            "browser_cookie": "cookie-value",
            "signingCredentials": "signing-value",
            "private_key": "private-value",
            "monkey": "harmless-monkey",
            "keyboard_layout": "harmless-layout",
            "token_count": 17,
            "cookie_policy": "strict",
        }

        sanitized = sanitize_diagnostic(value)

        for key in (
            "webhook_secret",
            "session_token",
            "encryption_key",
            "browser_cookie",
            "signingCredentials",
            "private_key",
        ):
            self.assertEqual(sanitized[key], "<REDACTED>")
        self.assertEqual(sanitized["monkey"], "harmless-monkey")
        self.assertEqual(sanitized["keyboard_layout"], "harmless-layout")
        self.assertEqual(sanitized["token_count"], 17)
        self.assertEqual(sanitized["cookie_policy"], "strict")

    def test_redacts_slack_file_ids(self):
        sanitized = str(sanitize_diagnostic("uploaded F0123456789 to Slack"))

        self.assertNotIn("F0123456789", sanitized)
        self.assertIn("REDACTED_SLACK_ID", sanitized)

    def test_redacts_suffix_secret_key_inside_json_diagnostic(self):
        sanitized = str(
            sanitize_diagnostic('{"webhook_secret":"do-not-leak","result":"ok"}')
        )

        self.assertNotIn("do-not-leak", sanitized)
        self.assertIn("REDACTED", sanitized)
        self.assertIn("ok", sanitized)

    def test_bounds_mapping_iterator_without_materializing_all_items(self):
        class GuardedMapping(Mapping[str, str]):
            def __len__(self) -> int:
                return 100

            def __getitem__(self, key: str) -> str:
                return f"value-{key}"

            def __iter__(self) -> Iterator[str]:
                for index in range(100):
                    if index >= 2:
                        raise AssertionError("mapping iterator was over-consumed")
                    yield f"key-{index}"

        sanitized = sanitize_diagnostic(GuardedMapping(), max_items=2)

        self.assertEqual(sanitized["key-0"], "value-key-0")
        self.assertEqual(sanitized["key-1"], "value-key-1")
        self.assertEqual(sanitized["<TRUNCATED>"], "98 item(s) omitted")


if __name__ == "__main__":
    unittest.main()
