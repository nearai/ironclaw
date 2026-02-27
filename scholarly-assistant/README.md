# IronClaw Scholarly Assistant

Transform IronClaw into a PhD-level research assistant with academic paper search, literature review, and citation management capabilities.

## Overview

This directory contains tools and configurations to enhance IronClaw for academic research:

- **MCP Servers** - Semantic Scholar and ArXiv integration
- **Skills** - Literature review workflows
- **Tools** - PDF parsing and bibliography management specs
- **Setup Guides** - Complete installation instructions

## Quick Start

### 1. Install MCP Servers

```bash
# Install Python MCP servers
pip install semantic-scholar-mcp arxiv-mcp

# Start HTTP wrapper
python mcp_http_wrapper.py
```

### 2. Configure IronClaw

```bash
# Copy MCP server config
cp mcp-servers.json ~/.ironclaw/mcp-servers.json

# Install literature review skill
cp literature-review.skill.md ~/.ironclaw/skills/
```

### 3. Test Integration

```bash
ironclaw --cli-only -m "Search Semantic Scholar for papers about machine learning"
```

## Files

### Core Files
- **mcp-servers.json** - MCP server configuration for Semantic Scholar and ArXiv
- **mcp_http_wrapper.py** - HTTP bridge for Python MCP servers
- **test_http_wrapper.py** - Test suite for HTTP wrapper

### Skills & Workflows
- **literature-review.skill.md** - Systematic literature review skill for IronClaw

### Tool Specifications
- **pdf-parser-tool-spec.md** - PDF text extraction tool specification
- **bibliography-manager-tool-spec.md** - Citation management tool specification

### Documentation
- **SCHOLARLY_SETUP.md** - Complete setup instructions
- **TESTING_GUIDE.md** - Testing and troubleshooting guide

## Features

### 🔍 Academic Paper Search
- **Semantic Scholar** - 200M+ papers with citations, authors, abstracts
- **ArXiv** - Preprint repository for latest research

### 📚 Literature Review
- Systematic search strategies
- Paper selection and filtering
- Documentation and synthesis
- Citation tracking

### 📄 Document Processing
- PDF text extraction
- Bibliography management
- Citation formatting (BibTeX, APA, MLA, Chicago)

### 🤖 AI Integration
- IronClaw's WASM sandbox for security
- MCP protocol for tool interoperability
- Skills system for complex workflows

## Architecture

```
┌─────────────────┐
│    IronClaw     │
│   (WASM Host)   │
└────────┬────────┘
         │ HTTP
         ▼
┌─────────────────┐
│  HTTP Wrapper   │
│  (Port 3100)    │
└────────┬────────┘
         │ stdio
    ┌────┴────┐
    ▼         ▼
┌────────┐ ┌────────┐
│Semantic│ │ ArXiv  │
│Scholar │ │  MCP   │
│  MCP   │ │ Server │
└────────┘ └────────┘
```

## Use Cases

### Literature Review
```
Search for papers on "deep learning in medical imaging" published after 2020,
filter by citation count > 100, and create a structured literature review.
```

### Citation Management
```
Extract all citations from my research papers, format them in APA style,
and create a bibliography.
```

### Paper Analysis
```
Read the PDF at papers/research.pdf, extract key findings,
and compare with related work from Semantic Scholar.
```

## Prerequisites

- Python 3.8+
- IronClaw compiled with MCP support
- Internet connection for API access

## Installation

See **SCHOLARLY_SETUP.md** for detailed installation instructions.

## Testing

```bash
# Test HTTP wrapper
python test_http_wrapper.py

# Test MCP integration
curl http://localhost:3100/semantic-scholar -d '{"method":"tools/list","params":{}}'
```

## Troubleshooting

See **TESTING_GUIDE.md** for common issues and solutions.

## Future Enhancements

- [ ] PDF annotation support
- [ ] Reference graph visualization
- [ ] Collaborative filtering
- [ ] Automated literature monitoring
- [ ] Research paper summarization
- [ ] Cross-reference validation

## Contributing

This is designed for IronClaw's plugin architecture. To add new capabilities:

1. Create MCP server or WASM tool
2. Add to `mcp-servers.json` or `tools-src/`
3. Document in relevant `.md` file
4. Update this README

## Resources

- [Semantic Scholar API](https://api.semanticscholar.org/)
- [ArXiv API](https://arxiv.org/help/api)
- [Model Context Protocol](https://modelcontextprotocol.io/)
- [IronClaw Documentation](https://github.com/nearai/ironclaw)

## License

MIT OR Apache-2.0
