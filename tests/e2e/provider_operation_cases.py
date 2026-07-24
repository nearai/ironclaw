"""Registry for typed full-path provider operation cases."""

from provider_operation_github_issue_cases import (
    GITHUB_ISSUE_PROVIDER_OPERATION_CASES,
)
from provider_operation_github_actions_cases import (
    GITHUB_ACTIONS_PROVIDER_OPERATION_CASES,
)
from provider_operation_github_pull_cases import (
    GITHUB_PULL_PROVIDER_OPERATION_CASES,
)
from provider_operation_github_repo_cases import (
    GITHUB_REPO_PROVIDER_OPERATION_CASES,
)
from provider_operation_gmail_cases import GMAIL_PROVIDER_OPERATION_CASES
from provider_operation_google_calendar_cases import (
    GOOGLE_CALENDAR_PROVIDER_OPERATION_CASES,
)
from provider_operation_google_docs_cases import GOOGLE_DOCS_PROVIDER_OPERATION_CASES
from provider_operation_google_drive_cases import GOOGLE_DRIVE_PROVIDER_OPERATION_CASES
from provider_operation_google_sheets_cases import GOOGLE_SHEETS_PROVIDER_OPERATION_CASES
from provider_operation_google_slides_cases import GOOGLE_SLIDES_PROVIDER_OPERATION_CASES

PROVIDER_OPERATION_CASES = (
    *GMAIL_PROVIDER_OPERATION_CASES,
    *GOOGLE_CALENDAR_PROVIDER_OPERATION_CASES,
    *GOOGLE_DOCS_PROVIDER_OPERATION_CASES,
    *GOOGLE_DRIVE_PROVIDER_OPERATION_CASES,
    *GOOGLE_SHEETS_PROVIDER_OPERATION_CASES,
    *GOOGLE_SLIDES_PROVIDER_OPERATION_CASES,
    *GITHUB_ACTIONS_PROVIDER_OPERATION_CASES,
    *GITHUB_ISSUE_PROVIDER_OPERATION_CASES,
    *GITHUB_PULL_PROVIDER_OPERATION_CASES,
    *GITHUB_REPO_PROVIDER_OPERATION_CASES,
)
