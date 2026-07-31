Use `github.search_issues_pull_requests` to search GitHub issues and pull requests.

The `sort` field accepts GitHub issue-search sorts including `comments`, `created`, `updated`, reaction sorts, and `interactions`.

The result keeps GitHub's `total_count`, `incomplete_results`, and `items` search envelope while returning compact item summaries with the repository URL, number, title, type marker, state/draft status, URL, author, labels, assignees, milestone, comment count, timestamps, and score. Use `page` and `limit` to continue through results. For bodies or other full detail, call `github.get_pull_request` for pull request items or `github.get_issue` for issue items.

Use the exact JSON field names from this capability schema. If the user provides a GitHub URL, extract the owner and repo fields plus the schema-specific number, path, or ref key; for pull-request tools, use `pr_number`; for issue tools, use `issue_number`.

This capability reads from the GitHub API through host HTTP egress and requires a configured GitHub product-auth account.
