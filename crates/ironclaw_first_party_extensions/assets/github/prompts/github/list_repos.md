Use `github.list_repos` to list repositories visible to the authenticated GitHub user, or public repositories for a named user or organization.

For the current authenticated user, omit `username` or set it to `@me`. Do not guess a username for "my repos".

For another GitHub user or organization, set `username` to that exact login; this returns public repositories for that login.

Do not answer "who am I on GitHub?" from this capability. GitHub `/user/repos` returns repositories the authenticated user can access, including organization-owned repositories, so `owner.login` in these results is not proof of the authenticated account. Use `github.get_authenticated_user` for identity questions.

Use the exact JSON field names from this capability schema. If the user provides a GitHub URL, extract the owner and repo fields plus the schema-specific number, path, or ref key; for pull-request tools, use `pr_number`; for issue tools, use `issue_number`.

This capability reads from the GitHub API through host HTTP egress and requires a configured GitHub product-auth account.
