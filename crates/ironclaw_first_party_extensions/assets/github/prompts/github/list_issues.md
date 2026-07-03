Use `github.list_issues` to list issues in one repository.

Use `state`, `labels`, `assignee`, `milestone`, `page`, and `limit` to narrow repository issues through GitHub's native repository issues endpoint. For `milestone`, pass the milestone number, `none`, or `*`; this capability does not resolve milestone titles.

Use the exact JSON field names from this capability schema. If the user provides a GitHub URL, extract the owner and repo fields plus the schema-specific number, path, or ref key; for pull-request tools, use `pr_number`; for issue tools, use `issue_number`.

This capability reads from the GitHub API through host HTTP egress and requires a configured GitHub product-auth account.
