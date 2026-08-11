# ICWM G0C candidate-neutral harness

This standalone Rust crate packages candidate-neutral contracts, schemas, and
deterministic control fixtures produced by the ICWM Gate 0C architecture
investigation. It is deliberately excluded from the IronClaw workspace and has
no dependency on IronClaw or a Matrix SDK.

Governing Lighthouse G0C status: **Revisions Required**. This is
contribution-grade research infrastructure, and this tracked publication
package is **pending exact-head review**. Investigation review history does not
approve these tracked bytes. The package does not accept Gate 0, G0C, ADR-005,
ADR-006, a production dependency, store, or runtime placement.

## Boundaries

- Inputs are credential-free. Authentication and network mediation remain host
  responsibilities.
- Stable identifiers use domain-separated, length-delimited typed bytes.
- The runner owns effect ordering, virtual time, response scripts, failpoints,
  cancellation, and normalized result production.
- `not_applicable` requires a topology or exact-feature reason; it cannot hide
  an unimplemented applicable test.
- Candidate adapters must expose prepare, commit, and acknowledge boundaries.
  The neutral harness does not claim candidate-store atomicity.
- The control adapter proves only the common contracts.

## Reproduce

From the repository root:

```sh
cargo fmt --manifest-path harness/icwm-g0c/Cargo.toml -- --check
target_dir="$(mktemp -d)"
CARGO_TARGET_DIR="$target_dir" cargo test --locked --manifest-path harness/icwm-g0c/Cargo.toml
CARGO_TARGET_DIR="$target_dir" cargo clippy --locked --manifest-path harness/icwm-g0c/Cargo.toml --all-targets -- -D warnings
python3 harness/icwm-g0c/verify-publication.py
```

Remove the temporary target directory after recording author or verifier
results. A verifier must run these commands independently from a clean
worktree; author runs are not verification.

## Provenance

The tested IronClaw source baseline is
`2d64363101ef0ff062a6345a5573ee855766552f`. The approved ignored-worktree
common aggregate was
`e603556c99acd3de3c69b60acb74486a858993e5bb7fe9d1bd66827ea6d3d34b`.
Tracked-path publication changes paths and explanatory material, so
`PUBLICATION-MANIFEST.json` is the authoritative file manifest for this
surface. Its detached digest is in `PUBLICATION-MANIFEST.sha256`.

The containing Git commit is intentionally not embedded in either manifest.
Exact-head review publication and the post-merge receipt bind that commit
externally, avoiding a circular hash.
