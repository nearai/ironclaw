# Repository Guidelines

## Project Structure & Module Organization
This repository is a small Rust client for the Ironclaw Unix socket REPL. The primary package manifest is `Cargo.toml`, and the current implementation lives in `src/lib.rs`. `README.md` only contains a placement note, so keep contributor-facing guidance here. There is also a duplicate `src/Cargo.toml`; if you touch dependency metadata, keep both manifests aligned or remove the duplicate in a dedicated cleanup change.

## Build, Test, and Development Commands
Use Cargo from the repository root:

- `cargo fmt` formats the codebase with standard Rust style.
- `cargo check` verifies types and dependencies without producing a release binary.
- `cargo build` compiles the client.
- `cargo test` runs unit and integration tests.
- `cargo run -- ~/.ironclaw/ironclaw.sock` starts the client against an explicit socket path.

If Cargo cannot resolve crates, confirm network access to `crates.io` or use a pre-populated dependency cache.

## Coding Style & Naming Conventions
Target Rust 2021 and follow `rustfmt` defaults: 4-space indentation, trailing commas where `rustfmt` adds them, and no manual alignment. Prefer `snake_case` for functions and variables, `PascalCase` for enums and structs, and descriptive message variants such as `Connect`, `Response`, and `Disconnect`. Keep async I/O paths explicit and avoid hiding protocol behavior in large helper functions.

## Testing Guidelines
There are no committed tests yet. Add unit tests next to the code they exercise with `#[cfg(test)]`, and add integration tests under `tests/` when validating end-to-end socket behavior. Name tests after the expected behavior, for example `disconnects_on_quit_command`. Run `cargo test` before opening a PR.

## Commit & Pull Request Guidelines
The existing history uses short subjects (`initial`, `changes`) plus automated setup commits. Keep new commits imperative and more descriptive, such as `Add pong reply for server ping`. Pull requests should explain the behavior change, list local verification steps, and link any related issue. Include terminal output or screenshots only when the change affects user-visible CLI behavior.

## Next
codex integration
