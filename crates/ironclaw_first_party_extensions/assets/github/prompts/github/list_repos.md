Use `github.list_repos` to list repositories visible to the authenticated GitHub user.

Use `type` to control GitHub's repository affiliation filter. `all`, `owner`, `public`, `private`, and `member` are all valid on the authenticated `/user/repos` endpoint. Use `member` when the user needs organization or collaborator repositories visible to the authenticated account.

Do not answer "who am I on GitHub?" from this capability. GitHub `/user/repos` returns repositories the authenticated user can access, including organization-owned repositories, so `owner.login` in these results is not proof of the authenticated account. Use `github.get_authenticated_user` for identity questions.

Use the exact JSON field names from this capability schema. If the user provides a GitHub URL, extract the owner and repo fields plus the schema-specific number, path, or ref key; for pull-request tools, use `pr_number`; for issue tools, use `issue_number`.

This capability reads from the GitHub API through host HTTP egress and requires a configured GitHub product-auth account.
