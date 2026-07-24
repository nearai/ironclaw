"""Deterministic coverage for the admin-configuration handle mapping.

The live-canary Slack bootstrap routes non-secret setup values to the
operator extension-configuration surface; these tests pin the pure mapping
(bare source names -> declared ``slack_*`` handles) without a live server.
"""

import unittest

from run_live_qa import LiveQaError, _map_admin_configuration_values

SLACK_DECLARED = {
    "slack_bot_token",
    "slack_signing_secret",
    "slack_team_id",
    "slack_api_app_id",
    "slack_installation_id",
    "slack_bot_user_id",
    "slack_shared_subject_user_id",
    "slack_oauth_client_id",
    "slack_oauth_client_secret",
}


class AdminHandleMappingTests(unittest.TestCase):
    def test_bare_slack_sources_map_onto_declared_handles(self):
        declared, unmapped = _map_admin_configuration_values(
            {
                "team_id": "T123",
                "api_app_id": "A456",
                "installation_id": "I789",
                "bot_user_id": "U000",
                "shared_subject_user_id": "U111",
                "oauth_client_id": "C222",
            },
            SLACK_DECLARED,
        )
        self.assertEqual(
            declared,
            {
                "slack_team_id": "T123",
                "slack_api_app_id": "A456",
                "slack_installation_id": "I789",
                "slack_bot_user_id": "U000",
                "slack_shared_subject_user_id": "U111",
                "slack_oauth_client_id": "C222",
            },
        )
        self.assertEqual(unmapped, [])

    def test_exact_handle_names_pass_through(self):
        declared, unmapped = _map_admin_configuration_values(
            {"slack_team_id": "T123"}, SLACK_DECLARED
        )
        self.assertEqual(declared, {"slack_team_id": "T123"})
        self.assertEqual(unmapped, [])

    def test_unmapped_sources_are_reported_not_submitted(self):
        declared, unmapped = _map_admin_configuration_values(
            {"team_id": "T123", "bot_username": "qa_bot", "auth_user_id": "U9"},
            SLACK_DECLARED,
        )
        self.assertEqual(declared, {"slack_team_id": "T123"})
        self.assertEqual(unmapped, ["auth_user_id", "bot_username"])

    def test_suffix_ambiguity_across_declared_handles_is_unmapped(self):
        # `user_id` suffix-matches both bot_user_id and shared_subject_user_id
        # declared handles -> ambiguous, never guessed.
        declared, unmapped = _map_admin_configuration_values(
            {"user_id": "U1"},
            {"slack_bot_user_id", "slack_shared_subject_user_id"},
        )
        self.assertEqual(declared, {})
        self.assertEqual(unmapped, ["user_id"])

    def test_duplicate_target_mapping_raises(self):
        with self.assertRaises(LiveQaError):
            _map_admin_configuration_values(
                {"foo_bar": "explicit", "bar": "suffixed"},
                {"foo_bar"},
            )


if __name__ == "__main__":
    unittest.main()
