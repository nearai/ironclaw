#![allow(clippy::all)]

wasmtime::component::bindgen!({
    path: "wit/product_adapter.wit",
    world: "product-adapter-component",
    with: {},
});
