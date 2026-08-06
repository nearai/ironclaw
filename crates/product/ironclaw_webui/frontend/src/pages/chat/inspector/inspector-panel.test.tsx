// @vitest-environment jsdom

import assert from "node:assert/strict";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, test, vi } from "vitest";

import { INSPECTOR_HEALTH } from "./inspector-state";
import { InspectorPanel } from "./inspector-panel";

const inspectorCalls = vi.hoisted(() => [] as any[]);
const inspectorState = vi.hoisted(() => ({
  snapshot: null as any,
  updates: [] as any[],
  health: "connected",
  error: null as string | null,
  lastCursor: null as string | null,
}));

vi.mock("./useInspector", () => ({
  useInspector: (input: unknown) => {
    inspectorCalls.push(input);
    return inspectorState;
  },
}));

let root: ReturnType<typeof createRoot> | null = null;

function setViewport(width: number) {
  Object.defineProperty(window, "innerWidth", {
    configurable: true,
    value: width,
  });
  window.dispatchEvent(new Event("resize"));
}

beforeEach(() => {
  inspectorCalls.length = 0;
  inspectorState.snapshot = null;
  inspectorState.updates = [];
  inspectorState.health = INSPECTOR_HEALTH.CONNECTED;
  inspectorState.error = null;
  inspectorState.lastCursor = null;
  sessionStorage.clear();
  setViewport(1440);
  root = createRoot(document.body.appendChild(document.createElement("div")));
});

test("prompt tab renders metadata, bounded components, and reconstruction notice", async () => {
  const text = (content: string, truncated = false) => ({
    content,
    original_bytes: content.length + (truncated ? 10 : 0),
    truncated,
  });
  inspectorState.snapshot = {
    prompt: {
      components: [
        {
          kind: "identity",
          label: text("Identity 1"),
          content: text("You are a careful assistant.", true),
          estimated_tokens: 8,
        },
      ],
      components_truncated: false,
      reconstructed_prompt: text("Identity 1:\nYou are a careful assistant."),
      total_estimated_tokens: 32,
      message_count: 4,
      identity_message_count: 1,
      instruction_snippet_count: 2,
      active_skills: [text("workspace-search")],
      active_skills_truncated: false,
      capability_count: 3,
      requested_model: text("interactive_model"),
      effective_model: text("provider-model"),
      context_limit: 128_000,
    },
  };

  await act(async () =>
    root?.render(<InspectorPanel threadId="thread-a" runId="run-a" />),
  );
  const prompt = document.querySelector("[data-testid='inspector-prompt-content']");
  assert.ok(prompt);
  assert.match(prompt.textContent || "", /provider-model/);
  assert.match(prompt.textContent || "", /workspace-search/);
  assert.match(prompt.textContent || "", /Some prompt content was safely truncated/);
  assert.match(prompt.textContent || "", /may differ from a specific historical model call/);
  assert.equal(document.querySelectorAll("details").length, 2);
});

test("stats tab formats aggregates and unavailable samples without zero fabrication", async () => {
  inspectorState.snapshot = {
    stats: {
      total_model_calls: 3,
      calls_per_model: [
        {
          model: { content: "provider-model", original_bytes: 14, truncated: false },
          calls: 3,
        },
      ],
      calls_per_model_truncated: false,
      input_tokens: { known_total: 1_200, unavailable_samples: 1 },
      output_tokens: { known_total: 80, unavailable_samples: 1 },
      cache_read_input_tokens: { known_total: 0, unavailable_samples: 3 },
      cache_creation_input_tokens: { known_total: 20, unavailable_samples: 1 },
      total_latency_ms: { known_total: 900, unavailable_samples: 1 },
    },
  };

  await act(async () =>
    root?.render(<InspectorPanel threadId="thread-a" runId="run-a" />),
  );
  await act(async () =>
    document.querySelector<HTMLButtonElement>("[data-testid='inspector-tab-stats']")?.click(),
  );

  const stats = document.querySelector("[data-testid='inspector-stats-content']");
  assert.ok(stats);
  assert.match(stats.textContent || "", /1,200/);
  assert.match(stats.textContent || "", /450 ms/);
  assert.match(stats.textContent || "", /Unavailable/);
  assert.match(stats.textContent || "", /provider-model3/);
  assert.doesNotMatch(stats.textContent || "", /Tool calls|Tool outcomes/);
  assert.match(stats.textContent || "", /metric samples were unavailable/);
});

test("activity tab renders ordered correlations and navigates retained turns", async () => {
  inspectorState.snapshot = {
    stream_id: "stream-a",
    activity: [
      {
        sequence: 1,
        event: {
          occurred_at: "2026-08-06T10:00:00Z",
          kind: "model_call_started",
          iteration: 2,
          activity_id: null,
          model_call_id: "call-1234567890",
          summary: { content: "Model call started", original_bytes: 18, truncated: false },
        },
      },
    ],
  };

  await act(async () => root?.render(<InspectorPanel threadId="thread-a" runId="run-a" />));
  await act(async () => root?.render(<InspectorPanel threadId="thread-a" runId="run-b" />));
  await act(async () =>
    document.querySelector<HTMLButtonElement>("[data-testid='inspector-tab-activity']")?.click(),
  );

  const activity = document.querySelector("[data-testid='inspector-activity-content']");
  assert.ok(activity);
  assert.match(activity.textContent || "", /Model call started/);
  assert.match(activity.textContent || "", /Pending/);
  assert.match(activity.textContent || "", /Turn 2 of 2/);

  await act(async () =>
    document.querySelector<HTMLButtonElement>("[aria-label='Previous turn']")?.click(),
  );
  assert.equal(inspectorCalls.at(-1)?.runId, "run-a");
});

afterEach(async () => {
  await act(async () => root?.unmount());
  document.body.replaceChildren();
});

test("panel switches tabs, closes, reopens, overlays tablets, and hides on mobile", async () => {
  await act(async () =>
    root?.render(<InspectorPanel threadId="thread-a" runId="run-a" />),
  );
  const panel = document.querySelector<HTMLElement>("[data-testid='inspector-panel']");
  assert.equal(panel?.dataset.layout, "sidebar");
  assert.equal(document.querySelector("[data-testid='inspector-health']")?.textContent, "Live");
  assert.equal(inspectorCalls.at(-1)?.enabled, true);

  await act(async () =>
    document.querySelector<HTMLButtonElement>("[data-testid='inspector-tab-stats']")?.click(),
  );
  assert.equal(
    document.querySelector("[data-testid='inspector-tab-stats']")?.getAttribute("aria-selected"),
    "true",
  );

  await act(async () =>
    document.querySelector<HTMLButtonElement>("[data-testid='inspector-close']")?.click(),
  );
  assert.equal(document.querySelector("[data-testid='inspector-panel']"), null);
  assert.ok(document.querySelector("[data-testid='inspector-open']"));
  assert.equal(inspectorCalls.at(-1)?.enabled, false);

  await act(async () =>
    document.querySelector<HTMLButtonElement>("[data-testid='inspector-open']")?.click(),
  );
  assert.equal(
    document.querySelector("[data-testid='inspector-tab-stats']")?.getAttribute("aria-selected"),
    "true",
  );
  assert.equal(inspectorCalls.at(-1)?.enabled, true);

  await act(async () => setViewport(900));
  assert.equal(
    document.querySelector<HTMLElement>("[data-testid='inspector-panel']")?.dataset.layout,
    "overlay",
  );

  await act(async () => setViewport(500));
  assert.equal(document.querySelector("[data-testid='inspector-panel']"), null);
  assert.equal(inspectorCalls.at(-1)?.enabled, false);
});
