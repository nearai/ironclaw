Use `github.handle_webhook` to normalize a GitHub webhook payload into system event intents.

Use the exact JSON field names from this capability schema. If the user provides a GitHub URL, extract the owner and repo fields plus the schema-specific number, path, or ref key; for pull-request tools, use `pr_number`; for issue tools, use `issue_number`.

This capability does not call GitHub; it normalizes a webhook payload already verified and supplied by the host.
