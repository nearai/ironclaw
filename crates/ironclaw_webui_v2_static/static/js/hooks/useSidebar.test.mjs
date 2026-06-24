import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function useSidebarSourceForTest() {
  const source = readFileSync(new URL("./useSidebar.js", import.meta.url), "utf8");
  const lines = [];
  for (const line of source.split("\n")) {
    if (line.startsWith("import ")) continue;
    lines.push(line.replace(/^export function /, "function "));
  }
  return `${lines.join(
    "\n",
  )}\nglobalThis.__testExports = { readDesktopSidebarOpen, isDesktopSidebarViewport, toggleSidebarState, useSidebar };`;
}

function createLocalStorage(initial = {}) {
  const values = new Map(Object.entries(initial));
  return {
    getItem: (key) => (values.has(key) ? values.get(key) : null),
    setItem: (key, value) => values.set(key, String(value)),
    removeItem: (key) => values.delete(key),
    dump: () => Object.fromEntries(values.entries()),
  };
}

function createReactStub({ stateUpdates = [] } = {}) {
  return {
    useCallback: (fn) => fn,
    useEffect: (fn) => {
      fn();
    },
    useState: (initial) => {
      let value = typeof initial === "function" ? initial() : initial;
      return [
        value,
        (next) => {
          value = typeof next === "function" ? next(value) : next;
          stateUpdates.push(value);
        },
      ];
    },
  };
}

function instantiate({ isDesktop = true, storedOpen = null } = {}) {
  const stateUpdates = [];
  const navigations = [];
  const storage =
    storedOpen === null
      ? createLocalStorage()
      : createLocalStorage({ "ironclaw:v2-sidebar-open": storedOpen });
  const context = {
    React: createReactStub({ stateUpdates }),
    useNavigate: () => (path) => navigations.push(path),
    window: {
      localStorage: storage,
      matchMedia: () => ({ matches: isDesktop }),
    },
    globalThis: {},
  };
  vm.runInNewContext(useSidebarSourceForTest(), context);
  return {
    hook: context.globalThis.__testExports.useSidebar(),
    exports: context.globalThis.__testExports,
    navigations,
    stateUpdates,
    storage,
  };
}

function plain(value) {
  return JSON.parse(JSON.stringify(value));
}

test("readDesktopSidebarOpen defaults to open unless the stored value is false", () => {
  const { exports } = instantiate();
  assert.equal(exports.readDesktopSidebarOpen(), true);

  const closed = instantiate({ storedOpen: "false" });
  assert.equal(closed.exports.readDesktopSidebarOpen(), false);
});

test("toggleSidebarState targets only the active viewport state", () => {
  const { exports } = instantiate();
  assert.deepEqual(
    plain(exports.toggleSidebarState({ mobileOpen: false, desktopOpen: true }, true)),
    { mobileOpen: false, desktopOpen: false },
  );
  assert.deepEqual(
    plain(exports.toggleSidebarState({ mobileOpen: false, desktopOpen: true }, false)),
    { mobileOpen: true, desktopOpen: true },
  );
});

test("useSidebar toggles desktop visibility on desktop viewports", () => {
  const { hook, stateUpdates } = instantiate({ isDesktop: true, storedOpen: "true" });

  assert.equal(hook.desktopOpen, true);
  assert.equal(hook.mobileOpen, false);

  hook.toggle();

  assert.deepEqual(plain(stateUpdates.at(-1)), {
    mobileOpen: false,
    desktopOpen: false,
  });
});

test("useSidebar keeps mobile drawer behavior scoped to mobile viewports", () => {
  const { hook, navigations, stateUpdates } = instantiate({
    isDesktop: false,
    storedOpen: "true",
  });

  hook.toggle();
  assert.deepEqual(plain(stateUpdates.at(-1)), {
    mobileOpen: true,
    desktopOpen: true,
  });

  hook.selectThread("thread-1");

  assert.deepEqual(navigations, ["/chat/thread-1"]);
  assert.deepEqual(plain(stateUpdates.at(-1)), {
    mobileOpen: false,
    desktopOpen: true,
  });
});
