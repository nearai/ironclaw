Use `github.get_file_content` to fetch repository file content metadata/content.

Text file responses contain decoded UTF-8 in `content`. An `encoding` of `binary_unsupported` means the file is binary and `content` is intentionally absent. An `encoding` of `none` means GitHub did not include inline content for the file.

Use the exact JSON field names from this capability schema. If the user provides a GitHub URL, extract the owner and repo fields plus the schema-specific number, path, or ref key; for pull-request tools, use `pr_number`; for issue tools, use `issue_number`.

This capability reads from the GitHub API through host HTTP egress and requires a configured GitHub product-auth account.
