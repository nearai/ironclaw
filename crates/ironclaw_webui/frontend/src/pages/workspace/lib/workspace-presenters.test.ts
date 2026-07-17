import assert from "node:assert/strict";
import { test, vi } from "vitest";

import {
  areaDisplayName,
  formatWorkspaceFileSize,
  sortEntries,
} from "./workspace-presenters";

const LABELS = {
  "workspace.area.home": "Home",
  "workspace.area.memory": "Memory",
};
const t = (key) => LABELS[key] || key;

test("workspace area labels are translated without changing unknown backend ids", () => {
  assert.equal(areaDisplayName("workspace", t), "Home");
  assert.equal(areaDisplayName("memory", t), "Memory");
  assert.equal(areaDisplayName("future-area", t), "future-area");
  assert.equal(areaDisplayName("toString", t), "toString");
});

test("workspace root entries sort by their localized labels", () => {
  const entries = [
    { name: "workspace", path: "workspace", is_dir: true },
    { name: "memory", path: "memory", is_dir: true },
  ];

  assert.deepEqual(
    sortEntries(entries, (entry) => areaDisplayName(entry.path, t)).map((entry) => entry.path),
    ["workspace", "memory"],
  );
});

test("workspace entry sorting tolerates missing display names", () => {
  const entries = [
    { name: "report.txt", path: "report.txt", is_dir: false },
    { path: "unnamed", is_dir: false },
  ];

  assert.doesNotThrow(() => sortEntries(entries));
  assert.doesNotThrow(() => sortEntries(entries, () => undefined));
});

test("workspace file sizes use human-readable locale-aware units", () => {
  assert.equal(formatWorkspaceFileSize(120, "en"), "120 bytes");
  assert.equal(formatWorkspaceFileSize(5 * 1024 * 1024, "en"), "5 MB");
  assert.equal(formatWorkspaceFileSize(1536, "de"), "1,5 kB");
  assert.equal(formatWorkspaceFileSize(null, "en"), "");
  assert.equal(formatWorkspaceFileSize(undefined, "en"), "");
  assert.equal(formatWorkspaceFileSize(-1, "en"), "");
});

test("workspace file sizes retain a primitive fallback when Intl units are unavailable", () => {
  const formatter = vi.spyOn(Intl, "NumberFormat").mockImplementation(() => {
    throw new RangeError("unit formatting unavailable");
  });

  try {
    assert.equal(formatWorkspaceFileSize(1536, "de"), "1.5 KB");
    assert.equal(formatWorkspaceFileSize(1, "en"), "1 byte");
  } finally {
    formatter.mockRestore();
  }
});
