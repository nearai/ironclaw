Use `github.comment_issue` to add a Markdown comment to an existing GitHub issue or pull request when the repository owner, repository name, issue number, and exact comment body are known.

Pass `owner`, `repo`, `issue_number`, and `body` exactly. If the user provides a GitHub issue or pull request URL, extract the owner, repository, and number before calling this capability.

This capability performs an external write through the GitHub API using the host HTTP egress port. It requires approval before dispatch and requires a configured `github_token` runtime credential declared by the manifest.
