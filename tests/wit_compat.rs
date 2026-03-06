//! WIT compatibility tests for WASM tools and channels.
//!
//! These tests verify that pre-built WASM components can be compiled and
//! instantiated against the current host linker. If the WIT interface
//! changes, these tests catch any breakage in existing tools/channels.
//!
//! Prerequisites: build WASM extensions first with:
//!   ./scripts/build-wasm-extensions.sh
//!
//! The tests are skipped (not failed) when no WASM artifacts are found,
//! so `cargo test` still passes without building extensions first.
//! CI runs the build script before these tests.

use std::path::{Path, PathBuf};

use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiView};

/// Minimal store data that satisfies WasiView for component instantiation.
struct TestStoreData {
    wasi: WasiCtx,
    table: ResourceTable,
}

impl TestStoreData {
    fn new() -> Self {
        Self {
            wasi: WasiCtxBuilder::new().build(),
            table: ResourceTable::new(),
        }
    }
}

impl WasiView for TestStoreData {
    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.wasi
    }

    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

/// Search paths for WASM artifacts produced by cargo-component.
fn find_wasm_artifact(source_dir: &Path, crate_name: &str) -> Option<PathBuf> {
    let artifact_name = crate_name.replace('-', "_");

    // Crate-local target dir (CI, default cargo)
    for target_triple in &["wasm32-wasip2", "wasm32-wasip1", "wasm32-wasi"] {
        let candidate = source_dir
            .join("target")
            .join(target_triple)
            .join("release")
            .join(format!("{artifact_name}.wasm"));
        if candidate.exists() {
            return Some(candidate);
        }
    }

    // Shared target dir (CARGO_TARGET_DIR env)
    if let Ok(shared) = std::env::var("CARGO_TARGET_DIR") {
        for target_triple in &["wasm32-wasip2", "wasm32-wasip1", "wasm32-wasi"] {
            let candidate = Path::new(&shared)
                .join(target_triple)
                .join("release")
                .join(format!("{artifact_name}.wasm"));
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    // Common shared target location (~/.cargo/shared-target)
    if let Ok(home) = std::env::var("HOME") {
        let shared = PathBuf::from(home).join(".cargo/shared-target");
        if shared.exists() {
            for target_triple in &["wasm32-wasip2", "wasm32-wasip1", "wasm32-wasi"] {
                let candidate = shared
                    .join(target_triple)
                    .join("release")
                    .join(format!("{artifact_name}.wasm"));
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
    }

    None
}

/// Parse registry manifests to discover all WASM extensions.
/// Returns (name, source_dir, crate_name) tuples.
fn discover_extensions() -> Vec<(String, PathBuf, String)> {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut extensions = Vec::new();

    for dir in &["registry/tools", "registry/channels"] {
        let registry_dir = repo_root.join(dir);
        if !registry_dir.exists() {
            continue;
        }

        for entry in std::fs::read_dir(&registry_dir)
            .expect("failed to read registry dir")
            .flatten()
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }

            let content = std::fs::read_to_string(&path).expect("failed to read manifest");
            let manifest: serde_json::Value =
                serde_json::from_str(&content).expect("failed to parse manifest");

            let name = manifest["name"].as_str().unwrap_or("unknown").to_string();
            let source_dir = manifest["source"]["dir"]
                .as_str()
                .map(|d| repo_root.join(d));
            let crate_name = manifest["source"]["crate_name"]
                .as_str()
                .map(|s| s.to_string());

            if let (Some(source_dir), Some(crate_name)) = (source_dir, crate_name)
                && source_dir.exists()
            {
                extensions.push((name, source_dir, crate_name));
            }
        }
    }

    extensions
}

fn compile_component(
    engine: &wasmtime::Engine,
    wasm_bytes: &[u8],
) -> Result<wasmtime::component::Component, String> {
    wasmtime::component::Component::new(engine, wasm_bytes)
        .map_err(|e| format!("compilation failed: {e}"))
}

/// Stub the shared tool host functions on a linker instance builder.
fn stub_tool_host_functions(
    host: &mut wasmtime::component::LinkerInstance<'_, TestStoreData>,
) -> Result<(), String> {
    host.func_new("log", |_ctx, _args, _results| Ok(()))
        .map_err(|e| format!("stub 'log': {e}"))?;

    host.func_new("now-millis", |_ctx, _args, results| {
        results[0] = wasmtime::component::Val::U64(0);
        Ok(())
    })
    .map_err(|e| format!("stub 'now-millis': {e}"))?;

    host.func_new("workspace-read", |_ctx, _args, results| {
        results[0] = wasmtime::component::Val::Option(None);
        Ok(())
    })
    .map_err(|e| format!("stub 'workspace-read': {e}"))?;

    host.func_new("http-request", |_ctx, _args, results| {
        results[0] = wasmtime::component::Val::Result(Err(Some(Box::new(
            wasmtime::component::Val::String("stub".into()),
        ))));
        Ok(())
    })
    .map_err(|e| format!("stub 'http-request': {e}"))?;

    host.func_new("tool-invoke", |_ctx, _args, results| {
        results[0] = wasmtime::component::Val::Result(Err(Some(Box::new(
            wasmtime::component::Val::String("stub".into()),
        ))));
        Ok(())
    })
    .map_err(|e| format!("stub 'tool-invoke': {e}"))?;

    host.func_new("secret-exists", |_ctx, _args, results| {
        results[0] = wasmtime::component::Val::Bool(false);
        Ok(())
    })
    .map_err(|e| format!("stub 'secret-exists': {e}"))?;

    Ok(())
}

/// Instantiate a tool component (world: sandboxed-tool, imports: near:agent/host).
fn instantiate_tool_component(
    engine: &wasmtime::Engine,
    component: &wasmtime::component::Component,
) -> Result<(), String> {
    use wasmtime::Store;
    use wasmtime::component::Linker;

    let mut linker: Linker<TestStoreData> = Linker::new(engine);

    wasmtime_wasi::add_to_linker_sync(&mut linker)
        .map_err(|e| format!("WASI linker failed: {e}"))?;

    // If the WIT added/removed/renamed a function, stub registration
    // or instantiation will fail.
    {
        let mut root = linker.root();
        let mut host = root
            .instance("near:agent/host")
            .map_err(|e| format!("failed to create host instance: {e}"))?;
        stub_tool_host_functions(&mut host)?;
    }

    let mut store = Store::new(engine, TestStoreData::new());
    linker
        .instantiate(&mut store, component)
        .map_err(|e| format!("instantiation failed: {e}"))?;

    Ok(())
}

/// Instantiate a channel component (world: sandboxed-channel, imports: near:agent/channel-host).
fn instantiate_channel_component(
    engine: &wasmtime::Engine,
    component: &wasmtime::component::Component,
) -> Result<(), String> {
    use wasmtime::Store;
    use wasmtime::component::Linker;

    let mut linker: Linker<TestStoreData> = Linker::new(engine);

    wasmtime_wasi::add_to_linker_sync(&mut linker)
        .map_err(|e| format!("WASI linker failed: {e}"))?;

    {
        let mut root = linker.root();
        let mut host = root
            .instance("near:agent/channel-host")
            .map_err(|e| format!("failed to create channel-host instance: {e}"))?;

        // Shared tool host functions
        stub_tool_host_functions(&mut host)?;

        // Channel-specific host functions
        host.func_new("emit-message", |_ctx, _args, _results| Ok(()))
            .map_err(|e| format!("stub 'emit-message': {e}"))?;

        host.func_new("workspace-write", |_ctx, _args, results| {
            results[0] = wasmtime::component::Val::Result(Ok(None));
            Ok(())
        })
        .map_err(|e| format!("stub 'workspace-write': {e}"))?;

        host.func_new("pairing-upsert-request", |_ctx, _args, results| {
            results[0] = wasmtime::component::Val::Result(Err(Some(Box::new(
                wasmtime::component::Val::String("stub".into()),
            ))));
            Ok(())
        })
        .map_err(|e| format!("stub 'pairing-upsert-request': {e}"))?;

        host.func_new("pairing-is-allowed", |_ctx, _args, results| {
            results[0] = wasmtime::component::Val::Result(Err(Some(Box::new(
                wasmtime::component::Val::String("stub".into()),
            ))));
            Ok(())
        })
        .map_err(|e| format!("stub 'pairing-is-allowed': {e}"))?;

        host.func_new("pairing-read-allow-from", |_ctx, _args, results| {
            results[0] = wasmtime::component::Val::Result(Err(Some(Box::new(
                wasmtime::component::Val::String("stub".into()),
            ))));
            Ok(())
        })
        .map_err(|e| format!("stub 'pairing-read-allow-from': {e}"))?;
    }

    let mut store = Store::new(engine, TestStoreData::new());
    linker
        .instantiate(&mut store, component)
        .map_err(|e| format!("instantiation failed: {e}"))?;

    Ok(())
}

fn create_engine() -> wasmtime::Engine {
    let mut config = wasmtime::Config::new();
    config.wasm_component_model(true);
    config.wasm_threads(false);
    wasmtime::Engine::new(&config).expect("failed to create wasmtime engine")
}

#[test]
fn wit_compat_tool_components_compile_and_instantiate() {
    let extensions = discover_extensions();
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let engine = create_engine();

    let tool_extensions: Vec<_> = extensions
        .iter()
        .filter(|(_, source_dir, _)| source_dir.starts_with(repo_root.join("tools-src")))
        .collect();

    if tool_extensions.is_empty() {
        eprintln!("SKIP: no tool extensions found in registry");
        return;
    }

    let mut found_any = false;
    let mut failures: Vec<String> = Vec::new();

    for (name, source_dir, crate_name) in &tool_extensions {
        let wasm_path = match find_wasm_artifact(source_dir, crate_name) {
            Some(p) => p,
            None => {
                eprintln!(
                    "  SKIP {name}: no built WASM artifact (run ./scripts/build-wasm-extensions.sh)"
                );
                continue;
            }
        };

        found_any = true;
        eprintln!("  TEST {name}: {}", wasm_path.display());

        let wasm_bytes = std::fs::read(&wasm_path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", wasm_path.display()));

        let component = match compile_component(&engine, &wasm_bytes) {
            Ok(c) => c,
            Err(e) => {
                failures.push(format!("{name}: {e}"));
                continue;
            }
        };

        if let Err(e) = instantiate_tool_component(&engine, &component) {
            failures.push(format!("{name}: {e}"));
        }
    }

    if !found_any {
        eprintln!("SKIP: no WASM artifacts found (build extensions first)");
        return;
    }

    assert!(
        failures.is_empty(),
        "WIT compatibility failures for tools:\n{}",
        failures.join("\n")
    );
}

#[test]
fn wit_compat_channel_components_compile_and_instantiate() {
    let extensions = discover_extensions();
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let engine = create_engine();

    let channel_extensions: Vec<_> = extensions
        .iter()
        .filter(|(_, source_dir, _)| source_dir.starts_with(repo_root.join("channels-src")))
        .collect();

    if channel_extensions.is_empty() {
        eprintln!("SKIP: no channel extensions found in registry");
        return;
    }

    let mut found_any = false;
    let mut failures: Vec<String> = Vec::new();

    for (name, source_dir, crate_name) in &channel_extensions {
        let wasm_path = match find_wasm_artifact(source_dir, crate_name) {
            Some(p) => p,
            None => {
                eprintln!(
                    "  SKIP {name}: no built WASM artifact (run ./scripts/build-wasm-extensions.sh)"
                );
                continue;
            }
        };

        found_any = true;
        eprintln!("  TEST {name}: {}", wasm_path.display());

        let wasm_bytes = std::fs::read(&wasm_path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", wasm_path.display()));

        let component = match compile_component(&engine, &wasm_bytes) {
            Ok(c) => c,
            Err(e) => {
                failures.push(format!("{name}: {e}"));
                continue;
            }
        };

        if let Err(e) = instantiate_channel_component(&engine, &component) {
            failures.push(format!("{name}: {e}"));
        }
    }

    if !found_any {
        eprintln!("SKIP: no WASM artifacts found (build extensions first)");
        return;
    }

    assert!(
        failures.is_empty(),
        "WIT compatibility failures for channels:\n{}",
        failures.join("\n")
    );
}

#[test]
fn wit_compat_all_registry_extensions_have_source() {
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut missing = Vec::new();

    for dir in &["registry/tools", "registry/channels"] {
        let registry_dir = repo_root.join(dir);
        if !registry_dir.exists() {
            continue;
        }

        for entry in std::fs::read_dir(&registry_dir).unwrap().flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }

            let content = std::fs::read_to_string(&path).unwrap();
            let manifest: serde_json::Value = serde_json::from_str(&content).unwrap();

            let name = manifest["name"].as_str().unwrap_or("unknown");
            let source_dir = manifest["source"]["dir"].as_str();
            let crate_name = manifest["source"]["crate_name"].as_str();

            match (source_dir, crate_name) {
                (Some(d), Some(_)) => {
                    if !repo_root.join(d).exists() {
                        missing.push(format!("{name}: source dir '{d}' does not exist"));
                    }
                }
                _ => {
                    missing.push(format!("{name}: missing source.dir or source.crate_name"));
                }
            }
        }
    }

    assert!(
        missing.is_empty(),
        "Registry entries with missing sources:\n{}",
        missing.join("\n")
    );
}
