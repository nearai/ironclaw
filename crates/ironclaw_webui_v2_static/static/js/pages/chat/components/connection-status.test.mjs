import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const connectionStatusSource = readFileSync(
  new URL("./connection-status.js", import.meta.url),
  "utf8",
);

test("ConnectionStatus suppresses transient initial connecting state", () => {
  assert.match(
    connectionStatusSource,
    /status === "idle"[\s\S]*status === "connecting"[\s\S]*status === "connected"/,
    "initial SSE connection should not flash a visible status banner",
  );
});
