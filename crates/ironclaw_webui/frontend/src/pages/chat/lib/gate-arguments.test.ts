import assert from "node:assert/strict";
import { test } from "vitest";

import { enrichApprovalGateWithActivityArguments } from "./gate-arguments";

test("adds matching activity arguments by invocationId", () => {
  const gate = {
    kind: "gate",
    invocationId: "invocation-search",
    approvalDetails: [{ label: "Capability", value: "nearai.web_search" }],
  };

  const enriched = enrichApprovalGateWithActivityArguments(gate, [
    {
      role: "tool_activity",
      invocationId: "invocation-search",
      toolParameters: "query: deploy status",
    },
  ]);

  assert.deepEqual(JSON.parse(JSON.stringify(enriched.approvalDetails)), [
    { label: "Capability", value: "nearai.web_search" },
    { label: "Arguments", value: "query: deploy status" },
  ]);
  assert.equal(enriched.parameters, "query: deploy status");
});

test("does not duplicate an existing arguments detail, but fills parameters", () => {
  const gate = {
    kind: "gate",
    invocationId: "invocation-search",
    approvalDetails: [{ label: "Parameters", value: "query: existing" }],
  };

  const enriched = enrichApprovalGateWithActivityArguments(gate, [
    {
      role: "tool_activity",
      invocationId: "invocation-search",
      toolParameters: "query: deploy status",
    },
  ]);

  assert.deepEqual(enriched.approvalDetails, [
    { label: "Parameters", value: "query: existing" },
  ]);
  assert.equal(enriched.parameters, "query: deploy status");
});

test("strict join: a gate without an invocationId is never enriched", () => {
  const gate = {
    kind: "gate",
    runId: "run-1",
    description: "capability requires approval",
    approvalDetails: [],
  };

  const enriched = enrichApprovalGateWithActivityArguments(gate, [
    {
      role: "tool_activity",
      turnRunId: "run-1",
      invocationId: "invocation-search",
      toolParameters: "query: total market cap",
    },
  ]);

  assert.equal(enriched, gate);
});

test("strict join: does not borrow another invocation's arguments in the same run", () => {
  const gate = {
    kind: "gate",
    runId: "run-1",
    invocationId: "invocation-http",
    approvalDetails: [{ label: "Method", value: "GET" }],
  };

  const enriched = enrichApprovalGateWithActivityArguments(gate, [
    {
      role: "tool_activity",
      turnRunId: "run-1",
      invocationId: "invocation-search",
      toolParameters: "query: deploy status",
    },
    {
      role: "tool_activity",
      turnRunId: "run-1",
      invocationId: "invocation-http",
      toolStatus: "running",
    },
  ]);

  assert.deepEqual(JSON.parse(JSON.stringify(enriched.approvalDetails)), [
    { label: "Method", value: "GET" },
  ]);
});

test("matches a nested toolCalls activity by invocationId", () => {
  const gate = {
    kind: "gate",
    invocationId: "invocation-nested",
    approvalDetails: [],
  };

  const enriched = enrichApprovalGateWithActivityArguments(gate, [
    {
      role: "assistant",
      toolCalls: [
        { invocationId: "invocation-nested", toolParameters: "path: /tmp" },
      ],
    },
  ]);

  assert.deepEqual(JSON.parse(JSON.stringify(enriched.approvalDetails)), [
    { label: "Arguments", value: "path: /tmp" },
  ]);
});

test("non-gate prompts pass through unchanged", () => {
  const gate = { kind: "auth_required", invocationId: "x" };
  assert.equal(enrichApprovalGateWithActivityArguments(gate, []), gate);
});
