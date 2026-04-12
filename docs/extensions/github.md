---
title: "Github"
description: "Let your agent access Github"
---

The Github extension allows your agent to interact with Github repositories, issues, pull requests, and more, making it ideal for automating code-related tasks, managing projects, or gathering information from Github.

---

## Setup


<Steps>

<Step title="Install the Web Search Extension">

To install the GitHub extension, run:

```bash
ironclaw registry install github
```

</Step>

<Step title="Configure Browser OAuth (Preferred)">

Create a GitHub OAuth app at [github.com/settings/apps](https://github.com/settings/apps)
and set its callback URL to the IronClaw OAuth callback URL your gateway uses.

Then expose the app credentials to IronClaw:

```bash
export GITHUB_OAUTH_CLIENT_ID=...
export GITHUB_OAUTH_CLIENT_SECRET=...
```

Now authenticate:

```bash
ironclaw tool auth github
```

IronClaw will open the browser OAuth flow and store the resulting `github_token`.

</Step>

<Step title="Configure a PAT (Fallback)">

If you do not want to run a GitHub OAuth app, you can still use a Personal Access Token:

```bash
ironclaw secret set github_token YOUR_TOKEN
```

<Warning>
Be sure to create a fine-grained personal access token with only the necessary permissions for your use case. When in doubt, choose the least permissive options, you can always create new tokens with different permissions later on
</Warning>

</Step>

</Steps>

---

## Available Actions:

Here are some of the actions your agent can perform with the Github extension:

- `get_repo`: Retrieve repository information  
- `list_issues`: List all issues in a repository  
- `create_issue`: Create a new issue  
- `get_issue`: Get details of a specific issue  
- `list_issue_comments`: List comments on an issue  
- `create_issue_comment`: Add a comment to an issue  
- `list_pull_requests`: List pull requests  
- `create_pull_request`: Create a new pull request  
- `get_pull_request`: Get details of a specific pull request  
- `get_pull_request_files`: Get the list of files in a pull request  
- `create_pr_review`: Submit a pull request review  
- `list_pull_request_comments`: List review comments on a pull request  
- `reply_pull_request_comment`: Reply to a pull request review comment  
- `get_pull_request_reviews`: Get reviews for a pull request  
- `get_combined_status`: Get the combined status for a ref  
- `merge_pull_request`: Merge a pull request  
- `list_repos`: List repositories (user/org)  
- `get_file_content`: Retrieve the content of a file in the repo  
- `trigger_workflow`: Manually trigger a GitHub Actions workflow  
- `get_workflow_runs`: List recent workflow runs  
- `handle_webhook`: Handle a GitHub webhook payload  

---

## Working on Public Repositories

Lets configure our agent to have its own github account, which it can use to create issues and comment on PRs in **public repositories**.

<Steps>

<Step title="Create a new Github account">

Go to https://github.com and create a new account for your agent. If you are already logged in with your personal account you will need to briefly log out to create the new account, but you can log back in right after

</Step>

<Step title="Generate a Personal Access Token">

On the agent's Github account, go to [Settings -> Developer settings -> Personal access tokens -> Tokens (classic)](https://github.com/settings/tokens) and generate a new token (classic) with the following permissions: `repo` -> `public_repo`

</Step>

<Step title="Authenticate the Github Extension">
Now that you have either OAuth app credentials or a PAT, authenticate the Github extension:

```bash
ironclaw tool auth github
```

If `GITHUB_OAUTH_CLIENT_ID` and `GITHUB_OAUTH_CLIENT_SECRET` are set, IronClaw
will use browser OAuth. Otherwise it falls back to prompting for a PAT.

</Step>

<Step title="Test it out!">

Ask your agent to create a test issue in one of your public repositories, and check if the issue was created successfully.

<Tip>
Ask your agent to read the [Github Markdown Guidelines](https://github.com/adam-p/markdown-here/wiki/markdown-cheatsheet) and remember then when creating issues and comments, it can make the formatting much nicer!
</Tip>

</Step>

</Steps>
