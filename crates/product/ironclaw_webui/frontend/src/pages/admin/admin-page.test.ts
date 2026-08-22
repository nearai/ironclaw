// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";

import { runVmModuleForTest } from "../../test-support/vm-module-harness";

test("admin production code does not expose placeholder usage analytics", () => {
  const apiSource = readFileSync(new URL("./lib/admin-api.ts", import.meta.url), "utf8");
  const userDetailSource = readFileSync(
    new URL("./components/user-detail.tsx", import.meta.url),
    "utf8",
  );

  assert.doesNotMatch(apiSource, /fetchUsageSummary|fetchUsage/);
  assert.doesNotMatch(userDetailSource, /useUsage|admin\.usage\./);
});

function visit(node, fn) {
  if (Array.isArray(node)) {
    for (const item of node) visit(item, fn);
    return;
  }
  if (node == null || typeof node !== "object") return;
  fn(node);
  for (const value of Object.values(node)) visit(value, fn);
}

// First lazy import in admin-page.tsx is UserDetail; the later ones are the
// tab views. Each lazy() call gets a distinct stub so the tree walk can pick
// out the UserDetail element specifically.
let stubs = [];

function createAdminPageHarness() {
  stubs = [];
  const state = [];
  let cursor = 0;
  const React = {
    useState(initial) {
      const index = cursor;
      cursor += 1;
      if (!(index in state)) state[index] = typeof initial === "function" ? initial() : initial;
      // AdminPage starts with no selection; the forwarding we assert only
      // renders once a user is selected, so the harness seeds the selection.
      const value = index === 0 && state[index] === null ? "user-1" : state[index];
      return [
        value,
        (next) => {
          state[index] = typeof next === "function" ? next(state[index]) : next;
        },
      ];
    },
    useCallback(factory) {
      return factory();
    },
    // The lazy chunk loads through the real bundler; the VM sandbox only
    // needs the wrapper to render the underlying view component.
    lazy: () => {
      const stub = function LazyStub() {
        return null;
      };
      stubs.push(stub);
      return stub;
    },
  };
  const Navigate = () => null;
  const PageScroll = ({ children }) => children;
  const PageStack = ({ children }) => children;
  const router = {
    useNavigate: () => () => {},
    useParams: () => ({ tab: "users" }),
  };
  const RouteLoadBoundary = ({ children }) => children;
  return {
    React,
    Navigate,
    PageScroll,
    PageStack,
    ...router,
    RouteLoadBoundary,
    reset: () => {
      cursor = 0;
    },
  };
}

function loadAdminPage(harness) {
  return runVmModuleForTest("./admin-page.tsx", ["AdminPage"], harness, import.meta.url);
}

function renderAdminPage(harness, AdminPage, props) {
  harness.reset();
  return AdminPage(props);
}

function userDetailPropsFrom(tree) {
  let props = null;
  visit(tree, (node) => {
    if (!props && node.type === stubs[0]) props = node.props;
  });
  return props;
}

test("admin page forwards the thread scraping gate to the user detail view", () => {
  const harness = createAdminPageHarness();
  const { AdminPage } = loadAdminPage(harness);

  const enabledProps = userDetailPropsFrom(
    renderAdminPage(harness, AdminPage, { threadScrapingEnabled: true }),
  );
  assert.ok(enabledProps, "the user detail view must render inside the users tab");
  assert.equal(enabledProps.threadScrapingEnabled, true);

  const disabledProps = userDetailPropsFrom(
    renderAdminPage(harness, AdminPage, { threadScrapingEnabled: false }),
  );
  assert.equal(disabledProps.threadScrapingEnabled, false);
});
