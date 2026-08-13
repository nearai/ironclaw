Use `github.rerun_workflow_job` to rerun one GitHub Actions workflow job.

Provide `owner`, `repo`, and `job_id`. Set `enable_debug_logging` or `enable_debugger` only when explicitly needed.

Use the exact JSON field names from this capability schema. If the user provides a GitHub URL, extract the owner and repo fields plus the schema-specific number, path, or ref key; for pull-request tools, use `pr_number`; for issue tools, use `issue_number`.

This capability performs an external write through the GitHub API using host HTTP egress. It requires approval and a configured GitHub product-auth account.
