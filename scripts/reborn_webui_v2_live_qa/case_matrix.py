"""Static case metadata for the Reborn WebUI v2 live QA lane."""

from __future__ import annotations

import re
from collections.abc import Awaitable, Callable
from typing import Any

QA_SHEET_URL = (
    "https://docs.google.com/spreadsheets/d/"
    "1IpioaRFnDw8cW4fj9vxg1pBRWN7swVQLRq1FqVlJAls/edit?gid=0#gid=0"
)
QA_SHEET_TAB = "Automated"

CaseFn = Callable[[Any], Awaitable[Any]]


class CaseSpec:
    def __init__(
        self,
        fn: CaseFn,
        *,
        requires_slack: bool = False,
        requires_slack_target: bool = False,
        requires_google_product_auth: bool = False,
        requires_google_runtime_access: bool = False,
        requires_telegram: bool = False,
        requires_github_auth: bool = False,
        default_enabled: bool = True,
        implemented: bool = True,
    ) -> None:
        self.fn = fn
        self.requires_slack = requires_slack
        self.requires_slack_target = requires_slack_target
        self.requires_google_product_auth = requires_google_product_auth
        self.requires_google_runtime_access = requires_google_runtime_access
        self.requires_telegram = requires_telegram
        self.requires_github_auth = requires_github_auth
        self.default_enabled = default_enabled
        self.implemented = implemented


def qa_row_sort_key(row_id: str) -> tuple[int, str]:
    match = re.match(r"^(\d+)([A-Z]+)$", row_id)
    if not match:
        return (9999, row_id)
    return (int(match.group(1)), match.group(2))


