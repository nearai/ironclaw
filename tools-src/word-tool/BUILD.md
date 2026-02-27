# Building the Word Tool

## Prerequisites

1. **Rust toolchain** (1.70 or later)
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

2. **wasm32-wasip2 target**
   ```bash
   rustup target add wasm32-wasip2
   ```

3. **WIT files** (already included in `wit/` directory)

## Build Steps

### 1. Navigate to project directory

```bash
cd ~/Dev/agents/ironclaw/groups/main/word-tool
```

### 2. Build for WASM

```bash
cargo build --target wasm32-wasip2 --release
```

This will:
- Compile the Rust code to WebAssembly
- Apply Component Model bindings via `wit-bindgen`
- Optimize for size (LTO, strip, single codegen unit)
- Output to: `target/wasm32-wasip2/release/word_tool.wasm`

### 3. Verify the build

```bash
ls -lh target/wasm32-wasip2/release/word_tool.wasm
```

Expected output: ~1-3 MB WASM file

## Install in IronClaw

```bash
cd ~/Dev/agents/ironclaw/groups/main/ironclaw-build

./target/release/ironclaw tool install \
  ../word-tool/target/wasm32-wasip2/release/word_tool.wasm
```

This will:
- Copy the .wasm file to `~/.ironclaw/tools/`
- Register the tool in IronClaw's registry
- Load the capabilities from `word-tool.capabilities.json`

## Verify Installation

```bash
./target/release/ironclaw tool list
```

Should show `word-tool` in the list.

## Testing

Create a test .docx file:

```bash
mkdir -p ~/.ironclaw/documents
# Create a sample .docx file or copy one
cp /path/to/sample.docx ~/.ironclaw/documents/test.docx
```

Test reading:

```bash
./target/release/ironclaw --cli-only -m \
  '{"tool": "word-tool", "params": {"action": "read_docx", "path": "documents/test.docx"}}'
```

Test creating:

```bash
./target/release/ironclaw --cli-only -m \
  '{"tool": "word-tool", "params": {
    "action": "create_docx",
    "path": "documents/output.docx",
    "paragraphs": ["Hello World", "This is a test document"]
  }}'
```

## Troubleshooting

### Build fails with "cannot find target wasm32-wasip2"

Install the target:
```bash
rustup target add wasm32-wasip2
```

### Build fails with WIT binding errors

Make sure the `wit/` directory is present:
```bash
ls wit/tool.wit
```

If missing, copy from IronClaw repo:
```bash
cp -r ~/Dev/agents/ironclaw/wit ./
```

### Runtime error: "Tool not found"

Check installation:
```bash
ls ~/.ironclaw/tools/word_tool.wasm
cat ~/.ironclaw/tools/word_tool.capabilities.json
```

### Runtime error: "Fuel exhausted"

The document might be too large or complex. Check capabilities:
```json
{
  "capabilities": {
    "fuel": {
      "initial": 10000000,
      "max_per_execution": 50000000
    }
  }
}
```

Increase fuel limits if needed.

## Development

### Hot reload during development

```bash
# In one terminal, watch for changes
cargo watch -x 'build --target wasm32-wasip2 --release'

# In another, reinstall on each build
watch -n 2 'ironclaw tool install target/wasm32-wasip2/release/word_tool.wasm'
```

### Debug logging

Add to your code:
```rust
use near::agent::host::{log, LogLevel};

log(LogLevel::Info, "Processing document...");
```

View logs:
```bash
ironclaw --cli-only -m '...' 2>&1 | grep word-tool
```

## Clean Build

```bash
cargo clean
rm -rf target/
cargo build --target wasm32-wasip2 --release
```

## Size Optimization

The `.wasm` file is already optimized via Cargo profile:
- `opt-level = "s"` - Optimize for size
- `lto = true` - Link-time optimization
- `strip = true` - Strip debug symbols
- `codegen-units = 1` - Single codegen unit for better optimization

For further size reduction, use `wasm-opt`:

```bash
# Install wasm-opt
cargo install wasm-opt

# Optimize
wasm-opt -Oz \
  target/wasm32-wasip2/release/word_tool.wasm \
  -o target/wasm32-wasip2/release/word_tool_optimized.wasm
```

This can reduce size by 10-30%.
