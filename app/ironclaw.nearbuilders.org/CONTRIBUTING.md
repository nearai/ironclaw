# Contributing Guide

Thank you for contributing to everything-dev! 🎉

## Quick Setup

```bash
bun install              # Install dependencies
bun db:migrate           # Run database migrations
bos dev                  # Start development (host mode auto-detected)
```

Visit http://localhost:3003 (UI), http://localhost:3001 (API), and http://localhost:3002 (Auth).

**Need more details?** See [README.md](./README.md) for architecture overview and [LLM.txt](./LLM.txt) for technical deep-dive.

## Development Workflow

### Making Changes

- **UI Changes**: Edit `ui/src/` → hot reload automatically → deploy with `bun run build:ui`
- **API Changes**: Edit `api/src/` → hot reload automatically → deploy with `bun run build:api`
- **Plugin Changes**: Edit `plugins/*/src/` → hot reload automatically → deploy per plugin
- **Host Changes**: Edit `host/src/` or `bos.config.json` → deploy with `bun run build:host`

### Plugin Architecture

Business logic lives in independent plugins under `plugins/`:

- **`plugins/registry/`** — FastKV app discovery, metadata publish/relay (no database)
- **`plugins/auth/`** — Authentication and authorization (Better-Auth, NEAR SIWN, organizations, API keys)
- **`plugins/projects/`** — Projects CRUD, KV store, org management, API keys (SQLite via libsql)
 - **`plugins/_template/`** — Scaffold for new plugins

Each plugin has its own `contract.ts`, `index.ts`, `rspack.config.js`, and `package.json`. Routes are namespaced in the UI: `apiClient.registry.*()` and `apiClient.projects.*()`.

The `api/` package is a thin structural shell with only health/ping routes and shared auth middleware. It can compose across plugins in-process via `createPlugin.withPlugins<PluginsClient>()` — the API receives typed client factories for all other plugins and calls their routers directly without HTTP roundtrips.

Plugin and API variables are configured in `bos.config.json`:
- API variables: `app.api.variables` → `config.variables` in `initialize`
- Plugin variables: `plugins.{key}.variables` → plugin's own `config.variables` in `initialize`

Plugins are accessible both directly via HTTP (`/api/{key}/*`) and in-process via `services.plugins.{key}()`. The UI uses HTTP; the API uses in-process for composition.

### Environment Configuration

All runtime URLs are configured in `bos.config.json` - no rebuild needed! Switch environments:

```bash
NODE_ENV=development bun dev:host  # Use local services (default)
NODE_ENV=production bun dev:host   # Use production CDN URLs
```

Secrets go in `.env` (see [.env.example](./.env.example) for required variables).

### Project Documentation

- **[AGENTS.md](./AGENTS.md)** - Operational guide for AI agents
- **[README.md](./README.md)** - Architecture, tech stack, and quick start
- **[LLM.txt](./LLM.txt)** - Technical guide for LLMs and developers
- **[api/README.md](./api/README.md)** - API plugin documentation
- **[ui/README.md](./ui/README.md)** - Frontend documentation
- **[host/README.md](./host/README.md)** - Server host documentation
- **[plugins/auth/README.md](./plugins/auth/README.md)** - Auth plugin documentation

## Git Workflow

### Branch Naming

Create feature branches from `main`:

```bash
git checkout main
git pull origin main
git checkout -b feature/amazing-feature
```

**Branch naming conventions:**
- `feature/description` - New features
- `fix/description` - Bug fixes
- `docs/description` - Documentation changes
- `refactor/description` - Code refactoring
- `test/description` - Test additions/changes

### Semantic Commits

Use [Semantic Commits](https://gist.github.com/joshbuchea/6f47e86d2510bce28f8e7f42ae84c716) for clear history:

```bash
# Format: <type>(<scope>): <subject>
git commit -m "feat(api): add user profile endpoint"
git commit -m "fix(ui): resolve routing issue on mobile"
git commit -m "docs(readme): update setup instructions"
git commit -m "refactor(api): simplify auth middleware"
git commit -m "test(ui): add coverage for login flow"
```

**Types:**
- `feat:` - New feature
- `fix:` - Bug fix
- `docs:` - Documentation only
- `style:` - Code style (formatting, no logic change)
- `refactor:` - Code refactoring
- `perf:` - Performance improvements
- `test:` - Tests
- `chore:` - Build/config/tooling changes

### Changesets

We use [Changesets](https://github.com/changesets/changesets) for versioning.

**When to add a changeset:**
- Any user-facing change (features, fixes, deprecations)
- Breaking changes
- Skip for: docs-only changes, internal refactors, test-only changes

**Create a changeset:**
```bash
bun run changeset
# Follow prompts to select packages and write description
```

**Changeset file format:**
```markdown
---
"api": minor
"ui": patch
---

Added new endpoint for user profiles
```

**The release workflow:**
1. Changesets action creates a "Version Packages" PR on merge to main
2. On merge of that PR, GitHub releases are created for api/ui
3. Deployments happen automatically via CI

### Pull Request Process

1. **Before creating PR:**
   ```bash
   bun run test    # Run all tests
   bun typecheck   # Type check all packages
   bun lint        # Run linting
   ```

2. **Create PR from your fork:**
   - Push branch to your fork: `git push origin feature/amazing-feature`
   - Open PR against `main` branch of upstream repo
   - Use descriptive title following semantic format
   - Fill out PR template if provided

3. **PR requirements:**
   - All tests must pass
   - Type checking must pass
   - Linting must pass
   - Changeset added (if applicable)

4. **After merge:**
   - Delete your branch
   - Changesets action will handle versioning

## Contributing Code

1. **Fork** the repository on GitHub
2. **Clone** your fork locally
3. **Create** a feature branch: `git checkout -b feature/amazing-feature`
4. **Make** your changes
5. **Test** thoroughly: `bun run test` and `bun typecheck`
6. **Add changeset** if needed: `bun run changeset`
7. **Commit** using [Semantic Commits](https://gist.github.com/joshbuchea/6f47e86d2510bce28f8e7f42ae84c716)
8. **Push** to your fork: `git push origin feature/amazing-feature`
9. **Open** a Pull Request to the main repository

### Code Style

- Follow existing TypeScript patterns and conventions
- Ensure type safety (no `any` types unless absolutely necessary)
- Write descriptive commit messages
- Add tests for new features
- Use semantic Tailwind classes (see LLM.txt for style guide)
- No code comments in implementation (code should be self-documenting)

### Linting

We use [Biome](https://biomejs.dev/) for linting and formatting:

```bash
bun lint        # Check linting
bun lint:fix    # Fix auto-fixable issues
bun format      # Format code
```

## Reporting Issues

Use [GitHub Issues](https://github.com/NEARBuilders/everything-dev/issues) with:

- **Clear description** of the problem
- **Steps to reproduce** the issue
- **Expected behavior** vs **actual behavior**
- **Environment details** (OS, Node/Bun version, browser, etc.)

## Getting Help

- Check [AGENTS.md](./AGENTS.md) for agent operational guidance
- Check the [README](./README.md) for architecture and setup
- Read the [LLM.txt](./LLM.txt) for technical details
- Review workspace READMEs for specific documentation
- Ask questions in GitHub Issues or Discussions

---

Thank you for your contributions! 💚
