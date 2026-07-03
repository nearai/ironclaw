# ironclaw-silk-decoder

Standalone helper that converts WeChat raw SILK v3 voice notes into a
WAV-wrapped 16‑bit mono PCM stream that downstream consumers (the
transcription pipeline, the agent, audio players) can use.

The decoder lives in its own crate for one reason: SILK decoding requires
[`silk-codec`](https://crates.io/crates/silk-codec), which compiles a vendored
C implementation and pulls in `bindgen` + `clang-sys`. Anything that uses
`silk-codec` therefore needs `libclang` and a C toolchain at build time. By
keeping that dependency in a separate, workspace-excluded crate, the main
IronClaw build does **not** need `libclang`.

## Build

```bash
./crates/ironclaw_silk_decoder/build.sh
```

The build script runs `cargo build --release` with this crate's manifest
and produces `target/release/ironclaw-silk-decoder` (relative to the
crate). It is excluded from the top-level workspace so it does not
participate in `cargo build --workspace`.

## Install

The IronClaw host looks for the binary in this order:

1. `IRONCLAW_SILK_DECODER` environment variable (a path to the binary)
2. A sibling of the running `ironclaw` executable named
   `ironclaw-silk-decoder` (with `.exe` on Windows)
3. `ironclaw-silk-decoder` on `$PATH`

If none of those resolve, WeChat voice notes are delivered as raw
`audio/silk` attachments and the agent's transcription pipeline skips
them. This is intentional — the decoder is optional.

## Protocol

```
stdin  <- raw SILK v3 bytes (the bytes WeChat ships as `audio/silk`)
stdout <- a complete WAV file (RIFF/WAVE, 16‑bit LE PCM, mono)
stderr <- human-readable diagnostics
exit   <- 0 success, 1 IO, 2 invalid argument, 3 decode failure
```

Default sample rate is 24 000 Hz to match WeChat's voice-note encoding.
Override with `--sample-rate <hz>` (8 000–48 000).

## Why a separate process?

- **No `libclang` in the main build.** Removing `silk-rs` from the host
  means contributors and CI no longer need a Clang toolchain just to
  compile IronClaw.
- **Crash isolation.** Untrusted SILK bytes from a remote messaging
  server are decoded in a child process. A bug in the C decoder kills
  the child, not the host.
- **Optional install.** Distributions that don't care about WeChat voice
  transcription can ship without the decoder; everything else still
  works.
