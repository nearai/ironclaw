// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

import { hasChannelSurface } from "../../extensions/lib/extensions-schema";

function useChannelsSourceForTest() {
  const source = readFileSync(new URL("./useChannels.ts", import.meta.url), "utf8");
  const lines = [];
  let skippingImport = false;
  for (const line of source.split("\n")) {
    if (!skippingImport && line.startsWith("import ")) {
      skippingImport = !line.trimEnd().endsWith(";");
      continue;
    }
    if (skippingImport) {
      skippingImport = !line.trimEnd().endsWith(";");
      continue;
    }
    lines.push(line.replace(/^export function /, "function "));
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { useChannels };`;
}

function useChannelsForTest({ extensions, registry }) {
  const queryData = new Map([
    ["gateway-status-settings", { enabled_channels: ["http"] }],
    ["extensions", { extensions }],
    ["extension-registry", { entries: registry }],
  ]);
  const context = {
    globalThis: {},
    gatewayStatus: () => {},
    fetchExtensions: () => {},
    fetchExtensionRegistry: () => {},
    // The real surface-taxonomy helper, so channel grouping matches the
    // extensions page (and production) exactly.
    hasChannelSurface,
    useQuery: (config) => ({ data: queryData.get(config.queryKey[0]), isLoading: false }),
  };
  vm.runInNewContext(useChannelsSourceForTest(), context);
  return context.globalThis.__testExports.useChannels();
}

const channelSurfaces = [
  { kind: "tool" },
  { kind: "channel", inbound: true, outbound: true },
];
const toolSurfaces = [{ kind: "tool" }, { kind: "auth" }];

test("useChannels derives channels from the extension channel surface, not the retired kind wire string", () => {
  const slack = {
    package_ref: { id: "slack" },
    display_name: "Slack",
    runtime: "wasm",
    surfaces: channelSurfaces,
    installation_state: "setup_needed",
  };
  const github = {
    package_ref: { id: "github" },
    display_name: "GitHub",
    runtime: "wasm",
    surfaces: toolSurfaces,
  };
  // A hostile fixture wearing a retired `kind` string but declaring no
  // channel surface must NOT be grouped as a channel: the wire carries
  // runtime + surfaces, and `kind` no longer exists on it.
  const impostor = {
    package_ref: { id: "impostor" },
    display_name: "Impostor",
    kind: "channel",
    runtime: "wasm",
    surfaces: toolSurfaces,
  };

  const result = useChannelsForTest({
    extensions: [slack, github, impostor],
    registry: [],
  });

  assert.deepEqual(
    result.channels.map((channel) => channel.package_ref.id),
    ["slack"],
    "only surface-declared channels appear in the settings channels group",
  );
  assert.equal(result.status.enabled_channels[0], "http");
});

test("useChannels offers uninstalled channel-surface registry entries and nothing else", () => {
  const telegramEntry = {
    package_ref: { id: "telegram" },
    display_name: "Telegram",
    runtime: "wasm",
    surfaces: channelSurfaces,
    installed: false,
  };
  const installedSlackEntry = {
    package_ref: { id: "slack" },
    display_name: "Slack",
    runtime: "wasm",
    surfaces: channelSurfaces,
    installed: true,
  };
  const mcpToolEntry = {
    package_ref: { id: "github" },
    display_name: "GitHub",
    runtime: "mcp",
    surfaces: toolSurfaces,
    installed: false,
  };

  const result = useChannelsForTest({
    extensions: [],
    registry: [telegramEntry, installedSlackEntry, mcpToolEntry],
  });

  assert.deepEqual(
    result.channelRegistry.map((entry) => entry.package_ref.id),
    ["telegram"],
    "installed entries and tool-only extensions stay out of the available-channels group",
  );
});

test("useChannels exposes no runtime-grouped MCP rails: runtime is a badge, never a grouping axis", () => {
  const result = useChannelsForTest({ extensions: [], registry: [] });
  assert.equal(
    "mcpServers" in result,
    false,
    "the settings channels view must not resurrect an MCP-servers group keyed on runtime",
  );
  assert.equal("mcpRegistry" in result, false);
});
