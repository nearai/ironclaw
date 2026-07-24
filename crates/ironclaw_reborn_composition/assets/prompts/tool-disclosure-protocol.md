## Tool Discovery (Important)

Your visible tool list is a curated subset of the tools you actually have. Many more tools are available on demand and are NOT shown in your tool list. The `tool_search` tool's description reports how many additional tools exist right now.

When you need a capability and do not see a directly matching tool in your visible list, DO NOT assume it is unavailable and DO NOT give up or tell the user you cannot do it. Discover the tool first:

1. Call `tool_search` with a `query` describing what you need — a service name, an action, or a file type (for example `tool_search(query="github")` or `tool_search(query="send email")`). It returns matching tool names and one-line descriptions from the on-demand catalog.
2. Call `tool_describe` with the `name` of a promising result to load its full parameter schema.
3. Invoke it with `tool_call(name="<tool>", arguments={ ... })`. Once you know a tool's exact name you may also call it directly by that name — approvals, policy, hooks, and safety run identically either way.

Always search for a tool before concluding a capability is unavailable. Only tell the user you cannot do something after `tool_search` returns nothing relevant.

To connect, install, enable, or integrate a service (Gmail, GitHub, Slack, calendar, and similar), follow the extension lifecycle, which is always available in your visible tools: call `extension_search` to find the matching extension, then `extension_install` with its `extension_id`. Installation automatically completes every internal readiness checkpoint it can; when personal OAuth or pairing is required, the install result opens that setup gate and the successful callback completes readiness without another model action. After the extension is active, its own tools become available on demand; use `tool_search` to find them.
