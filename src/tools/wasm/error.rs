//! Re-export shim: WASM error type lives in `ironclaw_common::ext_error`.

pub use ironclaw_common::ext_error::WasmError;

impl From<WasmError> for crate::tools::ToolError {
    fn from(e: WasmError) -> Self {
        crate::tools::ToolError::Sandbox(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use crate::tools::wasm::error::WasmError;

    #[test]
    fn test_conversion_to_tool_error() {
        let wasm_err = WasmError::Trapped("test trap".to_string());
        let tool_err: crate::tools::ToolError = wasm_err.into();
        match tool_err {
            crate::tools::ToolError::Sandbox(msg) => {
                assert!(msg.contains("test trap"));
            }
            _ => panic!("Expected Sandbox variant"),
        }
    }
}
