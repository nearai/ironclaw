# Word Tool - Quick Start Guide

Get up and running with Word document support in IronClaw in 5 minutes.

## Prerequisites

✅ Rust installed (`rustc --version`)
✅ IronClaw compiled at `~/Dev/agents/ironclaw/groups/main/ironclaw-build/`

## 3-Step Setup

### Step 1: Add WASM Target

```bash
rustup target add wasm32-wasip2
```

### Step 2: Build Word Tool

```bash
cd ~/Dev/agents/ironclaw/groups/main/word-tool
cargo build --target wasm32-wasip2 --release
```

⏱️ Takes ~2-5 minutes (downloads dependencies, compiles)

### Step 3: Install in IronClaw

```bash
cd ~/Dev/agents/ironclaw/groups/main/ironclaw-build
./target/release/ironclaw tool install \
  ../word-tool/target/wasm32-wasip2/release/word_tool.wasm
```

✅ Done! Word tool is now available.

## Test It

### Quick Test: Create a Document

Start IronClaw:
```bash
./target/release/ironclaw --cli-only
```

In the chat, ask:
```
Create a Word document called "test.docx" with two paragraphs:
"Hello World" and "This is a test."
```

IronClaw will:
1. Use the `word-tool` to generate .docx bytes
2. Use `write_file` to save it
3. Confirm creation

### Quick Test: Read a Document

Copy a sample .docx file:
```bash
mkdir -p ~/.ironclaw/documents
cp /path/to/sample.docx ~/.ironclaw/documents/
```

In IronClaw, ask:
```
Read the Word document at documents/sample.docx and summarize it.
```

## Usage Examples

### Create a Research Report

```
Create a Word document called "research_report.docx" with these sections:

Title: Literature Review on Machine Learning
Introduction: This review examines recent advances...
Methods: We searched papers from 2020-2026...
Results: Key findings include...
```

### Extract Text from Papers

```
Read all .docx files in my papers/ directory and create a summary
of the main topics discussed.
```

### Generate PhD Outline

```
Create a Word document "thesis_outline.docx" with my PhD structure:

Chapter 1: Introduction
Chapter 2: Literature Review
Chapter 3: Methodology
Chapter 4: Results
Chapter 5: Discussion
Chapter 6: Conclusions
```

## How It Works

**Reading .docx**:
1. You ask IronClaw to read a document
2. IronClaw calls `word-tool` with `action: read_docx`
3. Tool returns paragraphs, word count, character count
4. IronClaw shows you the content

**Creating .docx**:
1. You ask IronClaw to create a document
2. IronClaw calls `word-tool` with `action: create_docx`
3. Tool returns base64-encoded .docx bytes
4. IronClaw calls `write_file` to save it
5. Confirms creation

## What You Can Do

✅ Read .docx files (extract text, count words)
✅ Create .docx files (multiple paragraphs)
✅ Process multiple documents in batch
✅ Extract content for analysis
✅ Generate structured documents

## What's Not Supported (Yet)

❌ Bold/italic/formatting
❌ Images
❌ Tables
❌ Headers/footers
❌ Comments

(See `README.md` for future enhancements)

## Troubleshooting

**"Tool not found"**
```bash
# Check installation
ls ~/.ironclaw/tools/word_tool.wasm

# Reinstall if missing
./target/release/ironclaw tool install ../word-tool/target/wasm32-wasip2/release/word_tool.wasm
```

**"Failed to parse .docx"**
- File might be corrupted
- Try opening in Microsoft Word first
- Check file size (max 10MB)

**"File not found"**
- Paths are relative to `~/.ironclaw/`
- Use `documents/file.docx` not `/Users/you/documents/file.docx`

## Next Steps

📖 Read `README.md` for detailed API documentation
🔧 See `BUILD.md` for build troubleshooting
📝 Check `WORD_TOOL_SUMMARY.md` for architecture details

## Tips for PhD Work

### Organize Your Papers

```bash
mkdir -p ~/.ironclaw/papers/{downloaded,to_read,reviewed}
```

Move .docx papers there, then ask IronClaw:
```
Read all papers in papers/to_read/ and create a summary table
```

### Daily Writing Workflow

```
Create today's writing journal at documents/journal_2026-02-27.docx
with sections for: Research Progress, Ideas, Questions, Next Steps
```

### Literature Review

```
Read all papers in papers/reviewed/ and generate a literature
review document organizing findings by theme
```

## Support

Questions? Check:
- `README.md` - Full documentation
- `src/lib.rs` - Source code with comments
- IronClaw docs: `~/Dev/agents/ironclaw/src/tools/README.md`

Ready to build scholarly documents with IronClaw! 🎓📄
