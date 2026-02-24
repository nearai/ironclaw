# GitHub Actions Tailscale Deployment

This document describes how to deploy IronClaw to the Raspberry Pi via Tailscale using GitHub Actions.

## Required Secrets

The following repository secrets must be configured:

| Secret | Description | Default |
|--------|-------------|---------|
| `TS_OAUTH_CLIENT_ID` | Tailscale OAuth client ID | Required |
| `TS_OAUTH_SECRET` | Tailscale OAuth client secret | Required |
| `PI_HOST` | Raspberry Pi hostname on Tailscale | `watty-pi` |
| `PI_USER` | SSH user for Pi | `kieran` |
| `DEPLOY_PATH` | Path where binary is installed | `/usr/local/bin/ironclaw` |
| `SERVICE_NAME` | systemd service name | `ironclaw` |

## Deployment Triggers

The deployment workflow runs when:

1. **Tag push**: A version tag is pushed (e.g., `v0.11.1`)
2. **Manual trigger**: Workflow is dispatched manually from the GitHub UI

## Environment Protection

The deploy job uses the `production` environment, which requires approval before executing. Ensure the `production` environment is configured with required reviewers in your repository settings.

## Creating a Release Tag

To trigger a deployment by pushing a tag:

```bash
# Tag the current commit
git tag v0.11.1

# Push the tag to origin
git push origin v0.11.1
```

This will trigger the workflow which builds the ARM64 binary and deploys it to the Raspberry Pi.

## Manual Deployment

To manually trigger a deployment:

1. Navigate to the repository on GitHub
2. Go to **Actions** > **IronClaw ARM64 Build and Deploy**
3. Click **Run workflow**
4. Select the branch to deploy from
5. Click **Run workflow**

## Verification on Pi

After deployment, verify the installation:

```bash
# Check the binary exists and its size
ls -lh /usr/local/bin/ironclaw

# Verify the version
/usr/local/bin/ironclaw --version
```

## Rollback

If the new deployment has issues, rollback using the backup:

```bash
# Restore the previous version
sudo cp /usr/local/bin/ironclaw.bak /usr/local/bin/ironclaw

# Restart the service if needed
sudo systemctl restart ironclaw
```

The deployment script automatically creates a `.bak` file before overwriting the existing binary.
