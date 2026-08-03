Use `github.rerun_failed_workflow_run_jobs` to rerun only failed jobs in a GitHub Actions workflow run.

Provide `owner`, `repo`, and `run_id`. Set `enable_debug_logging` only when explicitly needed.

Use the exact JSON field names from this capability schema. If the user provides a GitHub URL, extract the owner and repo fields plus the schema-specific number, path, or ref key; for pull-request tools, use `pr_number`; for issue tools, use `issue_number`.

This capability performs an external write through the GitHub API using host HTTP egress. It requires approval and a configured GitHub product-auth account.
