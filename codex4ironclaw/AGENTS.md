# Repository Guidelines

## Project Structure & Module Organization
This repository is a small containerized Codex worker. Keep top-level runtime files focused and easy to scan. `Dockerfile` builds the worker image, `docker-compose.yml` runs the worker plus an example subagent, and `README.md` covers operator-facing setup. Python runtime helpers live in [`health_server.py`](/home/starforce/codex4ironclaw/health_server.py) and [`scripts/codex_agent_client.py`](/home/starforce/codex4ironclaw/scripts/codex_agent_client.py). Configuration templates live in [`example-codex.toml`](/home/starforce/codex4ironclaw/example-codex.toml), [`config.toml`](/home/starforce/codex4ironclaw/config.toml), and [`agent_comm_protocol.json`](/home/starforce/codex4ironclaw/agent_comm_protocol.json). Avoid committing generated files outside `__pycache__/`.

## Build, Test, and Development Commands
Use Docker-first workflows:

- `docker build -t ironclaw-codex-worker:latest .` builds the worker image.
- `docker compose up --build` starts the worker stack from [`docker-compose.yml`](/home/starforce/codex4ironclaw/docker-compose.yml).
- `python3 health_server.py --port 8443` runs the health endpoint locally for quick checks.
- `python3 -m py_compile health_server.py scripts/codex_agent_client.py` catches Python syntax errors before committing.

When changing container behavior, validate both CLI mode and WebSocket-related configuration paths.

## Coding Style & Naming Conventions
Python uses 4-space indentation, `snake_case` for functions and variables, and short module docstrings. Keep scripts dependency-light and standard-library-first unless the container already installs the package. Environment variables stay uppercase, and JSON/TOML keys should remain descriptive and stable because they are part of runtime configuration. Prefer small, single-purpose files over adding framework scaffolding.

## Testing Guidelines
There is no formal test suite checked in yet. Treat syntax checks, container builds, and health endpoint smoke tests as the minimum bar. If you add tests, place them in a new `tests/` directory and name files `test_*.py`. For runtime changes, verify `GET /health` and `GET /ready` inside the container.

## Commit & Pull Request Guidelines
Recent history uses short imperative subjects such as `Edit README.md` and `Add new file`. Keep that imperative style, but make it more specific, for example `Add WebSocket auth header validation`. Pull requests should include a concise summary, affected files, validation steps run, and linked issues. Include sample request/response output or screenshots when changing health checks, container behavior, or protocol-facing flows.

## Security & Configuration Tips
Do not commit real secrets. Use `.env.example` as the template, keep local secrets in `.env`, and rotate `AGENT_AUTH_TOKEN` for shared environments. Review exposed ports and mounted volumes in Compose before merging changes.