QA_SHEET_CASES: dict[str, dict[str, object]] = {
    "qa_1a_telegram_connect": {
        "rows": ["1A"],
        "feature": "Telegram connection flow",
        "gate": "requires live Telegram bot/user credentials and OAuth/pairing automation",
    },
    "qa_1b_telegram_near_news_chat": {
        "rows": ["1B"],
        "feature": "Telegram NEAR AI news summary delivery",
        "gate": "requires live Telegram connection and live Twitter/X or web search access",
    },
    "qa_1c_telegram_near_news_routine": {
        "rows": ["1C"],
        "feature": "Scheduled Telegram NEAR AI news digest routine",
        "gate": "requires live Telegram connection and routine delivery verification",
    },
    "qa_2a_gmail_connect": {
        "rows": ["2A"],
        "feature": "Gmail connection flow",
        "gate": "requires live Google browser consent state or OAuth test account",
    },
    "qa_2b_calendar_connect": {
        "rows": ["2B"],
        "feature": "Google Calendar connection flow",
        "gate": "requires live Google browser consent state or OAuth test account",
    },
    "qa_2c_drive_connect": {
        "rows": ["2C"],
        "feature": "Google Drive connection flow",
        "gate": "requires live Google browser consent state or OAuth test account",
    },
    "qa_2d_calendar_prep_live_chat": {
        "rows": ["2D"],
        "feature": "Calendar prep assistant using Google Docs and live news",
        "gate": (
            "requires a live Google OAuth account authorized for Calendar, Drive, "
            "Docs, and web/search runtime execution, plus Google OAuth refresh "
            "env when the copied access token is expired"
        ),
    },
    "qa_2e_calendar_prep_email_routine": {
        "rows": ["2E"],
        "feature": "Scheduled meeting-prep email routine",
        "gate": "requires live Gmail, Calendar, Drive, Docs, and routine verification",
    },
    "qa_2f_calendar_prep_email_delivery": {
        "rows": ["2F"],
        "feature": "Meeting-prep email side-effect delivery",
        "gate": "requires live Gmail inbox delivery verification",
    },
    "qa_3a_slack_connect": {
        "rows": ["3A"],
        "feature": "Slack connection flow",
        "gate": "requires live Slack OAuth or host-beta Slack bot/signing-secret env",
    },
    "qa_3b_endpoint_status_live_chat": {
        "rows": ["3B"],
        "feature": "Deployment health watcher endpoint status check",
    },
    "qa_3c_endpoint_status_slack_routine": {
        "rows": ["3C"],
        "feature": "Deployment health watcher Slack routine creation",
        "gate": "requires live Slack host-beta bot/signing-secret env",
    },
    "qa_3d_endpoint_status_slack_delivery": {
        "rows": ["3D"],
        "feature": "Deployment health watcher Slack delivery",
        "gate": "requires live Slack message delivery verification",
    },
    "qa_4a_gmail_connect": {
        "rows": ["4A"],
        "feature": "Gmail connection flow for release tracker",
        "gate": "requires live Google browser consent state or OAuth test account",
    },
    "qa_4b_github_connect": {
        "rows": ["4B"],
        "feature": "GitHub connection flow",
        "gate": "requires live GitHub PAT/auth state",
    },
    "qa_4c_github_release_live_chat": {
        "rows": ["4C"],
        "feature": "GitHub release tracker summary",
    },
    "qa_4d_github_release_slack_routine": {
        "rows": ["4D"],
        "feature": "Scheduled GitHub release summary routine",
        "gate": "requires live Slack delivery target and routine verification",
    },
    "qa_4e_github_release_email_delivery": {
        "rows": ["4E"],
        "feature": "GitHub release summary email delivery",
        "gate": "requires live Gmail delivery verification and a new release/change trigger",
    },
    "qa_5a_slack_connect": {
        "rows": ["5A"],
        "feature": "Slack connection flow for AMA",
        "gate": "requires live Slack OAuth or host-beta Slack bot/signing-secret env",
    },
    "qa_5b_drive_connect": {
        "rows": ["5B"],
        "feature": "Google Drive connection flow for AMA",
        "gate": "requires live Google browser consent state or OAuth test account",
    },
    "qa_5c_strategy_doc_knowledge_base": {
        "rows": ["5C"],
        "feature": "Google Drive strategy document grounding",
        "gate": (
            "requires a live Google OAuth account authorized for Google Docs/Drive "
            "runtime execution, plus Google OAuth refresh env when the copied "
            "access token is expired"
        ),
    },
    "qa_5d_slack_strategy_doc_answer": {
        "rows": ["5D"],
        "feature": "Slack AMA answer grounded in Google Drive document",
        "gate": "requires live Slack and Google Drive side-effect verification",
    },
    "qa_6a_gmail_connect": {
        "rows": ["6A"],
        "feature": "Gmail connection flow for CRM tracker",
        "gate": "requires live Google browser consent state or OAuth test account",
    },
    "qa_6b_sheets_connect": {
        "rows": ["6B"],
        "feature": "Google Sheets connection flow",
        "gate": "requires live Google browser consent state or OAuth test account",
    },
    "qa_6c_gmail_to_sheet_live_chat": {
        "rows": ["6C"],
        "feature": "CRM inbound email extraction to Google Sheet",
        "gate": (
            "requires a live Google OAuth account authorized for Gmail, Google "
            "Drive name lookup, and Google Sheets runtime execution plus test "
            "data, and Google OAuth refresh env when the copied access token is "
            "expired"
        ),
    },
    "qa_6d_gmail_to_sheet_routine": {
        "rows": ["6D"],
        "feature": "Scheduled CRM inbound email tracker routine",
        "gate": "requires live Gmail and Google Sheets routine verification",
    },
    "qa_6e_gmail_to_sheet_delivery": {
        "rows": ["6E"],
        "feature": "CRM inbound email row side effect",
        "gate": "requires live Gmail inbox and Google Sheets row verification",
    },
    "qa_7a_slack_product_channel_connect": {
        "rows": ["7A"],
        "feature": "Slack product channel connection flow",
        "gate": "requires live Slack OAuth/channel setup",
    },
    "qa_7b_sheets_connect": {
        "rows": ["7B"],
        "feature": "Google Sheets connection flow for bug logger",
        "gate": "requires live Google browser consent state or OAuth test account",
    },
    "qa_7c_slack_bug_logger_routine": {
        "rows": ["7C"],
        "feature": "Slack bug-message to Google Sheet routine creation",
        "gate": "requires live Slack and Google Sheets routine verification",
    },
    "qa_7d_slack_bug_message_trigger": {
        "rows": ["7D"],
        "feature": "Slack bug-message trigger",
        "gate": "requires live Slack message injection",
    },
    "qa_7e_slack_bug_sheet_delivery": {
        "rows": ["7E"],
        "feature": "Slack bug-message row side effect",
        "gate": "requires live Slack and Google Sheets row verification",
    },
    "qa_8a_slack_connect": {
        "rows": ["8A"],
        "feature": "Slack connection flow for HN monitor",
        "gate": "requires live Slack OAuth or host-beta Slack bot/signing-secret env",
    },
    "qa_8b_hn_keyword_live_chat": {
        "rows": ["8B"],
        "feature": "Hacker News keyword monitor search",
    },
    "qa_8c_hn_keyword_slack_routine": {
        "rows": ["8C"],
        "feature": "Hacker News keyword monitor Slack routine creation",
        "gate": "requires live Slack host-beta bot/signing-secret env",
    },
    "qa_8d_hn_keyword_slack_delivery": {
        "rows": ["8D"],
        "feature": "Hacker News keyword monitor Slack delivery",
        "gate": "requires live Slack message delivery verification",
    },
}
