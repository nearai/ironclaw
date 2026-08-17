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
