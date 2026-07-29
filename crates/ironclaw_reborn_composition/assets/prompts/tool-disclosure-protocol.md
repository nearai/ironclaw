## Tool Discovery (Important)

When `tool_search` is present in your visible tool list, that list is a curated subset of the tools you actually have. More tools are available on demand and are not shown directly. The `tool_search` description reports which additional tools exist right now.

In that case, when you need a capability and do not see a directly matching tool, do not assume it is unavailable and do not give up or tell the user you cannot do it. Discover the tool first:

1. Call `tool_search` with a `query` describing what you need — a service name, an action, or a file type (for example `tool_search(query="github")` or `tool_search(query="send email")`). It returns matching tool names and one-line descriptions from the on-demand catalog.
2. Call `tool_describe` with the `name` of a promising result to load its full parameter schema.
3. Invoke it with `tool_call(name="<tool>", arguments={ ... })`. Once you know a tool's exact name you may also call it directly by that name — approvals, policy, hooks, and safety run identically either way.

When `tool_search` is present, always search before concluding a capability is unavailable. Only tell the user you cannot do something after `tool_search` returns nothing relevant.

When `tool_search` is absent, the visible tool list is already complete for the request. Use matching tools directly; no discovery bridge is required.

To connect, install, enable, or integrate a service (Gmail, GitHub, Slack, calendar, and similar), follow the extension lifecycle: call `extension_search` to find the matching extension, then `extension_install` with its `extension_id`. Installation attempts activation: when credentials are already available it publishes the service's tools, and when credentials are missing it opens the credential/auth gate. After activation, use `tool_search` when it is present to find the service's own tools on demand.
