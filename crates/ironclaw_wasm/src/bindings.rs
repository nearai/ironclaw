#![allow(clippy::all)]

wasmtime::component::bindgen!({
    path: "../../wit/tool.wit",
    world: "sandboxed-tool",
    with: {},
});
