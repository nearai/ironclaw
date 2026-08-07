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

function boundedText(content: string, truncated = false) {
  return {
    content,
    original_bytes: content.length + (truncated ? 10 : 0),
    truncated,
  };
}

function promptDiagnostic() {
  return {
    components: [
      {
        kind: "identity",
        label: boundedText("Identity 1"),
        content: boundedText("You are a careful assistant."),
        estimated_tokens: 8,
      },
    ],
    components_truncated: false,
    reconstructed_prompt: boundedText("Identity 1:\nYou are a careful assistant."),
    total_estimated_tokens: 32,
    message_count: 4,
    identity_message_count: 1,
    instruction_snippet_count: 2,
    active_skills: [boundedText("workspace-search")],
    active_skills_truncated: false,
    capability_count: 3,
    requested_model: boundedText("interactive_model"),
    effective_model: boundedText("provider-model"),
    context_limit: 128_000,
  };
}

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
  const prompt = promptDiagnostic();
  prompt.components[0].content = boundedText("You are a careful assistant.", true);
  inspectorState.snapshot = {
    prompt,
  };

  await act(async () =>
    root?.render(<InspectorPanel threadId="thread-a" runId="run-a" />),
  );
  const promptContent = document.querySelector("[data-testid='inspector-prompt-content']");
  assert.ok(promptContent);
  assert.match(promptContent.textContent || "", /provider-model/);
  assert.match(promptContent.textContent || "", /workspace-search/);
  assert.match(promptContent.textContent || "", /Some prompt content was safely truncated/);
  assert.match(promptContent.textContent || "", /may differ from a specific historical model call/);
  assert.equal(document.querySelectorAll("details").length, 2);
});

const truncationCases: Array<[string, (prompt: ReturnType<typeof promptDiagnostic>) => void]> = [
  ["component label", (prompt) => { prompt.components[0].label.truncated = true; }],
  ["requested model", (prompt) => { prompt.requested_model.truncated = true; }],
  ["effective model", (prompt) => { prompt.effective_model.truncated = true; }],
  ["active skill", (prompt) => { prompt.active_skills[0].truncated = true; }],
];

test.each(truncationCases)("prompt tab reports a truncated %s", async (_label, truncate) => {
  const prompt = promptDiagnostic();
  truncate(prompt);
  inspectorState.snapshot = { prompt };

  await act(async () =>
    root?.render(<InspectorPanel threadId="thread-a" runId="run-a" />),
  );

  assert.match(
    document.querySelector("[role='status']")?.textContent || "",
    /Some prompt content was safely truncated/,
  );
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
