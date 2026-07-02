import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

function threadCacheSourceForTest() {
  const source = readFileSync(
    new URL("./thread-cache.js", import.meta.url),
    "utf8",
  );
  const lines = [];
  for (const line of source.split("\n")) {
    if (line.startsWith("import ")) continue;
    lines.push(line.replace(/^export function /, "function "));
  }
  return `${lines.join("\n")}
globalThis.__testExports = {
  deriveSidebarTitle,
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
