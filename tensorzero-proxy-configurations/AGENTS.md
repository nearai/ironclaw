# Repository Guidelines

## Project Structure & Module Organization
This repository is a small collection of standalone Python proxy scripts and supporting configuration. `tensorzero-proxy.py` is the main OpenAI-compatible proxy with Codex/Responses API translation. `ironclaw-proxy.py` is a narrower compatibility proxy for IronClaw clients. Experimental variants live beside them as `experimental-*.py` and should be treated as draft implementations. Additionally, these files may be open in the user's editor and thus, would not be wise to edit willy-nilly. Runtime configuration lives in `tensorzero.toml`; CI settings live in `.gitlab-ci.yml`. There is no dedicated `src/` or `tests/` directory yet.

## Build, Test, and Development Commands
Use the system Python 3 interpreter; there is no build step or dependency lockfile in this repo.

- `python3 tensorzero-proxy.py --help` checks CLI options for the main proxy.
- `python3 tensorzero-proxy.py --port 3001 --tensorzero http://127.0.0.1:3000` runs the main proxy locally.
- `python3 ironclaw-proxy.py --port 3002 --tensorzero http://127.0.0.1:3000` runs the IronClaw adapter.
- `python3 -m py_compile tensorzero-proxy.py ironclaw-proxy.py` performs a fast syntax check before committing.

## Coding Style & Naming Conventions
Follow existing Python style: 4-space indentation, standard library imports grouped at the top, `snake_case` for functions and variables, and `UPPER_SNAKE_CASE` for module-level constants such as `TENSORZERO_URL`. Keep scripts runnable as CLIs with `argparse` and `if __name__ == "__main__": main()`. Prefer focused, in-file helper functions over adding abstraction layers unless duplication becomes persistent.

## Testing Guidelines
There is no formal test suite yet. For now, contributors should treat syntax checks and manual endpoint smoke tests as the minimum bar. Validate both `--help` output and a local request path against the proxy you changed. When adding reusable logic, prefer extracting it into pure functions so future `pytest` coverage can be added easily. If you add tests later, place them in a new `tests/` directory and name files `test_*.py`.

## Commit & Pull Request Guidelines
Git history currently mixes short ad hoc commits (`test`, `m`) with generated GitLab-style messages. Prefer concise, imperative subjects such as `Add embeddings timeout logging` or `Clean TensorZero response fields`. Pull requests should describe the proxy path affected, list manual verification steps, link the related issue, and include request/response samples when behavior changes.

## Configuration & Security Notes
Do not commit real service URLs, API keys, or `.env` secrets. Keep local overrides in an untracked `.env` file next to the scripts, and review `.gitlab-ci.yml` changes carefully because secret detection and SAST are enabled in CI.
