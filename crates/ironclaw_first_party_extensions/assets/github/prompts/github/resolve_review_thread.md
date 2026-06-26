Use `github.resolve_review_thread` to mark an inline pull request review thread as resolved.

Use the exact JSON field names from this capability schema. This schema only accepts `thread_id`.

Use the GraphQL pull request review thread id from `github.list_pull_request_review_threads`; ask for it if unavailable.

This capability performs an external write through the GitHub GraphQL API using host HTTP egress. It requires approval and a configured GitHub product-auth account.
