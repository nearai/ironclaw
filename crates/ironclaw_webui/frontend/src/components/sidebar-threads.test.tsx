import assert from "node:assert/strict";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { test, vi } from "vitest";
import { deleteThreadErrorMessage } from "../lib/thread-errors";
import { runVmModuleForTest } from "../test-support/vm-module-harness";

vi.mock("react-router", async () => {
  const { createElement } = await import("react");
  return {
    NavLink: ({ children, to, onClick, className }) =>
      createElement(
        "a",
        {
          href: typeof to === "string" ? to : "#",
          onClick,
          className: typeof className === "function" ? className({ isActive: false }) : className,
        },
        children,
      ),
  };
});

vi.mock("../design-system/icons", async () => {
  const { createElement } = await import("react");
  return {
    Icon: ({ name, className }) =>
      createElement("span", { className, "data-icon": name }),
  };
});

vi.mock("../lib/i18n", () => ({ useT: () => (key) => key }));
vi.mock("../lib/pin-store", () => ({
  getPinnedIds: () => new Set(),
  subscribePins: () => () => {},
  togglePin: () => {},
}));
vi.mock("../lib/thread-state", () => ({
  THREAD_STATE: {
    FAILED: "failed",
    NEEDS_ATTENTION: "needs_attention",
    RUNNING: "running",
  },
  useThreadStates: () => new Map(),
}));

function createReactStub() {
  return {
    useCallback: (fn) => fn,
    useEffect: () => {},
    useMemo: (fn) => fn(),
    useState: (initial) => {
      let value = typeof initial === "function" ? initial() : initial;
      return [value, (next) => {
        value = typeof next === "function" ? next(value) : next;
      }];
    },
  };
}

function visitNode(node, fn) {
  if (Array.isArray(node)) {
    for (const item of node) visitNode(item, fn);
    return;
  }
  if (!node || typeof node !== "object") return;
  fn(node);
  visitNode(node.children, fn);
  visitNode(node.values, fn);
}

function expandComponents(node) {
  if (Array.isArray(node)) return node.map(expandComponents);
  if (!node || typeof node !== "object") return node;
  if (typeof node.type === "function") {
    return expandComponents(node.type({ ...node.props, children: node.children }));
  }
  return {
    ...node,
    children: expandComponents(node.children),
    values: expandComponents(node.values),
  };
}

function findNodeByProp(root, prop, value) {
  let found = null;
  visitNode(root, (node) => {
    if (!found && node.props?.[prop] === value) found = node;
  });
  assert.ok(found, `expected node with ${prop}=${value}`);
  return found;
}

function findNodeByType(root, type) {
  let found = null;
  visitNode(root, (node) => {
    if (!found && node.type === type) found = node;
  });
  assert.ok(found, `expected node with type=${type}`);
  return found;
}

function renderInteractiveSidebarThreads(props = {}, windowOverrides = {}) {
  function ConfirmDialog(dialogProps) {
    return { type: "confirm-dialog", props: dialogProps };
  }
  const context = {
    ConfirmDialog,
    React: createReactStub(),
    NavLink: "NavLink",
    Icon: "Icon",
    THREAD_STATE: {
      FAILED: "failed",
      NEEDS_ATTENTION: "needs_attention",
      RUNNING: "running",
    },
    byActivityDesc: (a, b) => (b.updated_at || "").localeCompare(a.updated_at || ""),
    cn: (...classes) => classes.flat().filter(Boolean).join(" "),
    console: { error: (...args) => context.errors.push(args) },
    deleteThreadErrorMessage,
    displaySidebarTitle: (thread, fallback) => thread.title || fallback,
    errors: [],
    formatThreadActivityLabel: () => "",
    formatThreadActivityTooltip: () => "",
    getPinnedIds: () => new Set(),
    subscribePins: () => () => {},
    threadActivityIso: (thread) => thread.updated_at || thread.created_at || null,
    togglePin: () => {},
    useT: () => (key) => key,
    useThreadStates: () => new Map(),
    window: {
      alert: () => {},
      ...windowOverrides,
    },
  };
  const { SidebarThreads } = runVmModuleForTest(
    "./sidebar-threads.tsx",
    ["SidebarThreads"],
    context,
    import.meta.url,
  );
  const rendered = expandComponents(
    SidebarThreads({
      threads: [
        {
          id: "thread-old",
          title: "Old investigation",
          created_at: "2026-07-01T12:00:00.000Z",
          updated_at: "2026-07-02T12:00:00.000Z",
        },
      ],
      activeThreadId: "thread-old",
      onSelect: () => {},
      onDelete: () => {},
      onNavigate: () => {},
      ...props,
    }),
  );
  return { context, rendered };
}

function clickEvent() {
  return {
    defaultPrevented: false,
    propagationStopped: false,
    preventDefault() {
      this.defaultPrevented = true;
    },
    stopPropagation() {
      this.propagationStopped = true;
    },
  };
}

async function flushPromises() {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

test("SidebarThreads exposes a visible delete action for listed threads", async () => {
  const { SidebarThreads } = await import("./sidebar-threads");
  const html = renderToStaticMarkup(
    <SidebarThreads
      threads={[
        {
          id: "thread-old",
          title: "Old investigation",
          created_at: "2026-07-01T12:00:00.000Z",
          updated_at: "2026-07-02T12:00:00.000Z",
        },
      ]}
      activeThreadId="thread-old"
      onSelect={() => {}}
      onDelete={() => {}}
      onNavigate={() => {}}
    />,
  );

  assert.match(html, /data-testid="thread-delete"/);
  assert.match(html, /data-thread-id="thread-old"/);
  assert.match(html, /aria-label="common.deleteChat"/);

  const deleteButton = html.match(/<button[^>]*data-testid="thread-delete"[^>]*>/)?.[0];
  assert.ok(deleteButton, "thread delete button should render in each thread row");
  assert.match(deleteButton, /\bopacity-70\b/);
  assert.doesNotMatch(
    deleteButton,
    /\bopacity-0\b/,
    "delete action must not be invisible until hover, because touch users cannot discover it",
  );
});

test("SidebarThreads surfaces delete handler failures from the delete button", async () => {
  const alerts = [];
  const deletions = [];
  const { context, rendered } = renderInteractiveSidebarThreads(
    {
      onDelete: async (threadId) => {
        deletions.push(threadId);
        throw { status: 409, payload: { kind: "busy" }, message: "Busy" };
      },
    },
    {
      alert: (message) => alerts.push(message),
    },
  );

  const deleteButton = findNodeByProp(rendered, "data-testid", "thread-delete");
  const event = clickEvent();
  deleteButton.props.onClick(event);
  assert.deepEqual(deletions, [], "opening the dialog must not delete the thread");

  const confirmDialog = findNodeByType(rendered, "confirm-dialog");
  confirmDialog.props.onConfirm();
  await flushPromises();

  assert.deepEqual(deletions, ["thread-old"]);
  assert.deepEqual(alerts, ["chat.deleteBusy"]);
  assert.equal(event.defaultPrevented, true);
  assert.equal(event.propagationStopped, true);
  assert.equal(context.errors.length, 1);
});
