Use `github.search_repositories` to search repositories.

The response preserves GitHub's `total_count`, `incomplete_results`, and `items` envelope. Each item is a compact repository summary with `id`, `node_id`, `name`, `full_name`, `private`, `fork`, `archived`, `visibility`, `html_url`, `description`, `language`, `default_branch`, `stargazers_count`, `open_issues_count`, `updated_at`, `pushed_at`, `owner.login`, `license.spdx_id`, and `permissions.{admin,push,pull}`. Use `github.get_repo` when omitted provider fields or full repository details are needed.

Use the exact JSON field names from this capability schema. If the user provides a GitHub URL, extract the owner and repo fields plus the schema-specific number, path, or ref key; for pull-request tools, use `pr_number`; for issue tools, use `issue_number`.

This capability reads from the GitHub API through host HTTP egress and requires a configured GitHub product-auth account.
