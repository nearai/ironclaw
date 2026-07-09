# test-tools — uploadable WASM tool fixture bundles

Standalone extension bundles for exercising the WebUI v2 **Import Tool**
flow (`POST /api/webchat/v2/extensions/import`, admin-only) during live QA.
Each directory is one uploadable bundle; zipping it produces the file the
Import button accepts.

These are used by both manual/live QA and automated tests: the E2E suite builds
and uploads these bundles, while the Rust suites build their zip fixtures in-memory
(`ironclaw_reborn_composition::extension_lifecycle` tests) and pin only the
manifests here via `include_str!`
(`available_extensions::tests::test_tool_fixture_manifests_stay_importable`),
so a manifest that drifts out of the import-legal shape fails CI, not the
demo.

## Tool matrix — the three obligation categories

| Tool | Effects | Credential | Case |
|---|---|---|---|
| `ascii-renderer` | `dispatch_capability` | none | pure compute, no obligations |
| `hacker-news` | `+ network` | none | egress allowlist via manifest `network_targets`, no key |
| `market-data` | `+ network, use_secret` | tenant-shared `market_data_api_key` | egress allowlist + host-mediated secret injection |

All data is **canned** — no fixture ever performs live egress; the network
declarations exist to exercise the obligation planner, not to call out.

For `market-data`, seed the shared key before activating:
`IRONCLAW_REBORN_DEV_SECRET__market_data_api_key=<value>` (see
`.env.example`).

## Bundle layout

```
<tool>/
├── manifest.toml   # reborn.extension_manifest.v2 — InstalledLocal-legal shape:
│                   #   trust = "third_party" + capability_provider host_api
│                   #   (uploads are validated as ManifestSource::InstalledLocal,
│                   #   which rejects first-party trust/runtime claims and
│                   #   legacy top-level [[capabilities]])
├── wasm/           # built module at the path [runtime].module declares
├── schemas/        # capability input/output JSON schemas
├── prompts/        # capability prompt docs
└── wasm-src/       # cargo source (cdylib + wit-bindgen against /wit/tool.wit)
```

The import path requires every manifest-declared asset (module, schemas,
prompt docs) to be present in the zip, rejects duplicate zip entry names,
and rejects non-WASM runtimes.

## Building

```bash
rustup target add wasm32-wasip2          # once
bash scripts/build-test-tools.sh         # all tools
bash scripts/build-test-tools.sh market-data   # one tool
```

The script builds each `wasm-src/` for `wasm32-wasip2` (release) — this
target emits a **WASI component**, which is what the runtime loads
(`wasmtime::component::Component::new`); a `wasm32-wasip1` core module
imports fine but fails at dispatch with "the tool manifest is invalid".
The script verifies the component header, copies the artifact to the
manifest's `[runtime].module` path, and produces `test-tools/<tool>.zip`.
The `.zip` files and `wasm-src/target/` are git-ignored build artifacts —
only sources, manifests, schemas, and prompts are tracked.
