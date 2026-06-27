import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function sourceForTest(path, exportNames) {
  const source = readFileSync(new URL(path, import.meta.url), "utf8");
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
  return `${lines.join("\n")}\nglobalThis.__testExports = { ${exportNames.join(", ")} };`;
}

function html(strings, ...values) {
  return { strings: Array.from(strings), values };
}

function visit(node, fn) {
  if (Array.isArray(node)) {
    for (const item of node) visit(item, fn);
    return;
  }
  if (!node || typeof node !== "object") return;
  fn(node);
  if (Array.isArray(node.values)) {
    for (const value of node.values) visit(value, fn);
  }
}

function collectScalars(root) {
  const scalars = [];
  visit(root, (node) => {
    if (!Array.isArray(node.values)) return;
    for (const value of node.values) {
      if (["string", "number", "boolean"].includes(typeof value)) {
        scalars.push(value);
      }
    }
  });
  return scalars;
}

function findComponentNodes(root, component) {
  const nodes = [];
  visit(root, (node) => {
    if (Array.isArray(node.values) && node.values.includes(component)) nodes.push(node);
  });
  return nodes;
}

function componentProps(node, component) {
  const props = {};
  const start = node.values.indexOf(component);
  for (let index = start + 1; index < node.values.length; index += 1) {
    const name = node.strings[index]?.match(/([A-Za-z][A-Za-z0-9]*)=\s*$/)?.[1];
    if (name) props[name] = node.values[index];
  }
  return props;
}

function renderChannelsTab({ searchQuery = "", channelState = {} } = {}) {
  const defaults = {
    status: { enabled_channels: ["http", "cli"], sse_connections: 2, ws_connections: 1 },
    channels: [
      {
        name: "slack",
        display_name: "Slack",
        description: "Team messages",
        onboarding_state: "ready",
      },
    ],
    channelRegistry: [
      { name: "telegram", display_name: "Telegram", description: "Mobile updates" },
    ],
    mcpServers: [
      { name: "filesystem", display_name: "Filesystem MCP", description: "Files", active: true },
    ],
    mcpRegistry: [
      { name: "github", display_name: "GitHub MCP", description: "Pull requests" },
    ],
    isLoading: false,
  };
  const context = {
    Badge: "Badge",
    Card: "Card",
    SettingsSearchEmpty: "SettingsSearchEmpty",
    globalThis: {},
    html,
    matchesSearch: (query, values) =>
      !query ||
      values.some((value) => String(value ?? "").toLowerCase().includes(query.toLowerCase())),
    useChannels: () => ({ ...defaults, ...channelState }),
    useT: () => (key) => key,
  };

  vm.runInNewContext(
    sourceForTest("./channels-tab.js", [
      "ChannelsTab",
      "BuiltinChannelCard",
      "ExtensionChannelCard",
      "deriveVisibleChannelGroups",
    ]),
    context,
  );
  return {
    exports: context.globalThis.__testExports,
    rendered: context.globalThis.__testExports.ChannelsTab({ searchQuery }),
  };
}

function renderUsersTab({ searchQuery = "", usersState = {} } = {}) {
  const defaults = {
    users: [
      {
        id: "u-1",
        display_name: "Ada Admin",
        email: "ada@example.com",
        role: "admin",
        status: "active",
        last_active: "2026-06-01T00:00:00Z",
      },
      {
        id: "u-2",
        display_name: "Mina Member",
        email: "mina@example.com",
        role: "member",
        status: "disabled",
      },
    ],
    query: { isLoading: false, error: null },
    isForbidden: false,
    createUser: () => {},
    createError: null,
    isCreating: false,
  };
  const context = {
    Badge: "Badge",
    Button: "Button",
    Card: "Card",
    FormField: "FormField",
    Icon: "Icon",
    Input: "Input",
    Label: "Label",
    React: {
      useState: (value) => [value, () => {}],
    },
    globalThis: {},
    html,
    matchesSearch: (query, values) =>
      !query ||
      values.some((value) => String(value ?? "").toLowerCase().includes(query.toLowerCase())),
    useT: () => (key, values = {}) => (values.count == null ? key : `${key}:${values.count}`),
    useUsers: () => ({ ...defaults, ...usersState }),
  };

  vm.runInNewContext(
    sourceForTest("./users-tab.js", ["UsersTab", "CreateUserForm", "UserRow"]),
    context,
  );
  return {
    exports: context.globalThis.__testExports,
    rendered: context.globalThis.__testExports.UsersTab({ searchQuery }),
  };
}

test("ChannelsTab groups built-in extension and MCP channels and filters by search", () => {
  const all = renderChannelsTab();
  const allScalars = collectScalars(all.rendered);
  const allExtensionCards = findComponentNodes(all.rendered, all.exports.ExtensionChannelCard)
    .map((node) => componentProps(node, all.exports.ExtensionChannelCard));

  assert.ok(allScalars.includes("channels.builtIn"));
  assert.ok(allScalars.includes("channels.messaging"));
  assert.ok(allScalars.includes("channels.mcpServers"));
  assert.ok(allScalars.includes("channels.webGateway"));
  assert.ok(allExtensionCards.some((props) => props.channel?.display_name === "Slack"));
  assert.ok(allExtensionCards.some((props) => props.registryEntry?.display_name === "Telegram"));
  assert.ok(allScalars.includes("Filesystem MCP"));
  assert.ok(allScalars.includes("GitHub MCP"));

  const filtered = renderChannelsTab({ searchQuery: "telegram" });
  const filteredScalars = collectScalars(filtered.rendered);
  const filteredExtensionCards = findComponentNodes(
    filtered.rendered,
    filtered.exports.ExtensionChannelCard,
  ).map((node) => componentProps(node, filtered.exports.ExtensionChannelCard));

  assert.ok(
    filteredExtensionCards.some((props) => props.registryEntry?.display_name === "Telegram"),
  );
  assert.equal(
    filteredExtensionCards.some((props) => props.channel?.display_name === "Slack"),
    false,
  );
  assert.equal(filteredScalars.includes("channels.builtIn"), false);
});

test("ChannelsTab renders loading and shared empty-search states", () => {
  const loading = renderChannelsTab({ channelState: { isLoading: true } });
  const empty = renderChannelsTab({ searchQuery: "nope" });

  assert.equal(findComponentNodes(loading.rendered, loading.exports.BuiltinChannelCard).length, 0);
  const emptyNode = findComponentNodes(empty.rendered, "SettingsSearchEmpty")[0];
  assert.equal(componentProps(emptyNode, "SettingsSearchEmpty").query, "nope");
});

test("UsersTab renders forbidden error list and filtered empty states", () => {
  const forbidden = renderUsersTab({ usersState: { isForbidden: true } });
  assert.ok(collectScalars(forbidden.rendered).includes("users.adminRequired"));

  const failed = renderUsersTab({
    usersState: { query: { isLoading: false, error: new Error("boom") } },
  });
  assert.ok(collectScalars(failed.rendered).includes("users.failedLoad"));

  const filtered = renderUsersTab({ searchQuery: "mina" });
  const filteredUsers = findComponentNodes(filtered.rendered, filtered.exports.UserRow)
    .map((node) => componentProps(node, filtered.exports.UserRow).user);
  assert.deepEqual(filteredUsers.map((user) => user.display_name), ["Mina Member"]);

  const empty = renderUsersTab({ searchQuery: "missing" });
  assert.ok(collectScalars(empty.rendered).includes("settings.noMatchingSettings"));
});
