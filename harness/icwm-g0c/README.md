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

Identifier differential tests use the exact crates.io `ruma-common 0.19.0`
parser layer. That is the current release observed during the 2026-08-11
recapture and the same `ruma-common` version resolved through the Matrix SDK
0.18 candidate graph. It is a test-only oracle, not a production dependency or
an architectural selection of the full SDK.

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
The tracked
[candidate evidence provenance index](../../docs/internal/research/icwm-g0c/evidence-provenance.json)
binds candidate conclusions to exact source and dependency identities,
normalized retained-receipt hashes, and bounded recapture commands. It does
not publish raw transcripts, graphs, build products, crypto stores, secrets,
ciphertext, live state, or service logs. Authorized recapture uses an
access-controlled disposable worktree and compares new hashes without
overwriting historical receipts. In an authorized retained-evidence checkout,
the gitignored material is placed at the repository-relative
`.work/evidence/icwm-n-g0c/candidates/` convention. It is absent from an
ordinary clone; the index provides per-file and candidate-source-tree digests
that must be confirmed after approved private transfer and before recapture.

Tracked-path publication changes paths and explanatory material, so
`PUBLICATION-MANIFEST.json` is the authoritative file manifest for this
surface. Its detached digest is in `PUBLICATION-MANIFEST.sha256`.

The containing Git commit is intentionally not embedded in either manifest.
Exact-head review publication and the post-merge receipt bind that commit
externally, avoiding a circular hash.
