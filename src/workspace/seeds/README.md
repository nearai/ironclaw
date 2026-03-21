# Workspace

This is your agent's persistent memory. Files here are indexed for search
and used to build the agent's context.

## Structure

- `MEMORY.md` - Long-term curated notes (loaded into system prompt)
- `IDENTITY.md` - Agent name, vibe, personality
- `SOUL.md` - Core values and behavioral boundaries
- `AGENTS.md` - Session routine and operational instructions
- `USER.md` - Information about you (the user)
- `TOOLS.md` - Environment-specific tool notes
- `HEARTBEAT.md` - Periodic background task checklist
- `daily/` - Automatic daily session logs
- `context/` - Additional context documents

Edit these files to shape how your agent thinks and acts.
The agent reads them at the start of every session.