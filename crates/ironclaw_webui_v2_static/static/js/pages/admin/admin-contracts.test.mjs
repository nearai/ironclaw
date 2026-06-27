import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

import {
  activateAdminUser,
  createAdminUser,
  createUserToken,
  deleteAdminUser,
  fetchAdminUser,
  fetchAdminUsers,
  fetchUsage,
  fetchUsageSummary,
  suspendAdminUser,
  updateAdminUser,
} from "./lib/admin-api.js";
import {
  aggregateUsageByModel,
  aggregateUsageByUser,
  filterUsers,
  formatCost,
  formatTokenCount,
  formatUptime,
  roleTone,
  statusTone,
  summarizeUsers,
  totalUsage,
  truncateId,
} from "./lib/admin-presenters.js";

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
    if (line.startsWith("export {")) {
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

function collectScalars(root) {
  const scalars = [];
  visit(root, (node) => {
    if (!Array.isArray(node.values)) return;
    for (const value of node.values) {
      if (typeof value === "string") scalars.push(value);
    }
  });
  return scalars;
}

function collectClickHandlers(root) {
  const handlers = [];
  visit(root, (node) => {
    if (!Array.isArray(node.strings) || !Array.isArray(node.values)) return;
    node.strings.forEach((part, index) => {
      if (part.includes("onClick=")) handlers.push(node.values[index]);
    });
  });
  return handlers;
}

function renderAdminPage({ tab } = {}) {
  const navigations = [];
  const stateSetters = [];
  const components = {
    DashboardTab: "DashboardTab",
    UsageTab: "UsageTab",
    UserDetail: "UserDetail",
    AdminUsersTab: "AdminUsersTab",
    Navigate: "Navigate",
  };
  const context = {
    ...components,
    globalThis: {},
    html,
    React: {
      useCallback: (fn) => fn,
      useState: (value) => {
        const setter = (next) => stateSetters.push(next);
        return [value, setter];
      },
    },
    useNavigate: () => (path) => navigations.push(path),
    useParams: () => (tab == null ? {} : { tab }),
  };

  vm.runInNewContext(sourceForTest("./admin-page.js", ["AdminPage"]), context);

  return {
    ...components,
    rendered: context.globalThis.__testExports.AdminPage(),
    navigations,
    stateSetters,
  };
}

function renderAdminTabs({ activeTab = "dashboard", mobile = false } = {}) {
  const calls = [];
  const context = {
    Icon: "Icon",
    globalThis: {},
    html,
    useT: () => (key) => key,
  };

  vm.runInNewContext(
    sourceForTest("./components/admin-tabs.js", [
      "ADMIN_TABS",
      "AdminTabs",
      "AdminTabsMobile",
    ]),
    context,
  );
  const component = mobile
    ? context.globalThis.__testExports.AdminTabsMobile
    : context.globalThis.__testExports.AdminTabs;

  return {
    rendered: component({ activeTab, onTabChange: (id) => calls.push(id) }),
    calls,
  };
}

test("AdminPage defaults to dashboard and redirects unsupported tabs", () => {
  const dashboard = renderAdminPage();
  const fallback = renderAdminPage({ tab: "missing" });
  const fallbackNavigate = findComponentNodes(fallback.rendered, fallback.Navigate)[0];

  assert.equal(findComponentNodes(dashboard.rendered, dashboard.DashboardTab).length, 1);
  assert.equal(
    fallbackNavigate.strings.join(""),
    "< to=\"/admin/dashboard\" replace />",
  );
});

test("AdminPage wires user drilldowns from dashboard and usage tabs", () => {
  const dashboard = renderAdminPage();
  const dashboardProps = componentProps(
    findComponentNodes(dashboard.rendered, dashboard.DashboardTab)[0],
    dashboard.DashboardTab,
  );
  dashboardProps.onSelectUser("user-1");
  dashboardProps.onNavigateTab("usage");

  assert.deepEqual(dashboard.stateSetters, ["user-1"]);
  assert.deepEqual(dashboard.navigations, ["/admin/users", "/admin/usage"]);

  const usage = renderAdminPage({ tab: "usage" });
  const usageProps = componentProps(
    findComponentNodes(usage.rendered, usage.UsageTab)[0],
    usage.UsageTab,
  );
  usageProps.onSelectUser("user-2");

  assert.deepEqual(usage.stateSetters, ["user-2"]);
  assert.deepEqual(usage.navigations, ["/admin/users"]);
});

test("AdminTabs expose dashboard users and usage navigation", () => {
  const desktop = renderAdminTabs({ activeTab: "users" });
  const mobile = renderAdminTabs({ activeTab: "usage", mobile: true });

  assert.deepEqual(
    collectScalars(desktop.rendered).filter((value) => value.startsWith("admin.tab.")),
    ["admin.tab.dashboard", "admin.tab.users", "admin.tab.usage"],
  );
  assert.deepEqual(
    collectScalars(mobile.rendered).filter((value) => value.startsWith("admin.tab.")),
    ["admin.tab.dashboard", "admin.tab.users", "admin.tab.usage"],
  );

  const handlers = collectClickHandlers(desktop.rendered);
  assert.equal(handlers.length, 3);
  handlers[2]();
  assert.deepEqual(desktop.calls, ["usage"]);
});

test("admin API stubs stay fail-closed and avoid legacy v1 fetches", async () => {
  const originalFetch = globalThis.fetch;
  let fetchCalled = false;
  globalThis.fetch = async () => {
    fetchCalled = true;
    throw new Error("admin v2 stubs must not call fetch");
  };

  try {
    assert.deepEqual(await fetchAdminUsers(), { users: [], total: 0, todo: true });
    assert.equal(await fetchAdminUser("user-1"), null);
    assert.deepEqual(await fetchUsage("day"), { entries: [], todo: true });
    assert.deepEqual(await fetchUsageSummary(), {
      total_users: 0,
      active_users: 0,
      suspended_users: 0,
      admin_users: 0,
      total_jobs: 0,
      llm_calls: 0,
      total_cost_usd: 0,
      active_jobs: 0,
      uptime_seconds: 0,
      recent_users: [],
      todo: true,
    });

    for (const action of [
      createAdminUser,
      updateAdminUser,
      deleteAdminUser,
      suspendAdminUser,
      activateAdminUser,
      createUserToken,
    ]) {
      const result = await action("user-1", {});
      assert.equal(result.success, false);
      assert.match(result.message, /v2 admin endpoint/);
    }
    assert.equal(fetchCalled, false);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("admin presenters format counts costs uptime ids and tones deterministically", () => {
  assert.equal(formatTokenCount(null), "0");
  assert.equal(formatTokenCount(999), "999");
  assert.equal(formatTokenCount(1_500), "1.5K");
  assert.equal(formatTokenCount(1_250_000), "1.3M");
  assert.equal(formatCost(null), "$0.00");
  assert.equal(formatCost("bad"), "$0.00");
  assert.equal(formatCost("12.345"), "$12.35");
  assert.equal(formatUptime(0), "0s");
  assert.equal(formatUptime(3_725), "1h 2m");
  assert.equal(formatUptime(90_000), "1d 1h");
  assert.equal(truncateId("abcdefghijklmnop"), "abcdefghijkl\u2026");
  assert.equal(statusTone("active"), "success");
  assert.equal(statusTone("suspended"), "danger");
  assert.equal(statusTone("unknown"), "muted");
  assert.equal(roleTone("admin"), "signal");
  assert.equal(roleTone("member"), "muted");
});

test("admin presenters filter users and aggregate usage by cost", () => {
  const users = [
    { id: "u-1", display_name: "Ada", email: "ada@example.com", role: "admin", status: "active" },
    { id: "u-2", display_name: "Grace", email: "grace@example.com", role: "member", status: "suspended" },
    { id: "u-3", display_name: "Linus", email: "linus@example.com", role: "member", status: "active" },
  ];
  assert.deepEqual(summarizeUsers(users), {
    total: 3,
    active: 2,
    suspended: 1,
    admins: 1,
  });
  assert.deepEqual(
    filterUsers(users, { filter: "admin" }).map((user) => user.id),
    ["u-1"],
  );
  assert.deepEqual(
    filterUsers(users, { filter: "active", search: "linus" }).map((user) => user.id),
    ["u-3"],
  );
  assert.deepEqual(
    filterUsers(users, { filter: "suspended", search: "example" }).map((user) => user.id),
    ["u-2"],
  );

  const entries = [
    { user_id: "u-1", model: "gpt-a", call_count: 2, input_tokens: 100, output_tokens: 25, total_cost: "0.40" },
    { user_id: "u-2", model: "gpt-b", call_count: 1, input_tokens: 60, output_tokens: 40, total_cost: "1.20" },
    { user_id: "u-1", model: "gpt-b", call_count: 3, input_tokens: 10, output_tokens: 5, total_cost: "0.30" },
  ];
  const byUser = aggregateUsageByUser(entries);
  const byModel = aggregateUsageByModel(entries);

  assert.deepEqual(byUser.map((row) => row.user_id), ["u-2", "u-1"]);
  assert.deepEqual(byModel.map((row) => row.model), ["gpt-b", "gpt-a"]);
  assert.deepEqual(totalUsage(byUser), {
    calls: 6,
    input_tokens: 170,
    output_tokens: 70,
    cost: 1.9,
  });
});
