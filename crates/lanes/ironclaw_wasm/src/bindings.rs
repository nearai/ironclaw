#![allow(clippy::all)]

/// Current (near:agent@0.4.0) WIT bindings. Re-exported at this module's
/// root so existing `crate::bindings::*` call sites keep resolving to the
/// current world without change.
mod current {
    wasmtime::component::bindgen!({
        path: "wit/tool.wit",
        world: "sandboxed-tool",
        with: {},
    });
}
pub(crate) use current::*;

/// Frozen legacy (near:agent@0.3.0) WIT bindings.
///
/// The host runtime tries the current world first (see
/// `WitToolRuntime::prepare`); a component compiled against the older
/// package version fails that instantiation on an import/version mismatch,
/// and the runtime falls back to instantiating against this frozen world so
/// already-bundled 0.3.0 guest components keep working while guests migrate
/// to the typed 0.4.0 response. Removed in PR 4, once every guest has
/// migrated.
pub(crate) mod legacy {
    wasmtime::component::bindgen!({
        path: "wit/legacy/tool_v0_3_0.wit",
        world: "sandboxed-tool",
        with: {},
    });
}
