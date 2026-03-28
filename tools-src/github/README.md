# GitHub Tool for IronClaw

WASM tool for GitHub integration - manage repos, issues, PRs, and workflows.

## Features

- **Repository Info** - Get repo details, list user repos
- **Issues** - List/create/get issues, list/add issue comments
- **Pull Requests** - List/create/get PRs, review files, create reviews, list/reply review comments, merge PRs
- **File Content** - Read files from repos
- **Workflows** - Trigger GitHub Actions, check run status

## Setup

1. Create a GitHub Personal Access Token at <https://github.com/settings/tokens>
2. Required scopes: `repo`, `workflow`, `read:org`
3. Store the token:

   ```
   ironclaw secret set github_token YOUR_TOKEN
   ```

For webhook delivery, you can either store the HMAC secret in IronClaw secrets:

```
ironclaw secret set github_webhook_secret YOUR_WEBHOOK_SECRET
```

or provide it via environment:

```bash
export GITHUB_WEBHOOK_SECRET=YOUR_WEBHOOK_SECRET
```

## Usage Examples

### Get Repository Info

```json
{
  "action": "get_repo",
  "owner": "nearai",
  "repo": "ironclaw"
}
```

### List Open Issues

```json
{
  "action": "list_issues",
  "owner": "nearai",
  "repo": "ironclaw",
  "state": "open",
  "limit": 10
}
```

### Create Issue

```json
{
  "action": "create_issue",
  "owner": "nearai",
  "repo": "ironclaw",
  "title": "Bug: Something is broken",
  "body": "Detailed description...",
  "labels": ["bug", "help wanted"]
}
```

### List Pull Requests

```json
{
  "action": "list_pull_requests",
  "owner": "nearai",
  "repo": "ironclaw",
  "state": "open",
  "limit": 5
}
```

### Review PR

```json
{
  "action": "create_pr_review",
  "owner": "nearai",
  "repo": "ironclaw",
  "pr_number": 42,
  "body": "LGTM! Great work.",
  "event": "APPROVE"
}
```

### Create Pull Request

```json
{
  "action": "create_pull_request",
  "owner": "nearai",
  "repo": "ironclaw",
  "title": "feat: add event-driven routines",
  "head": "feat/event-routines",
  "base": "main",
  "body": "Implements system_event trigger + event_emit tool."
}
```

### Merge Pull Request

```json
{
  "action": "merge_pull_request",
  "owner": "nearai",
  "repo": "ironclaw",
  "pr_number": 42,
  "merge_method": "squash"
}
```

### List Issue Comments

```json
{
  "action": "list_issue_comments",
  "owner": "nearai",
  "repo": "ironclaw",
  "issue_number": 42,
  "limit": 10
}
```

### Add Issue Comment

```json
{
  "action": "create_issue_comment",
  "owner": "nearai",
  "repo": "ironclaw",
  "issue_number": 42,
  "body": "Thanks for reporting this!"
}
```

### List PR Review Comments

```json
{
  "action": "list_pull_request_comments",
  "owner": "nearai",
  "repo": "ironclaw",
  "pr_number": 42,
  "limit": 30
}
```

### Reply to PR Review Comment

```json
{
  "action": "reply_pull_request_comment",
  "owner": "nearai",
  "repo": "ironclaw",
  "comment_id": 123456789,
  "body": "Fixed in the latest commit."
}
```

### Get PR Reviews

```json
{
  "action": "get_pull_request_reviews",
  "owner": "nearai",
  "repo": "ironclaw",
  "pr_number": 42
}
```

### Get Combined Status

```json
{
  "action": "get_combined_status",
  "owner": "nearai",
  "repo": "ironclaw",
  "ref": "main"
}
```

### Get File Content

```json
{
  "action": "get_file_content",
  "owner": "nearai",
  "repo": "ironclaw",
  "path": "README.md",
  "ref": "main"
}
```

### Trigger Workflow

```json
{
  "action": "trigger_workflow",
  "owner": "nearai",
  "repo": "ironclaw",
  "workflow_id": "ci.yml",
  "ref": "main",
  "inputs": {
    "environment": "staging"
  }
}
```

### Check Workflow Runs

```json
{
  "action": "get_workflow_runs",
  "owner": "nearai",
  "repo": "ironclaw",
  "limit": 5
}
```

### List Workflow Runs (Pagination)

```json
{
  "action": "get_workflow_runs",
  "owner": "nearai",
  "repo": "ironclaw",
  "limit": 5,
  "page": 2
}
```

## Error Handling

Errors are returned as strings in the `error` field of the response.

### Rate Limit Exceeded

When the GitHub API rate limit is exceeded (and retries fail), you might see:

```text
GitHub API error 429: { "message": "API rate limit exceeded for user ID ...", ... }
```

The tool automatically logs warnings when the rate limit is low (<10 remaining) and retries on 429/5xx errors.

### Invalid Parameters

```text
Invalid event: 'INVALID'. Must be one of: APPROVE, REQUEST_CHANGES, COMMENT
```

### Missing Token

```text
GitHub token not found in secret store. Set it with: ironclaw secret set github_token <token>...
```

## Troubleshooting

### "GitHub API error 404: Not Found"

- Check that the `owner` and `repo` are correct.
- Ensure the `github_token` has access to the repository (especially for private repos).
- Verify the token scopes include `repo` and `read:org`.

### "GitHub API error 401: Bad credentials"

- The token might be invalid or expired.
- Update the token: `ironclaw secret set github_token NEW_TOKEN`.

### Rate Limiting

- The tool logs a warning when remaining requests drop below 10.
- Check logs for "GitHub API rate limit low".
- If you hit the limit, wait for the reset time (usually 1 hour).

## Building

```bash
cd tools-src/github
cargo build --target wasm32-wasi --release
```

## License

MIT/Apache-2.0
