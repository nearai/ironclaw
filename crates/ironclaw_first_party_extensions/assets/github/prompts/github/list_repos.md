Use `github.list_repos` to list repositories visible to the authenticated GitHub account, or public repositories for a named user.

For the current authenticated account, omit `username` or set it to `@me`. Do not guess a username for "my repos".

For another GitHub user or organization, set `username` to that exact login; this returns public repositories for that login.

Use the exact JSON field names from this capability schema. If the user provides a GitHub URL, extract the owner and repo fields plus the schema-specific number, path, or ref key; for pull-request tools, use `pr_number`; for issue tools, use `issue_number`.

This capability reads from the GitHub API through host HTTP egress and requires a configured GitHub product-auth account.
