// @ts-nocheck
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import vm from "node:vm";

function threadCacheSourceForTest() {
  const titleSource = readFileSync(
    new URL("../../../lib/thread-title.ts", import.meta.url),
    "utf8",
  );
  const source = readFileSync(
    new URL("./thread-cache.ts", import.meta.url),
    "utf8",
  );
  const lines = [];
  for (const line of `${titleSource}\n${source}`.split("\n")) {
    if (line.startsWith("import ")) continue;
    lines.push(line.replace(/^export function /, "function "));
  }
  return `${lines.join("\n")}
globalThis.__testExports = {
  deriveSidebarTitle,
  displaySidebarTitle,
  normalizeSidebarTitle,
  removeThreadList,
  touchThreadList,
  upsertThreadList,
};`;
}

function loadThreadCache() {
  const context = { Array, Date, String, globalThis: {} };
  vm.runInNewContext(threadCacheSourceForTest(), context);
  return context.globalThis.__testExports;
}

function normalize(value) {
  return JSON.parse(JSON.stringify(value));
}

test("deriveSidebarTitle uses the first non-empty line", () => {
  const { deriveSidebarTitle } = loadThreadCache();

  assert.equal(
    deriveSidebarTitle("\n\n  hello thread  \nsecond line"),
    "hello thread",
  );
  assert.equal(deriveSidebarTitle("   "), null);
});

test("deriveSidebarTitle truncates long titles", () => {
  const { deriveSidebarTitle } = loadThreadCache();
  const title = deriveSidebarTitle("a".repeat(70));

  assert.equal(title.length, 60);
  assert.equal(title, `${"a".repeat(57)}...`);
});

test("normalizeSidebarTitle treats raw thread ids as missing titles", () => {
  const { displaySidebarTitle, normalizeSidebarTitle } = loadThreadCache();

  assert.equal(normalizeSidebarTitle("thread_abc123456", "thread_abc123456"), null);
  assert.equal(normalizeSidebarTitle("thread-abc123456", "thread-abc123456"), null);
  assert.equal(normalizeSidebarTitle("  Weekly status  ", "thread_abc123456"), "Weekly status");
  assert.equal(normalizeSidebarTitle("thread-safety", "thread_abc123456"), "thread-safety");
  assert.equal(normalizeSidebarTitle("thread_pool", "thread_abc123456"), "thread_pool");
  assert.equal(
    displaySidebarTitle({ thread_id: "thread_abc123456", title: "thread_abc123456" }),
    "Untitled thread",
  );
});

test("upsertThreadList inserts a created thread and preserves pagination metadata", () => {
  const { upsertThreadList } = loadThreadCache();
  const data = { threads: [{ thread_id: "thread-old" }], next_cursor: "cursor-1" };

  assert.deepEqual(
    normalize(upsertThreadList(data, { thread_id: "thread-new", title: "New" })),
    {
      threads: [
        { thread_id: "thread-new", title: "New" },
        { thread_id: "thread-old" },
      ],
      next_cursor: "cursor-1",
    },
  );
});

test("upsertThreadList merges an existing thread record", () => {
  const { upsertThreadList } = loadThreadCache();
  const data = {
    threads: [
      { thread_id: "thread-newer", title: "Newer" },
      { thread_id: "thread-1", title: null, created_at: "before" },
      { thread_id: "thread-older", title: "Older" },
    ],
    next_cursor: null,
  };

  assert.deepEqual(
    normalize(upsertThreadList(data, { thread_id: "thread-1", title: "Updated" })),
    {
      threads: [
        { thread_id: "thread-1", title: "Updated", created_at: "before" },
        { thread_id: "thread-newer", title: "Newer" },
        { thread_id: "thread-older", title: "Older" },
      ],
      next_cursor: null,
    },
  );
});

test("upsertThreadList preserves cached title when incoming record has raw thread id", () => {
  const { upsertThreadList } = loadThreadCache();
  const data = {
    threads: [
      { thread_id: "thread_abc123456", title: "Cached name", updated_at: "before" },
    ],
    next_cursor: null,
  };

  assert.deepEqual(
    normalize(
      upsertThreadList(data, {
        thread_id: "thread_abc123456",
        title: "thread_abc123456",
        updated_at: "after",
      }),
    ),
    {
      threads: [
        { thread_id: "thread_abc123456", title: "Cached name", updated_at: "after" },
      ],
      next_cursor: null,
    },
  );
});

test("removeThreadList removes only the matching cached thread", () => {
  const { removeThreadList } = loadThreadCache();
  const data = {
    threads: [
      { thread_id: "thread-keep", title: "Keep" },
      { thread_id: "thread-drop", title: "Drop" },
    ],
    next_cursor: "cursor-1",
  };

  assert.deepEqual(normalize(removeThreadList(data, "thread-drop")), {
    threads: [{ thread_id: "thread-keep", title: "Keep" }],
    next_cursor: "cursor-1",
  });
});

test("removeThreadList preserves cache identity when the thread is absent", () => {
  const { removeThreadList } = loadThreadCache();
  const data = { threads: [{ thread_id: "thread-keep" }], next_cursor: null };

  assert.equal(removeThreadList(data, "thread-missing"), data);
  assert.equal(removeThreadList(undefined, "thread-missing"), undefined);
});

test("touchThreadList updates activity without overwriting an existing title", () => {
  const { touchThreadList } = loadThreadCache();
  const data = {
    threads: [
      { thread_id: "thread-newer", title: "Newer", updated_at: "newer" },
      { thread_id: "thread-1", title: "Existing", updated_at: "before" },
      { thread_id: "thread-older", title: "Older", updated_at: "older" },
    ],
    next_cursor: "cursor-1",
  };

  assert.deepEqual(
    normalize(
      touchThreadList(data, {
        threadId: "thread-1",
        messageContent: "new first message",
        updatedAt: "after",
      }),
    ),
    {
      threads: [
        { thread_id: "thread-1", title: "Existing", updated_at: "after" },
        { thread_id: "thread-newer", title: "Newer", updated_at: "newer" },
        { thread_id: "thread-older", title: "Older", updated_at: "older" },
      ],
      next_cursor: "cursor-1",
    },
  );
});

test("touchThreadList creates a minimal missing thread record", () => {
  const { touchThreadList } = loadThreadCache();

  assert.deepEqual(
    normalize(
      touchThreadList(undefined, {
        threadId: "thread-new",
        messageContent: "hello world",
        updatedAt: "now",
      }),
    ),
    {
      threads: [
        {
          thread_id: "thread-new",
          title: "hello world",
          created_at: "now",
          updated_at: "now",
        },
      ],
      next_cursor: null,
    },
  );
});
