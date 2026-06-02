import assert from "node:assert/strict";
import test from "node:test";

import { primaryExtensionAction } from "./extension-actions.js";

const notionRef = { kind: "extension", id: "notion" };

test("primaryExtensionAction opens configuration before OAuth-required activation", () => {
  assert.equal(
    primaryExtensionAction({
      package_ref: notionRef,
      kind: "mcp_server",
      onboarding_state: "auth_required",
    }),
    "configure",
  );
});

test("primaryExtensionAction activates configured inactive MCP extensions", () => {
  assert.equal(
    primaryExtensionAction({
      package_ref: notionRef,
      kind: "mcp_server",
      activation_status: "installed",
    }),
    "activate",
  );
});

test("primaryExtensionAction hides activation for active extensions", () => {
  assert.equal(
    primaryExtensionAction({
      package_ref: notionRef,
      kind: "mcp_server",
      active: true,
    }),
    null,
  );
});
