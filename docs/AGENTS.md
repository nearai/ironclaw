> **Note**: For Mintlify product knowledge (components, configuration, writing standards),
> install the Mintlify skill: `npx skills add https://mintlify.com/docs`

# Documentation project instructions

## About this project

- This is a documentation site built on [Mintlify](https://mintlify.com)
- Pages are MDX files with YAML frontmatter
- Configuration lives in `docs.json`
- Run `mint dev` to preview locally
- Run `mint broken-links` to check links

## Terminology

- Use "agent" not "bot" for the AI assistant
- Use "channel" for input methods (TUI, web gateway, webhook)
- Use "job" for individual tasks the agent executes
- Use "skill" for SKILL.md prompt extensions
- Use "routine" for scheduled or reactive tasks
- Use "workspace" for the persistent memory system
- Use "sandbox" for Docker container execution environment

## Style preferences

- Use active voice and second person ("you")
- Keep sentences concise — one idea per sentence
- Use sentence case for headings
- Bold for UI elements: Click **Settings**
- Code formatting for file names, commands, paths, and code references

## Content boundaries

- Document user-facing features, configuration, and public APIs
- Don't document internal admin features or implementation details
- Focus on what users need to know to use IronClaw effectively
