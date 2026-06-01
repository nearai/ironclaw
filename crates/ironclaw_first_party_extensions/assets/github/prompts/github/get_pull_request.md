Use `github.get_pull_request` to fetch one pull request.

Pass the required fields exactly as requested by the user. If the user provides a GitHub URL, extract the owner, repository, and number/path/ref fields before calling this capability.

This capability reads from the GitHub API through host HTTP egress and requires a configured GitHub product-auth account.
