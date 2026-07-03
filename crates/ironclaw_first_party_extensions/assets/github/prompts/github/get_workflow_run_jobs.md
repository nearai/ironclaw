Use `github.get_workflow_run_jobs` to list jobs for a GitHub Actions workflow run.

Provide `owner`, `repo`, and `run_id`. Use `filter`, `limit`, and `page` to narrow results.

Use the exact JSON field names from this capability schema. If the user provides a GitHub URL, extract the owner and repo fields plus the schema-specific number, path, or ref key; for pull-request tools, use `pr_number`; for issue tools, use `issue_number`.

This capability reads from the GitHub API through host HTTP egress and requires a configured GitHub product-auth account.
