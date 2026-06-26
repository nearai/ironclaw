Use `github.list_pull_requests` to list pull requests in one repository.

Use `head`, `base`, `sort`, and `direction` when the user asks for branch-filtered or ordered pull request lists.

Use the exact JSON field names from this capability schema. If the user provides a GitHub URL, extract the owner and repo fields plus the schema-specific number, path, or ref key; for pull-request tools, use `pr_number`; for issue tools, use `issue_number`.

This capability reads from the GitHub API through host HTTP egress and requires a configured GitHub product-auth account.
