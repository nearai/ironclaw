Use `github.search_issues` for read-only discovery of GitHub issues and pull requests.

Provide a focused GitHub search query in `query`. Include qualifiers such as `repo:owner/name`, `org:name`, `is:issue`, `is:pr`, `state:open`, labels, authors, assignees, or `involves:@me` when the user asks for a narrow result set.

This capability reads from the GitHub API through the host HTTP egress port. It requires a configured `github_token` runtime credential declared by the manifest.
