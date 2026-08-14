Use `github.update_issue` to update an issue or pull request issue record.

Provide `owner`, `repo`, and `issue_number`. Include only the fields that should change: `title`, `body`, `state`, `milestone`, `labels`, or `assignees`.

Use `body: null` to clear an issue body.

Use the exact JSON field names from this capability schema. If the user provides a GitHub URL, extract the owner and repo fields plus the schema-specific number, path, or ref key; for pull-request tools, use `pr_number`; for issue tools, use `issue_number`.

This capability performs an external write through the GitHub API using host HTTP egress. It requires approval and a configured GitHub product-auth account.
