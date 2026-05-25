//! WASM hook execution path for Installed-tier hooks.
//!
//! The ABI is a small, hand-linked `wasmtime::Linker` surface. It deliberately
//! mirrors the tool-WASM runtime's fresh-store model while exposing only typed
//! hook sinks: no filesystem, network, wall clock, RNG, or WASI imports.

mod runtime;

pub use runtime::{
    WasmBeforeCapabilityHook, WasmBeforePromptHook, WasmHookFailure, WasmHookModuleRequest,
    WasmHookModuleResolver, WasmHookRuntime, WasmHookRuntimeError, WasmObserverHook,
};
