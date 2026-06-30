import assert from "node:assert/strict";
import test from "node:test";

import { classifyRisk } from "./approval-risk.js";

test("classifyRisk: command tool name wins over read-ish description", () => {
  assert.deepEqual(
    classifyRisk("bash", "reads files safely", "{}"),
    { tone: "warning", key: "tool.riskExec" },
  );
});

test("classifyRisk: read tool mentioning edit in description is not danger", () => {
  assert.deepEqual(
    classifyRisk("read", "Inspect edit history without changing files", "{}"),
    { tone: "muted", key: "tool.riskRead" },
  );
});

test("classifyRisk: write-like tool name is danger", () => {
  assert.deepEqual(
    classifyRisk("write_file", "Writes a file", "{}"),
    { tone: "danger", key: "tool.riskWrite" },
  );
});
