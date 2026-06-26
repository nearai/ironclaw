Use `github.get_workflow_runs` to list GitHub Actions workflow runs.

Use filters such as `head_sha`, `branch`, `event`, `status`, `actor`, `created`, `check_suite_id`, and `exclude_pull_requests` to find the relevant run instead of paging broad results.

Use the exact JSON field names from this capability schema. If the user provides a GitHub URL, extract the owner and repo fields plus the schema-specific number, path, or ref key; for pull-request tools, use `pr_number`; for issue tools, use `issue_number`.

This capability reads from the GitHub API through host HTTP egress and requires a configured GitHub product-auth account.
