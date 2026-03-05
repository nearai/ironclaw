> **Customize this file**: Tailor this template to your project by noting specific contribution types you're looking for, adding a Code of Conduct, or adjusting the writing guidelines to match your style.

# Contribute to the documentation

Thank you for your interest in contributing to our documentation! This guide will help you get started.

## How to contribute

### Option 1: Edit directly on GitHub

1. Navigate to the page you want to edit
2. Click the "Edit this file" button (the pencil icon)
3. Make your changes and submit a pull request

### Option 2: Local development

1. Fork and clone this repository
2. Install the Mintlify CLI: `npm i -g mint`
3. Create a branch for your changes
4. Make changes
5. Navigate to the docs directory and run `mint dev`
6. Preview your changes at `http://localhost:3000`
7. Commit your changes and submit a pull request

For more details on local development, see our [development guide](development.mdx).

## Updating Screenshots

When the web UI changes, regenerate screenshots and documentation:

```bash
./docs/scripts/capture-screenshots.sh
```

The script will:
1. Build IronClaw if needed
2. Start a temporary server with seeded data
3. Capture verified screenshots using Playwright
4. Generate/update Mintlify documentation in `docs/ui-reference/`
5. Clean up and exit

### Configuration

Create `.env.screenshot` at the repo root to customize:

```bash
# Screenshot server configuration
SCREENSHOT_PORT=13001
SCREENSHOT_VIEWPORT=1280x800
IRONCLAW_TOKEN=screenshot-test-token
```

See `.env.screenshot.template` for all options.

### Review Generated Documentation

After running the script:

```bash
# Review the generated docs
git diff docs/ui-reference/

# Review the new screenshots
git status docs/assets/screenshots/

# Commit both together
git add docs/assets/screenshots/ docs/ui-reference/
git commit -m "docs: update UI screenshots and reference docs"
```

### Troubleshooting

| Issue | Solution |
|-------|----------|
| Port already in use | Change `SCREENSHOT_PORT` in `.env.screenshot` |
| IronClaw fails to build | Check Rust toolchain and dependencies |
| Tests fail | Check IronClaw is running and accessible at the configured URL |
| Screenshots look wrong | Adjust `SCREENSHOT_VIEWPORT` for different dimensions |

## Writing guidelines

- **Use active voice**: "Run the command" not "The command should be run"
- **Address the reader directly**: Use "you" instead of "the user"
- **Keep sentences concise**: Aim for one idea per sentence
- **Lead with the goal**: Start instructions with what the user wants to accomplish
- **Use consistent terminology**: Don't alternate between synonyms for the same concept
- **Include examples**: Show, don't just tell

## Updating Screenshots

When the web UI changes, regenerate screenshots using the automated pipeline:

```bash
./docs/scripts/capture-screenshots.sh
```

The script will:
1. Build IronClaw if the binary is not found
2. Start IronClaw with a temporary database
3. Seed test data via REST API
4. Run Playwright tests to capture verified UI screenshots
5. Generate Mintlify documentation from test metadata
6. Clean up and stop IronClaw

### Configuration

Copy `.env.screenshot.template` to `.env.screenshot` at the repo root to customize:

```bash
cp .env.screenshot.template .env.screenshot
```

Available options:
- `SCREENSHOT_VIEWPORT`: Viewport dimensions (default: 1280x800)
- `SCREENSHOT_PORT`: Port for temporary IronClaw instance (default: 13001)
- `HEALTH_TIMEOUT`: Seconds to wait for IronClaw to start (default: 60)

### Reviewing Generated Documentation

After running the capture script:

1. Review screenshots in `docs/assets/screenshots/`
2. Review generated docs in `docs/ui-reference/`
3. Edit generated `.mdx` files to add edge cases or clarify instructions
4. Commit both screenshots and documentation together:

```bash
git add docs/assets/screenshots/*.png
git add docs/ui-reference/
git commit -m "docs: update UI screenshots and reference docs"
```

### Troubleshooting

| Issue | Solution |
|-------|----------|
| Port already in use | Change `SCREENSHOT_PORT` in `.env.screenshot` |
| IronClaw build fails | Run `cargo build --release` manually and check errors |
| Playwright browser missing | Run `npx playwright install chromium` |
| Tests timeout | Increase `HEALTH_TIMEOUT` in `.env.screenshot` |
