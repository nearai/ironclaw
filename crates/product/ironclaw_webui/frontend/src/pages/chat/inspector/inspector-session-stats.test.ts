import assert from "node:assert/strict";
import { test } from "vitest";

import {
  InspectorSessionStatsAccumulator,
  decodeSessionDiagnosticStats,
  type SessionDiagnosticStats,
} from "./inspector-session-stats";

function stats({
  model,
  calls,
  inputTokens,
  unavailableInputs = 0,
  tools = 0,
}: {
  model: string;
  calls: number;
  inputTokens: number;
  unavailableInputs?: number;
  tools?: number;
}): SessionDiagnosticStats {
  return {
    total_model_calls: calls,
    total_tool_calls: tools,
    successful_tool_calls: tools,
    failed_tool_calls: 0,
    calls_per_model: [{
      model: { content: model, original_bytes: model.length, truncated: false },
      calls,
    }],
    calls_per_model_truncated: false,
    input_tokens: {
      known_total: inputTokens,
      unavailable_samples: unavailableInputs,
    },
    output_tokens: { known_total: calls * 5, unavailable_samples: 0 },
    cache_read_input_tokens: { known_total: 0, unavailable_samples: calls },
    cache_creation_input_tokens: { known_total: 0, unavailable_samples: calls },
    total_latency_ms: { known_total: calls * 100, unavailable_samples: 0 },
  };
}

test("page-session stats accumulate distinct runs without double-counting refreshes", () => {
  const accumulator = new InspectorSessionStatsAccumulator();
  assert.equal(accumulator.snapshot("operator-a"), null);

  accumulator.record("operator-a", "thread-a/run-1", stats({
    model: "model-a",
    calls: 1,
    inputTokens: 10,
    tools: 1,
  }));
  let total = accumulator.record("operator-a", "thread-a/run-2", stats({
    model: "model-b",
    calls: 2,
    inputTokens: 20,
    unavailableInputs: 1,
    tools: 2,
  }));

  assert.equal(total.total_model_calls, 3);
  assert.equal(total.total_tool_calls, 3);
  assert.equal(total.input_tokens.known_total, 30);
  assert.equal(total.input_tokens.unavailable_samples, 1);
  assert.deepEqual(
    total.calls_per_model.map((entry) => [entry.model.content, entry.calls]),
    [["model-a", 1], ["model-b", 2]],
  );

  total = accumulator.record("operator-a", "thread-a/run-2", stats({
    model: "model-b",
    calls: 3,
    inputTokens: 35,
    tools: 2,
  }));
  assert.equal(total.total_model_calls, 4);
  assert.equal(total.total_tool_calls, 3);
  assert.equal(total.input_tokens.known_total, 45);
  assert.deepEqual(
    total.calls_per_model.map((entry) => [entry.model.content, entry.calls]),
    [["model-a", 1], ["model-b", 3]],
  );
});

test("page-session stats reset when the authenticated caller changes", () => {
  const accumulator = new InspectorSessionStatsAccumulator();
  accumulator.record("operator-a", "thread-a/run-1", stats({
    model: "private-model",
    calls: 1,
    inputTokens: 10,
  }));

  assert.equal(accumulator.snapshot("operator-b"), null);
  const total = accumulator.record("operator-b", "thread-b/run-1", stats({
    model: "other-model",
    calls: 2,
    inputTokens: 20,
  }));
  assert.equal(total.total_model_calls, 2);
  assert.deepEqual(
    total.calls_per_model.map((entry) => entry.model.content),
    ["other-model"],
  );
});

test("only a complete statistics record decodes into session accumulation", () => {
  const complete = stats({ model: "model-a", calls: 1, inputTokens: 10, tools: 1 });
  assert.equal(decodeSessionDiagnosticStats(complete), complete);

  // An absent or partial record must stay unavailable: accumulating it would
  // present fabricated zeros as a real "0 model calls" reading.
  assert.equal(decodeSessionDiagnosticStats(undefined), null);
  assert.equal(decodeSessionDiagnosticStats({}), null);
  assert.equal(decodeSessionDiagnosticStats([complete]), null);
  assert.equal(
    decodeSessionDiagnosticStats({ ...complete, total_model_calls: -1 }),
    null,
  );
  assert.equal(
    decodeSessionDiagnosticStats({ ...complete, input_tokens: { known_total: 1 } }),
    null,
  );
  assert.equal(
    decodeSessionDiagnosticStats({ ...complete, calls_per_model: null }),
    null,
  );

  // Nested breakdown entries, not just the array. A negative `calls` would
  // otherwise decode and then coerce to 0 during accumulation, silently
  // reporting a model as having made zero calls.
  const model = { content: "model-a", original_bytes: 7, truncated: false };
  for (const entry of [
    null,
    { calls: 4 },
    { model, calls: -1 },
    { model, calls: 1.5 },
    { model: "model-a", calls: 1 },
    { model: { content: "model-a" }, calls: 1 },
  ]) {
    assert.equal(
      decodeSessionDiagnosticStats({ ...complete, calls_per_model: [entry] }),
      null,
      `entry should be rejected: ${JSON.stringify(entry)}`,
    );
  }
  assert.ok(
    decodeSessionDiagnosticStats({ ...complete, calls_per_model: [{ model, calls: 0 }] }),
  );

  // The host truncates the breakdown at 64 and flags it, so a longer array is
  // rejected before it is scanned or retained. 64 exactly still decodes.
  const entry = { model, calls: 1 };
  assert.ok(
    decodeSessionDiagnosticStats({
      ...complete,
      calls_per_model: Array.from({ length: 64 }, () => entry),
    }),
  );
  assert.equal(
    decodeSessionDiagnosticStats({
      ...complete,
      calls_per_model: Array.from({ length: 65 }, () => entry),
    }),
    null,
  );
  assert.equal(
    decodeSessionDiagnosticStats({ ...complete, total_tool_calls: "3" }),
    null,
  );

  // Tool aggregates are optional: an older host may omit them entirely.
  const withoutToolTotals = { ...complete };
  delete withoutToolTotals.total_tool_calls;
  delete withoutToolTotals.successful_tool_calls;
  delete withoutToolTotals.failed_tool_calls;
  assert.equal(decodeSessionDiagnosticStats(withoutToolTotals), withoutToolTotals);
});

test("page-session stats tolerate malformed snapshot fields", () => {
  const accumulator = new InspectorSessionStatsAccumulator();
  const malformed = {
    total_model_calls: -1,
    calls_per_model: [{ calls: 4 }],
  } as unknown as SessionDiagnosticStats;

  const total = accumulator.record("operator-a", "thread-a/run-1", malformed);

  assert.equal(total.total_model_calls, 0);
  assert.deepEqual(total.calls_per_model, []);
  assert.equal(total.calls_per_model_truncated, true);
  assert.deepEqual(total.input_tokens, {
    known_total: 0,
    unavailable_samples: 0,
  });
});
