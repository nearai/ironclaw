# IronClaw Scholarly Setup - Testing Guide

This guide will help you compile, configure, and test IronClaw with scholarly research capabilities.

## Prerequisites Check

Before starting, verify you have:

```bash
# Check Rust
rustc --version  # Should be 1.85+
cargo --version

# Check Python
python3 --version  # Should be 3.8+
pip3 --version

# Check Git
git --version
```

If missing, install:
- **Rust:** https://rustup.rs/
- **Python:** https://python.org/downloads/
- **Git:** https://git-scm.com/downloads/

## Step 1: Compile IronClaw

```bash
cd ~/Dev/agents/ironclaw  # Or wherever you cloned it

# Build in release mode
cargo build --release

# This will take a while (first time: 5-10 minutes)
# Binary will be at: target/release/ironclaw

# Optional: Add to PATH
export PATH="$PWD/target/release:$PATH"

# Verify
ironclaw --version
```

## Step 2: Setup Database

IronClaw requires PostgreSQL with pgvector extension:

```bash
# Install PostgreSQL (if not already)
# macOS:
brew install postgresql@15
brew services start postgresql@15

# Linux:
sudo apt-get install postgresql-15 postgresql-contrib-15

# Create database
createdb ironclaw

# Enable pgvector
psql ironclaw -c "CREATE EXTENSION IF NOT EXISTS vector;"

# Set DATABASE_URL
export DATABASE_URL="postgresql://localhost/ironclaw"
```

## Step 3: Run Onboarding

```bash
# First-time setup wizard
ironclaw onboard

# Follow prompts to configure:
# - Database connection
# - NEAR AI authentication (or OpenAI-compatible provider)
# - Secrets encryption
```

## Step 4: Install MCP Servers

### Semantic Scholar MCP Server

```bash
cd ~/Dev  # Or your preferred location

# Clone repository
git clone https://github.com/JackKuo666/semanticscholar-MCP-Server.git
cd semanticscholar-MCP-Server

# Install dependencies
pip3 install --user semanticscholar mcp

# Test it
python3 semantic_scholar_server.py
# Press Ctrl+C to stop
```

### ArXiv MCP Server

```bash
# Install with uv (recommended)
curl -LsSf https://astral.sh/uv/install.sh | sh
uv tool install arxiv-mcp-server

# Or with pip
pip3 install --user arxiv-mcp-server

# Test it
uv tool run arxiv-mcp-server
# Or: arxiv-mcp-server
# Press Ctrl+C to stop
```

## Step 5: Setup HTTP Wrapper

The MCP servers use stdio transport, but IronClaw's MCP client uses HTTP. We need a wrapper.

```bash
# Copy the wrapper script
cp /path/to/mcp_http_wrapper.py ~/Dev/

# Install aiohttp
pip3 install --user aiohttp

# Edit paths in the script if needed
nano ~/Dev/mcp_http_wrapper.py

# Update this line to point to your Semantic Scholar clone:
# semantic_scholar_path = Path.home() / "Dev/semanticscholar-MCP-Server"

# Test the wrapper
python3 ~/Dev/mcp_http_wrapper.py

# You should see:
# ✓ MCP HTTP wrapper running on http://localhost:3100
#   - Semantic Scholar: http://localhost:3100/semantic-scholar
#   - ArXiv: http://localhost:3100/arxiv

# Test endpoints (in another terminal):
curl http://localhost:3100/health

# Should return:
# {"status":"ok","servers":{"semantic_scholar":true,"arxiv":true}}
```

## Step 6: Configure IronClaw MCP Servers

```bash
# Create MCP servers config
mkdir -p ~/.ironclaw
cp /path/to/mcp-servers.json ~/.ironclaw/mcp-servers.json

# Verify content:
cat ~/.ironclaw/mcp-servers.json

# Should show:
# {
#   "servers": [
#     {
#       "name": "semantic_scholar",
#       "url": "http://localhost:3100/semantic-scholar",
#       ...
#     }
#   ]
# }
```

## Step 7: Install Literature Review Skill

```bash
# Copy skill
mkdir -p ~/.ironclaw/skills
cp /path/to/literature-review.skill.md ~/.ironclaw/skills/

# Verify
ls ~/.ironclaw/skills/
# Should show: literature-review.skill.md
```

## Step 8: Create Workspace Structure

```bash
# Create research workspace
mkdir -p ~/.ironclaw/workspace/{papers,bibliography,notes,experiments,writing}

# Verify structure
tree ~/.ironclaw/workspace/ -L 1
```

## Step 9: Start IronClaw

```bash
# In one terminal: Start HTTP wrapper
python3 ~/Dev/mcp_http_wrapper.py

# In another terminal: Start IronClaw
export RUST_LOG=ironclaw::tools::mcp=debug  # For debugging
ironclaw run

# Or use the TUI:
ironclaw tui
```

## Step 10: Test MCP Integration

In IronClaw, try these commands:

### Test 1: List Tools

```
> List available tools
```

**Expected:** You should see tools like:
- `semantic_scholar_search_papers`
- `semantic_scholar_get_paper_details`
- `semantic_scholar_get_author_details`
- `semantic_scholar_get_citations`
- `arxiv_search_papers`
- `arxiv_download_paper`
- `arxiv_list_papers`
- `arxiv_read_paper`

### Test 2: Search Papers

```
> Search Semantic Scholar for papers on "large language models" from 2023-2024
```

**Expected:** List of papers with titles, authors, years, citation counts

### Test 3: Get Paper Details

