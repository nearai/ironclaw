Use `github.get_issue` for read-only retrieval of one GitHub issue or pull request when the repository owner, repository name, and issue number are known.

Pass `owner`, `repo`, and `issue_number` exactly. If the user provides a GitHub issue or pull request URL, extract those three fields before calling this capability.

This capability reads from the GitHub API through the host HTTP egress port. It requires a configured `github_token` secret, but Extension Manifest v2 does not yet define a canonical credential declaration field, so the token requirement is documented here rather than declared in the manifest.
