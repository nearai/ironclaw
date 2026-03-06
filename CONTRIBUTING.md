# Contributing to IronClaw

Thank you for your interest in contributing to IronClaw! We welcome contributions from the community.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Reporting Bugs](#reporting-bugs)
- [Suggesting Enhancements](#suggesting-enhancements)
- [Pull Requests](#pull-requests)
- [Development Setup](#development-setup)
- [Coding Standards](#coding-standards)
- [Feature Parity Updates](#feature-parity-updates)

## Code of Conduct

This project and everyone participating in it is governed by basic principles of respect and inclusivity. By participating, you are expected to uphold this code.

## Reporting Bugs

Before creating bug reports, please check the issue list as you might find out that you don't need to create one. When you are creating a bug report, please include as many details as possible:

- **Use a clear and descriptive title**
- **Describe the exact steps to reproduce the problem**
- **Provide specific examples to demonstrate the steps**
- **Describe the behavior you observed after following the steps**
- **Explain which behavior you expected to see instead and why**
- **Include screenshots or animated GIFs if helpful**
- **Include your environment details** (OS, Rust version, etc.)

## Suggesting Enhancements

Enhancement suggestions are tracked as GitHub issues. When creating an enhancement suggestion, include:

- **Use a clear and descriptive title**
- **Provide a step-by-step description of the suggested enhancement**
- **Provide specific examples to demonstrate the steps**
- **Describe the current behavior and explain the behavior you expected**
- **Explain why this enhancement would be useful**

## Pull Requests

### Before Submitting a PR

1. **Format your code**: Run `cargo fmt`
2. **Fix all warnings**: Run `cargo clippy --all --benches --tests --examples --all-features` and fix ALL warnings (including pre-existing ones)
3. **Run tests**: Ensure all tests pass with `cargo test`
4. **Update documentation**: Update relevant documentation if needed
5. **Check Feature Parity**: If your change affects tracked capabilities, update `FEATURE_PARITY.md`

### PR Submission Process

1. Fork the repo and create your branch from `main`
2. Make your changes with clear, descriptive commit messages
3. Push to your fork and submit a pull request
4. Ensure the PR description clearly describes the problem and solution
5. Wait for review and address any feedback

### PR Title Format

Use conventional commit format:
- `feat:` for new features
- `fix:` for bug fixes
- `docs:` for documentation changes
- `test:` for adding tests
- `refactor:` for code refactoring
- `chore:` for maintenance tasks

Example: `feat: add Brave Web Search WASM tool`

## Development Setup

### Prerequisites

- Rust 1.85+
- PostgreSQL 15+ with [pgvector](https://github.com/pgvector/pgvector) extension (or libSQL)
- NEAR AI account (authentication via setup wizard)

### Building

```bash
# Clone the repository
git clone https://github.com/nearai/ironclaw.git
cd ironclaw

# Build
cargo build

# Run tests
cargo test

# Run with logging
RUST_LOG=ironclaw=debug cargo run
```

### Database Setup

For PostgreSQL:
```bash
# Create database
createdb ironclaw

# Run migrations (handled automatically on first run)
```

For libSQL (embedded):
```bash
# Create ironclaw.db file in project root
touch ironclaw.db
```

## Coding Standards

### Rust Style Guide

- Follow the official Rust style guidelines
- Use `cargo fmt` to format code
- Use `cargo clippy` to catch common mistakes
- Write clear, descriptive function and variable names
- Add documentation comments for public APIs

### Code Organization

- Keep functions focused and concise
- Separate concerns into modules
- Use appropriate visibility (pub, pub(crate), private)
- Add inline comments for complex logic
- Write tests for new functionality

### Error Handling

- Use `Result<T, E>` for fallible operations
- Provide descriptive error messages with context
- Use the `thiserror` crate for custom error types
- Avoid `unwrap()` and `expect()` in production code (use in tests is acceptable)

### Logging

- Use the `tracing` crate for logging
- Use appropriate log levels:
  - `ERROR`: Serious problems
  - `WARN`: Potentially problematic situations
  - `INFO`: Important runtime events
  - `DEBUG`: Detailed diagnostic information
  - `TRACE`: Very detailed diagnostic information

## Feature Parity Updates

When your change affects any tracked capability in `FEATURE_PARITY.md`:

1. Review the relevant parity rows in `FEATURE_PARITY.md`
2. Update status indicators if behavior changed:
   - ❌ Not implemented
   - 🚧 Partially implemented
   - ✅ Fully implemented
3. Add notes explaining any changes
4. Include the `FEATURE_PARITY.md` diff in your commit when applicable

Do not open a PR that changes feature behavior without checking `FEATURE_PARITY.md` for needed status updates.

## Questions?

Feel free to open an issue with the `question` label or join our community:
- Telegram: [@ironclawAI](https://t.me/ironclawAI)
- Reddit: [r/ironclawAI](https://www.reddit.com/r/ironclawAI/)

---

Thank you for contributing to IronClaw! 🦞
