import assert from "node:assert/strict";
import { test } from "vitest";

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

test("workspace file sizes use human-readable locale-aware units", () => {
  assert.equal(formatWorkspaceFileSize(120, "en"), "120 bytes");
  assert.equal(formatWorkspaceFileSize(5 * 1024 * 1024, "en"), "5 MB");
  assert.equal(formatWorkspaceFileSize(1536, "de"), "1,5 kB");
  assert.equal(formatWorkspaceFileSize(-1, "en"), "");
});
