Use `github.get_pull_request_files` to list files changed in a pull request.

Use `page` and `limit` when a pull request changes many files; GitHub paginates this endpoint and omitting them returns only the first page.

Use the exact JSON field names from this capability schema. If the user provides a GitHub URL, extract the owner and repo fields plus the schema-specific number, path, or ref key; for pull-request tools, use `pr_number`; for issue tools, use `issue_number`.

This capability reads from the GitHub API through host HTTP egress and requires a configured GitHub product-auth account.
