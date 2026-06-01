Use `github.handle_webhook` to normalize a GitHub webhook payload into system event intents.

Pass the required fields exactly as requested by the user. If the user provides a GitHub URL, extract the owner, repository, and number/path/ref fields before calling this capability.

This capability does not call GitHub; it normalizes a webhook payload already verified and supplied by the host.