```
> Get details for the top paper including citations and references
```

**Expected:** Comprehensive paper information including abstract, citations, etc.

### Test 4: ArXiv Search

```
> Search ArXiv for recent papers on "transformer architecture"
```

**Expected:** List of ArXiv papers with IDs and metadata

### Test 5: Literature Review Skill Activation

```
> Help me conduct a literature review on neural networks
```

**Expected:** The literature-review skill should activate (check logs) and provide systematic guidance

## Troubleshooting

### Issue: Tools Not Appearing

**Check:**
```bash
# 1. Is HTTP wrapper running?
curl http://localhost:3100/health

# 2. Is MCP config correct?
cat ~/.ironclaw/mcp-servers.json

# 3. Check IronClaw logs
# Look for lines about MCP server initialization
```

**Fix:**
- Restart HTTP wrapper
- Verify URLs in config match (localhost:3100)
- Check wrapper logs for errors

### Issue: "Connection Refused"

**Cause:** HTTP wrapper not running or wrong port

**Fix:**
```bash
# Start wrapper
python3 ~/Dev/mcp_http_wrapper.py

# Check port
lsof -i :3100  # Should show Python process
```

### Issue: "Tool Execution Failed"

**Check wrapper logs:** The wrapper prints all requests/responses

**Common causes:**
- MCP server crashed (check wrapper terminal)
- Invalid parameters
- Rate limiting (Semantic Scholar: 100 req/5min)

### Issue: Semantic Scholar Path Not Found

**Fix:**
```python
# Edit mcp_http_wrapper.py
# Update this line:
semantic_scholar_path = Path.home() / "path/to/semanticscholar-MCP-Server"
```

### Issue: Skill Not Activating

**Check:**
```bash
# 1. Skill file exists
ls ~/.ironclaw/skills/literature-review.skill.md

# 2. Skill is valid YAML
head -20 ~/.ironclaw/skills/literature-review.skill.md

# 3. Check IronClaw logs for skill loading
```

**Fix:**
- Verify YAML frontmatter is valid
- Check activation keywords match your query
- Restart IronClaw to reload skills

## Performance Tips

### 1. Cache Paper Results

Store paper summaries in workspace to avoid re-fetching:

```
> Search for papers on [topic]
> Write a summary of the top 5 papers to workspace/papers/[topic]-papers.md
```

### 2. Use Memory Search

Before searching Semantic Scholar, check local memory:

```
> Search my workspace for papers on [topic]
```

### 3. Batch Operations

Get multiple paper details in one query:

```
> Get details for papers: [id1], [id2], [id3]
```

### 4. Monitor Rate Limits

Semantic Scholar free tier: 100 requests per 5 minutes

- Space out requests
- Cache results
- Get API key for higher limits: https://www.semanticscholar.org/product/api

## Testing Checklist

- [ ] IronClaw compiles successfully
- [ ] Database is set up and accessible
- [ ] Onboarding wizard completes
- [ ] HTTP wrapper starts without errors
- [ ] Health check endpoint responds
- [ ] MCP config file is in place
- [ ] Literature review skill is installed
- [ ] Workspace structure is created
- [ ] IronClaw starts successfully
- [ ] MCP tools appear in tool list
- [ ] Semantic Scholar search works
- [ ] ArXiv search works
- [ ] Paper details retrieval works
- [ ] Literature review skill activates
- [ ] Can store summaries in workspace

## Next Steps After Testing

Once everything works:

1. **Build PDF Parser**
   - Follow `pdf-parser-tool-spec.md`
   - Implement Python MCP server
   - Add to HTTP wrapper

2. **Build Bibliography Manager**
   - Follow `bibliography-manager-tool-spec.md`
   - Decide: WASM or built-in tool
   - Implement and test

3. **Set Up Routines**
   - Daily paper monitoring
   - Citation alerts
   - Progress tracking

4. **Start Research!**
   - Search for papers on your PhD topic
   - Store summaries
   - Build bibliography
   - Synthesize findings

## Getting Help

### IronClaw Issues
- GitHub: https://github.com/nearai/ironclaw/issues
- Logs: `RUST_LOG=ironclaw=debug ironclaw run`

### MCP Server Issues
- Semantic Scholar: https://github.com/JackKuo666/semanticscholar-MCP-Server/issues
- ArXiv: https://github.com/blazickjp/arxiv-mcp-server/issues

### HTTP Wrapper Issues
- Check wrapper terminal for error messages
- Test endpoints directly with curl
- Verify Python dependencies installed

## Environment Variables Reference

```bash
# Database
export DATABASE_URL="postgresql://localhost/ironclaw"

# Logging
export RUST_LOG=ironclaw=debug              # All debug logs
export RUST_LOG=ironclaw::tools::mcp=debug  # Just MCP logs

# API Keys (if needed)
export OPENAI_API_KEY="sk-..."              # For embeddings
export NEARAI_API_KEY="..."                 # For LLM (alternative)
```

## Success Criteria

You'll know it's working when:

1. ✅ IronClaw starts without errors
2. ✅ HTTP wrapper shows both servers running
3. ✅ `curl http://localhost:3100/health` returns OK
4. ✅ MCP tools appear in IronClaw's tool list
5. ✅ Can search Semantic Scholar successfully
6. ✅ Can search ArXiv successfully
7. ✅ Literature review skill activates on research queries
8. ✅ Can store paper summaries in workspace

---

*Testing Guide by Andy*
*For Joaquín's PhD Project*
*Last Updated: 2026-02-27*
