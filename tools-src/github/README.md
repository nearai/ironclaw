# GitHub Tool for IronClaw

WASM tool for GitHub integration. It covers repositories, issues, pull requests,
search, branches, file reads and writes, releases, and workflows.

## Features

- **Repositories** - Get repo details, list user repos, create repositories
- **Search** - Search repositories, code, and issues/PRs
- **Branches** - List branches and create new branches from an existing ref
- **Issues** - List/create/get issues, list/add issue comments
- **Pull Requests** - List/create/get PRs, review files, create reviews, list/reply review comments, merge PRs
- **File Content** - Read files and create/update/delete repository files
- **Releases** - List releases and create new releases
- **Workflows** - Trigger GitHub Actions, check run status

## Setup

Preferred: configure GitHub OAuth app credentials for browser auth:

1. Create a GitHub OAuth app at <https://github.com/settings/apps>
2. Set the callback URL to your IronClaw OAuth callback URL
3. Export:

   ```bash
   export GITHUB_OAUTH_CLIENT_ID=...
   export GITHUB_OAUTH_CLIENT_SECRET=...
   ```

4. Run:

   ```bash
   ironclaw tool auth github
   ```

IronClaw will open the browser OAuth flow and store the resulting `github_token`.

Fallback: use a Personal Access Token if you do not want to run an OAuth app:

1. Create a GitHub Personal Access Token at <https://github.com/settings/tokens>
2. Recommended scopes: `repo`, `workflow`, `read:org`
3. Store the token:

   ```
   ironclaw secret set github_token YOUR_TOKEN
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

### Create Repository

```json
{
  "action": "create_repo",
  "name": "infra-playground",
  "description": "Scratch repo for release automation",
  "private": true,
  "auto_init": true
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

### Search Code

```json
{
  "action": "search_code",
  "query": "repo:nearai/ironclaw tool_info",
  "limit": 5
}
```

### Search Issues and Pull Requests

```json
{
  "action": "search_issues_pull_requests",
  "query": "repo:nearai/ironclaw is:pr label:bug",
  "limit": 10
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

### Create or Update a File

```json
{
  "action": "create_or_update_file",
  "owner": "nearai",
  "repo": "ironclaw",
  "path": "docs/example.txt",
  "message": "docs: add example",
  "content": "Hello from IronClaw"
}
```

When updating an existing file, include the current blob `sha`.

### Delete a File

```json
{
  "action": "delete_file",
  "owner": "nearai",
  "repo": "ironclaw",
  "path": "docs/example.txt",
  "message": "docs: remove example",
  "sha": "0123456789abcdef0123456789abcdef01234567"
}
```

### List Branches

```json
{
  "action": "list_branches",
  "owner": "nearai",
  "repo": "ironclaw",
  "limit": 20
}
```

### Create Branch

```json
{
  "action": "create_branch",
  "owner": "nearai",
  "repo": "ironclaw",
  "branch": "feature/github-tool-audit",
  "from_ref": "main"
}
```

### List Releases

```json
{
  "action": "list_releases",
  "owner": "nearai",
  "repo": "ironclaw",
  "limit": 10
}
```

### Create Release

```json
{
  "action": "create_release",
  "owner": "nearai",
  "repo": "ironclaw",
  "tag_name": "v1.2.3",
  "name": "v1.2.3",
  "generate_release_notes": true
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
