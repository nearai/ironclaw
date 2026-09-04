# GitHub Contents Output Normalization

## Context

Issue #7891's generic durable-result work is already merged. The next planned
producer slice is `github.get_file_content`: GitHub returns ordinary file bytes
inside a base64 field, and the current prompt asks the model to decode that
transport representation itself.

This slice changes only the GitHub Contents producer. Its resulting open JSON
value continues through the existing redaction, durable writer, bounded first
observation, and `builtin.result_read` path without a capability-specific host
branch.

## Selected shape

Extend the existing
`crates/extensions/packages/github/wasm-src/src/api/contents.rs::get_file_content`
path after its mediated request succeeds:

```text
GitHub JSON response
  -> existing get_file_content producer
     -> directory/non-file: unchanged
     -> file + base64: decode owned encoding
        -> UTF-8: content=<text>, encoding="utf-8"
        -> non-UTF-8: omit content, encoding="binary_unsupported"
     -> file + none: unchanged (GitHub omitted inline large-file content)
     -> malformed owned encoding: visible capability failure
  -> existing output Value / redaction / durable write / result_read
```

This is an in-place producer transform, not a global output type, parser,
normalizer trait, registry, callback, or host-runtime capability switch.

## Behavior contract

- Preserve the existing request validation, URL construction, mediated HTTP
  request, transport/status error behavior, and useful provider metadata.
- Parse the successful response as JSON in the producer.
- Preserve directory arrays and non-file response objects unchanged.
- For a `type: "file"` object with `encoding: "base64"`:
  - require string `content`;
  - remove GitHub's permitted ASCII whitespace before decoding;
  - decode with the package's existing `base64` dependency;
  - for valid UTF-8, replace `content` with text and set `encoding` to
    `"utf-8"`;
  - for non-UTF-8, remove `content` and set `encoding` to
    `"binary_unsupported"`; do not re-encode or persist the bytes.
- Preserve `encoding: "none"` unchanged. It is GitHub's explicit signal that
  inline content is unavailable for the documented large-file response.
- Fail with cause intact when a file response is malformed, has invalid base64,
  or declares an unexpected encoding. Never fall back to the encoded blob.
- Do not add a new dependency or shared Rust output type. The producer remains
  an open `serde_json::Value` boundary.

## Implementation

### 1. Pin behavior red-first — behavioral

Extend the existing guest test in
`crates/extensions/packages/github/wasm-src/src/lib.rs` through
`execute_inner(...)` and add only distinct cases needed to prove:

- wrapped base64 text becomes `content: "fn main() {}"` and
  `encoding: "utf-8"`;
- binary bytes become `encoding: "binary_unsupported"` with no `content`;
- malformed base64 fails visibly and never returns the original blob;
- directory arrays and `encoding: "none"` file objects remain unchanged;
- the existing `ref` query assertion still holds.

Add a production-wired bundled-artifact test in
`crates/kernel/ironclaw_host_runtime/tests/github_wasm_runtime_contract.rs`
that invokes `github.get_file_content` through the existing local host-runtime
fixture and asserts the shipped artifact's UTF-8, unsupported-binary, and
malformed-base64 branches. The success cases must assert the exact
`RuntimeCapabilityOutcome::Completed` output, including absence of binary bytes;
the malformed case must assert a visible typed failure. The text case must fail
against the pre-change committed WASM artifact.

Update the existing provider-operation E2E assertion in
`tests/e2e/provider_operation_github_repo_cases.py` so the model-visible preview
contains decoded source text and no base64 payload. Keep the live provider
contract test unchanged because GitHub itself still returns base64.

### 2. Normalize in the existing producer — behavioral

Keep the existing signature:

```rust
pub(crate) fn get_file_content(
    owner: &str,
    repo: &str,
    path: &str,
    r#ref: Option<&str>,
) -> Result<String, String>
```

After `github_request(...)` returns, parse and transform the response inline in
this producer. Add no reusable helper unless a second production caller appears
or the function would otherwise exceed the local readability ceiling.

### 3. Align the model contract — behavioral

- Remove the base64-decoding instruction from
  `prompts/github/get_file_content.md`.
- Remove provider-output decoding guidance from the input schema's `path`
  description.
- Add a dedicated producer-specific output schema and manifest reference that
  pins the normalized `content`/`encoding` behavior while permitting preserved
  GitHub metadata and the existing directory-array response. Do not model the
  entire GitHub API as a new Rust type hierarchy.
- Treat the new model-visible schema as an incompatible package contract:
  create a new versioned schema asset and bump the GitHub package release from
  `0.2.8` to `0.3.0`, following
  `docs/internal/reborn/contracts/host-runtime.md`.
- Add the schema asset to the existing GitHub package embedding list and update
  the nearest package/schema validation assertion.
- Preserve
  `crates/app/ironclaw_composition/tests/fixtures/first_party_v2/github.toml` as
  the historical raw-output baseline. Extend
  `first_party_manifest_v3_parity.rs` beside the Gmail exception so it records
  `github.get_file_content` as an explicit semantic output-schema graduation;
  keep every other v2-to-v3 comparison strict.

### 4. Rebuild and verify the shipping artifact — behavioral

Run, in order:

```bash
cargo test --manifest-path crates/extensions/packages/github/wasm-src/Cargo.toml
./scripts/build-wasm-extensions.sh --first-party
python3 scripts/ci/check-wasm-artifact-freshness.py --update
python3 scripts/ci/check-wasm-artifact-freshness.py
cargo test -p ironclaw_host_runtime --test github_wasm_runtime_contract
cargo test -p ironclaw_composition --test first_party_manifest_v3_parity
python3 scripts/ci/test-build-wasm-extensions.sh
python3 scripts/ci/test-check-wasm-artifact-freshness.sh
cargo test -p ironclaw_architecture_tests
cargo fmt --check
```

Run the focused provider-operation E2E lane documented by
`tests/e2e/AGENTS.md`. If its external emulator prerequisite is unavailable,
report that exact limitation; the bundled-WASM host-runtime test remains the
required production-wired local proof.

## Deletion-first and scope checks

- The compensating prompt instruction is deleted in the same slice.
- No old producer normalization path exists to retain or remove.
- Generic result storage and reading already do the required work and stay
  untouched.
- Hosted MCP normalization, Drive regression hardening, format expansion, and
  the extension-wide encoded-output audit remain separate issue slices.

## Compatibility, rollback, and success

This intentionally changes new `github.get_file_content` model-visible records:
text is readable rather than base64, and unsupported binary bytes are absent.
Existing durable records are not rewritten and remain readable through the
merged generic result reader. Request arguments and GitHub egress behavior are
unchanged.

Rollback is code-only and side-effect free, but source, manifest/schema/prompt,
committed `github_tool.wasm`, and the source digest must be reverted together.

The slice succeeds when the guest, bundled runtime, and provider-operation
tests prove readable text reaches the caller; malformed/binary content never
leaks encoded bytes; directory and large-file metadata behavior remains intact;
and the non-mutating WASM freshness check passes.
