Use `github.list_issue_comments` to list issue or pull request timeline comments.

Pass the required fields exactly as requested by the user. If the user provides a GitHub URL, extract the owner, repository, and number/path/ref fields before calling this capability.

This capability reads from the GitHub API through host HTTP egress and requires a configured GitHub product-auth account.
