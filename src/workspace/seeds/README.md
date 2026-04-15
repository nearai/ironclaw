# Workspace

This is your agent's persistent memory. Files here are indexed for search
and used to build the agent's context.

## Structure

- `MEMORY.md` - Long-term curated notes (loaded into system prompt)
- `IDENTITY.md` - "Who you are" — agent name, vibe, personality (injected first)
- `SOUL.md` - Core values, org personality, behavioral boundaries
- `AGENTS.md` - Session routine, scenario duties, operational instructions
- `USER.md` - Information about you (the user) (injected last among identity files)
- `TOOLS.md` - Environment-specific tool notes
- `HEARTBEAT.md` - Periodic background task checklist
- `daily/` - Automatic daily session logs
- `context/` - Additional context documents

Edit these files to shape how your agent thinks and acts.
The agent reads them at the start of every session.