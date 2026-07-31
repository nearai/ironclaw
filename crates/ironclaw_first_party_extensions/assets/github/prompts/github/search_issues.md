Use `github.search_issues` for read-only discovery of GitHub issues and pull requests.

Provide a focused GitHub search query in `query`. Include qualifiers such as `repo:owner/name`, `org:name`, `is:issue`, `is:pr`, `state:open`, labels, authors, assignees, or `involves:@me` when the user asks for a narrow result set.

The result uses the same compact `total_count`, `incomplete_results`, and `items` envelope as `github.search_issues_pull_requests`. Use `page` and `limit` for pagination, then use `github.get_pull_request` or `github.get_issue` when full detail is needed for one result.

This capability reads from the GitHub API through host HTTP egress and requires a configured GitHub product-auth account.
