Use `github.create_pull_request` to create a pull request.

Use `head_repo` for same-organization cross-repository pull requests, `issue` to convert an existing issue into a pull request, and `maintainer_can_modify` when the user asks whether maintainers can push to the head branch.

Use the exact JSON field names from this capability schema. If the user provides a GitHub URL, extract the owner and repo fields plus the schema-specific number, path, or ref key; for pull-request tools, use `pr_number`; for issue tools, use `issue_number`.

This capability performs an external write through the GitHub API using host HTTP egress. It requires approval and a configured GitHub product-auth account.
