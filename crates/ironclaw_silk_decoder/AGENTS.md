# Agent Map — ironclaw_silk_decoder

## Start Here

- Read `README.md` first; it documents build/install/protocol and why this is isolated.
- Read `Cargo.toml` and `src/main.rs` before changing behavior.
- This crate is excluded from the workspace so the main IronClaw build does not require libclang.

## What This Crate Owns

- Standalone helper binary `ironclaw-silk-decoder`.
- WeChat raw SILK v3 voice-note decoding to WAV.
- CLI/protocol behavior for the helper process.

## Do Not Move In Here

- Main workspace dependencies or root IronClaw runtime behavior.
- Channel/product workflow logic beyond the decoder protocol.
- Secrets, network access, database access, or agent state.

## Validation

- Build/check from crate directory when touching decoder code: `cargo build --manifest-path crates/ironclaw_silk_decoder/Cargo.toml`
- Follow `README.md` install/protocol checks for manual validation.
- Do not use workspace-wide checks to prove this helper unless explicitly included.

## Agent Notes

- Keep libclang/native decoder requirements isolated here.
- Preserve helper process protocol compatibility for callers.
- Document any protocol change in `README.md`.
