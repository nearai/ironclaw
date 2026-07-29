---
title: "Web Access"
sidebarTitle: "Web Search"
description: "Let your agent search the web and read pages"
icon: globe
---

The Web Access extension lets your agent search the web and retrieve full page content,
so it can answer questions about current events, look up specific data, and cite its
sources.

It needs no credentials. There is no API key to obtain and no account to create — install
it, activate it, and the tools are available.

---

## Setup

<Steps>

<Step title="Install the extension">

```bash
ironclaw extension install web-access
```

<Note>
The extension id is `web-access`, not `web-search`. Run `ironclaw extension search web`
to confirm the id your install knows about.
</Note>

</Step>

<Step title="Activate it">

```bash
ironclaw extension activate web-access
```

Activation publishes the tools to the agent. Because no credentials are involved, there
is nothing to supply afterwards.

</Step>

</Steps>

You can do both steps from **Extensions** in the [web interface](/using/webui) instead.

---

## What the Agent Can Do

| Capability | Description |
| --- | --- |
| `web-access.search` | Search the web and return results with citations |
| `web-access.get_content` | Retrieve a page's full content, or re-read a page cached from an earlier search |

Both are allowed by default once the extension is active.

<Tip>
For GitHub repositories, issues, pull requests, releases, or workflow data, the
[GitHub extension](/extensions/github) returns better-structured results than a web
search. Install it if your agent works with GitHub regularly.
</Tip>
