Use `github.reply_pull_request_comment` to reply to a pull request review comment. Include the pull request number and review comment id.

Pass the required fields exactly as requested by the user. If the user provides a GitHub URL, extract the owner, repository, and number/path/ref fields before calling this capability.

This capability performs an external write through the GitHub API using host HTTP egress. It requires approval and a configured GitHub product-auth account.
