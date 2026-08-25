// @vitest-environment jsdom

import assert from "node:assert/strict";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, test, vi } from "vitest";

import "../../../i18n/en";
import "../../../i18n/zh-CN";
import { I18nProvider } from "../../../lib/i18n";
import {
  MAX_INSPECTOR_RUNS_PER_THREAD,
  inspectorRunHistoryKey,
} from "./inspector-activity";
import { INSPECTOR_HEALTH } from "./inspector-state";
import { InspectorPanel } from "./inspector-panel";
import { resetInspectorSessionStats } from "./inspector-session-stats";

const inspectorCalls = vi.hoisted(() => [] as any[]);
const fetchInspectorTool = vi.hoisted(() => vi.fn());
const inspectorState = vi.hoisted(() => ({
  snapshot: null as any,
  updates: [] as any[],
  health: "connected",
  error: null as string | null,
  lastCursor: null as string | null,
  reconnectCount: 0,
  receivedUpdateCount: 0,
  lastUpdateAt: null as string | null,
}));

vi.mock("./useInspector", () => ({
  useInspector: (input: unknown) => {
    inspectorCalls.push(input);
    return {
      ...inspectorState,
      sessionStats: inspectorState.snapshot?.stats ?? null,
    };
  },
}));

vi.mock("./inspector-api", () => ({ fetchInspectorTool }));

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
  resetInspectorSessionStats();
  fetchInspectorTool.mockReset();
  inspectorCalls.length = 0;
  inspectorState.snapshot = null;
  inspectorState.updates = [];
  inspectorState.health = INSPECTOR_HEALTH.CONNECTED;
  inspectorState.error = null;
  inspectorState.lastCursor = null;
  inspectorState.reconnectCount = 0;
  inspectorState.receivedUpdateCount = 0;
  inspectorState.lastUpdateAt = null;
  localStorage.clear();
  sessionStorage.clear();
  setViewport(1440);
  const headerAction = document.body.appendChild(document.createElement("span"));
  headerAction.id = "page-header-inspector-action";
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

test("stats tab formats aggregates and unavailable samples without zero fabrication", async () => {
  inspectorState.snapshot = {
    stats: {
      total_model_calls: 3,
      total_tool_calls: 5,
      successful_tool_calls: 4,
      failed_tool_calls: 1,
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
  assert.match(stats.textContent || "", /Tool calls5/);
  assert.match(stats.textContent || "", /Successful tool calls4/);
  assert.match(stats.textContent || "", /Failed tool calls1/);
  assert.match(stats.textContent || "", /7 metric samples were unavailable/);
});

test("stats expose unavailable tool totals and browser-observed stream health", async () => {
  inspectorState.snapshot = {
    stats: {
      total_model_calls: 0,
      calls_per_model: [],
      calls_per_model_truncated: false,
      input_tokens: { known_total: 0, unavailable_samples: 0 },
      output_tokens: { known_total: 0, unavailable_samples: 0 },
      cache_read_input_tokens: { known_total: 0, unavailable_samples: 0 },
      cache_creation_input_tokens: { known_total: 0, unavailable_samples: 0 },
      total_latency_ms: { known_total: 0, unavailable_samples: 0 },
    },
  };
  inspectorState.health = INSPECTOR_HEALTH.RECONNECTING;
  inspectorState.reconnectCount = 2;
  inspectorState.receivedUpdateCount = 7;
  inspectorState.lastUpdateAt = "2026-08-06T10:30:00.000Z";

  await act(async () => root?.render(<InspectorPanel threadId="thread-a" runId="run-a" />));
  await act(async () =>
    document.querySelector<HTMLButtonElement>("[data-testid='inspector-tab-stats']")?.click(),
  );

  const stats = document.querySelector("[data-testid='inspector-stats-content']");
  assert.ok(stats);
  assert.match(stats.textContent || "", /Tool callsUnavailable/);
  assert.match(stats.textContent || "", /Reconnects2/);
  assert.match(stats.textContent || "", /Diagnostic updates7/);
  assert.match(stats.textContent || "", /Reconnecting/);
  assert.doesNotMatch(stats.textContent || "", /Tool calls0/);
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
  assert.ok(document.querySelector<HTMLButtonElement>("[aria-label='Latest turn']"));
  await act(async () =>
    document.querySelector<HTMLButtonElement>("[aria-label='Latest turn']")?.click(),
  );
  assert.equal(inspectorCalls.at(-1)?.runId, "run-b");
});

test("inspector chrome follows the active locale", async () => {
  localStorage.setItem("ironclaw_language", "zh-CN");
  await act(async () => root?.render(
    <I18nProvider>
      <InspectorPanel threadId="thread-a" runId="run-a" />
    </I18nProvider>,
  ));

  assert.equal(document.querySelector("[data-testid='inspector-health']")?.textContent, "实时");
  assert.equal(
    document.querySelector("[data-testid='inspector-close']")?.getAttribute("aria-label"),
    "关闭检查器",
  );
  assert.match(document.querySelector("[data-testid='inspector-panel']")?.textContent || "", /提示词/);
});

test("product activity summaries resolve their translation key in the active locale", async () => {
  localStorage.setItem("ironclaw_language", "zh-CN");
  inspectorState.updates = [{
    local_id: "product-localized",
    update: {
      type: "activity",
      data: {
        occurred_at: "2026-08-06T10:00:00Z",
        kind: "tool_started",
        iteration: null,
        activity_id: "activity-localized",
        model_call_id: null,
        summary: {
          content: "inspector.activity.summary.toolStarted",
          original_bytes: 38,
          truncated: false,
        },
      },
    },
  }];

  await act(async () => root?.render(
    <I18nProvider>
      <InspectorPanel threadId="thread-a" runId="run-localized" />
    </I18nProvider>,
  ));
  await act(async () =>
    document.querySelector<HTMLButtonElement>("[data-testid='inspector-tab-activity']")?.click(),
  );

  const activity = document.querySelector("[data-testid='inspector-activity-content']");
  assert.match(activity?.textContent || "", /工具调用已开始/);
  assert.doesNotMatch(activity?.textContent || "", /inspector\.activity\.summary|Tool invocation/);
});

test("authoritative activity replacement keeps known summaries localized", async () => {
  localStorage.setItem("ironclaw_language", "zh-CN");
  inspectorState.snapshot = { stream_id: "stream-authoritative", activity: [] };
  inspectorState.updates = [
    {
      local_id: "product-turn",
      update: {
        type: "activity",
        data: {
          occurred_at: "2026-08-06T09:59:58Z",
          kind: "turn_started",
          iteration: null,
          activity_id: null,
          model_call_id: null,
          summary: {
            content: "inspector.activity.summary.turnRunning",
            original_bytes: 38,
            truncated: false,
          },
        },
      },
    },
    {
      local_id: "product-completed",
      update: {
        type: "activity",
        data: {
          occurred_at: "2026-08-06T10:00:00Z",
          kind: "tool_completed",
          iteration: null,
          activity_id: "activity-completed",
          model_call_id: null,
          summary: {
            content: "inspector.activity.summary.toolCompleted",
            original_bytes: 40,
            truncated: false,
          },
        },
      },
    },
    {
      stream_id: "stream-authoritative",
      sequence: 1,
      update: {
        type: "activity",
        data: {
          occurred_at: "2026-08-06T09:59:59Z",
          kind: "turn_started",
          iteration: null,
          activity_id: null,
          model_call_id: null,
          summary: {
            content: "Turn started",
            original_bytes: 12,
            truncated: false,
          },
        },
      },
    },
    {
      stream_id: "stream-authoritative",
      sequence: 2,
      update: {
        type: "activity",
        data: {
          occurred_at: "2026-08-06T10:00:01Z",
          kind: "tool_completed",
          iteration: null,
          activity_id: "activity-completed",
          model_call_id: null,
          summary: {
            content: "Tool invocation completed",
            original_bytes: 25,
            truncated: false,
          },
        },
      },
    },
    {
      stream_id: "stream-authoritative",
      sequence: 3,
      update: {
        type: "activity",
        data: {
          occurred_at: "2026-08-06T10:00:02Z",
          kind: "tool_failed",
          iteration: null,
          activity_id: "activity-failed",
          model_call_id: null,
          summary: {
            content: "Provider returned code 42",
            original_bytes: 25,
            truncated: false,
          },
        },
      },
    },
  ];

  await act(async () => root?.render(
    <I18nProvider>
      <InspectorPanel threadId="thread-a" runId="run-authoritative" />
    </I18nProvider>,
  ));
  await act(async () =>
    document.querySelector<HTMLButtonElement>("[data-testid='inspector-tab-activity']")?.click(),
  );

  const activity = document.querySelector("[data-testid='inspector-activity-content']");
  assert.match(activity?.textContent || "", /轮次已开始/);
  assert.match(activity?.textContent || "", /工具调用已完成/);
  assert.doesNotMatch(activity?.textContent || "", /Turn started|Tool invocation completed/);
  assert.match(activity?.textContent || "", /Provider returned code 42/);
});

test("tool activity loads bounded verbose details from the dedicated endpoint", async () => {
  inspectorState.snapshot = {
    stream_id: "stream-tool",
    activity: [{
      sequence: 1,
      event: {
        occurred_at: "2026-08-06T10:00:00Z",
        kind: "tool_completed",
        iteration: null,
        activity_id: "01890a5d-ac96-774b-bcce-b302099a8057",
        model_call_id: null,
        summary: { content: "Tool invocation completed", original_bytes: 25, truncated: false },
      },
    }],
  };
  fetchInspectorTool.mockResolvedValue({
    tool: {
      capability_name: { content: "filesystem.read", original_bytes: 15, truncated: false },
      arguments: { content: '{"path":"safe.txt"}', original_bytes: 19, truncated: false },
      result: { content: "bounded output", original_bytes: 75_000, truncated: true },
      status: "succeeded",
      duration_ms: 42,
      output_bytes: 75_000,
      failure_category: null,
      failure_summary: null,
    },
  });

  await act(async () => root?.render(<InspectorPanel threadId="thread-a" runId="run-tool" />));
  await act(async () =>
    document.querySelector<HTMLButtonElement>("[data-testid='inspector-tab-activity']")?.click(),
  );
  await act(async () => {
    document.querySelector<HTMLButtonElement>("[aria-expanded='false']")?.click();
    await Promise.resolve();
  });

  assert.deepEqual(fetchInspectorTool.mock.calls[0]?.[0]?.threadId, "thread-a");
  assert.deepEqual(fetchInspectorTool.mock.calls[0]?.[0]?.runId, "run-tool");
  const detail = document.querySelector("[data-testid^='inspector-tool-detail-']");
  assert.ok(detail);
  assert.match(detail.textContent || "", /filesystem\.read/);
  assert.match(detail.textContent || "", /safe\.txt/);
  assert.match(detail.textContent || "", /Duration: 42 ms/);
  assert.match(detail.textContent || "", /truncated from 75,000 bytes/);
  assert.match(detail.textContent || "", /bounded output/);
});

test("tool activity rejects detail missing a capability name without disabling inspector", async () => {
  inspectorState.snapshot = {
    stream_id: "stream-tool",
    activity: [{
      sequence: 1,
      event: {
        occurred_at: "2026-08-06T10:00:00Z",
        kind: "tool_completed",
        iteration: null,
        activity_id: "01890a5d-ac96-774b-bcce-b302099a8057",
        model_call_id: null,
        summary: { content: "Tool invocation completed", original_bytes: 25, truncated: false },
      },
    }],
  };
  fetchInspectorTool.mockResolvedValue({
    tool: {
      arguments: null,
      result: null,
      status: "succeeded",
      duration_ms: null,
      output_bytes: null,
      failure_category: null,
      failure_summary: null,
    },
  });

  await act(async () => root?.render(<InspectorPanel threadId="thread-a" runId="run-tool" />));
  await act(async () =>
    document.querySelector<HTMLButtonElement>("[data-testid='inspector-tab-activity']")?.click(),
  );
  await act(async () => {
    document.querySelector<HTMLButtonElement>("[aria-expanded='false']")?.click();
    await Promise.resolve();
  });

  assert.ok(document.querySelector("[data-testid='inspector-panel']"));
  assert.match(document.body.textContent || "", /Tool details are unavailable/);
});

test("tool detail request is cancelled when navigating to another run", async () => {
  inspectorState.snapshot = {
    stream_id: "stream-tool",
    activity: [{
      sequence: 1,
      event: {
        occurred_at: "2026-08-06T10:00:00Z",
        kind: "tool_completed",
        iteration: null,
        activity_id: "01890a5d-ac96-774b-bcce-b302099a8057",
        model_call_id: null,
        summary: { content: "Tool invocation completed", original_bytes: 25, truncated: false },
      },
    }],
  };
  fetchInspectorTool.mockImplementation(() => new Promise(() => {}));

  await act(async () => root?.render(<InspectorPanel threadId="thread-a" runId="run-a" />));
  await act(async () =>
    document.querySelector<HTMLButtonElement>("[data-testid='inspector-tab-activity']")?.click(),
  );
  await act(async () =>
    document.querySelector<HTMLButtonElement>("[aria-expanded='false']")?.click(),
  );
  const signal = fetchInspectorTool.mock.calls[0]?.[0]?.signal as AbortSignal | undefined;
  assert.equal(signal?.aborted, false);

  await act(async () => root?.render(<InspectorPanel threadId="thread-a" runId="run-b" />));

  assert.equal(signal?.aborted, true);
});

test("tool details can retry after a transient request failure", async () => {
  inspectorState.snapshot = {
    stream_id: "stream-tool",
    activity: [{
      sequence: 1,
      event: {
        occurred_at: "2026-08-06T10:00:00Z",
        kind: "tool_completed",
        iteration: null,
        activity_id: "01890a5d-ac96-774b-bcce-b302099a8057",
        model_call_id: null,
        summary: { content: "Tool invocation completed", original_bytes: 25, truncated: false },
      },
    }],
  };
  fetchInspectorTool
    .mockRejectedValueOnce(new Error("temporary failure"))
    .mockResolvedValueOnce({
      tool: {
        capability_name: { content: "builtin.echo", original_bytes: 12, truncated: false },
        arguments: null,
        result: { content: "retried output", original_bytes: 14, truncated: false },
        status: "succeeded",
        duration_ms: null,
        output_bytes: 14,
        failure_category: null,
        failure_summary: null,
      },
    });

  await act(async () => root?.render(<InspectorPanel threadId="thread-a" runId="run-tool" />));
  await act(async () =>
    document.querySelector<HTMLButtonElement>("[data-testid='inspector-tab-activity']")?.click(),
  );
  await act(async () => {
    document.querySelector<HTMLButtonElement>("[aria-expanded='false']")?.click();
    await Promise.resolve();
  });
  assert.match(document.body.textContent || "", /Tool details are unavailable/);

  await act(async () =>
    document.querySelector<HTMLButtonElement>("[aria-expanded='true']")?.click(),
  );
  await act(async () => {
    document.querySelector<HTMLButtonElement>("[aria-expanded='false']")?.click();
    await Promise.resolve();
  });

  assert.equal(fetchInspectorTool.mock.calls.length, 2);
  assert.match(document.body.textContent || "", /retried output/);
});

test("activity navigation advances when a pinned run leaves the history window", async () => {
  await act(async () => root?.render(<InspectorPanel threadId="thread-a" runId="run-0" />));
  await act(async () => root?.render(<InspectorPanel threadId="thread-a" runId="run-1" />));
  await act(async () =>
    document.querySelector<HTMLButtonElement>("[data-testid='inspector-tab-activity']")?.click(),
  );
  await act(async () =>
    document.querySelector<HTMLButtonElement>("[aria-label='Previous turn']")?.click(),
  );
  assert.equal(inspectorCalls.at(-1)?.runId, "run-0");

  const window = MAX_INSPECTOR_RUNS_PER_THREAD;
  sessionStorage.setItem(
    inspectorRunHistoryKey(),
    JSON.stringify({
      "thread-a": Array.from({ length: window }, (_, index) => `run-${index + 1}`),
    }),
  );
  await act(async () =>
    root?.render(<InspectorPanel threadId="thread-a" runId={`run-${window}`} />),
  );

  // The pinned run-0 was evicted from the bounded window. Rejoin the latest
  // observed turn — the default selection — rather than the oldest retained
  // one, so eviction never silently parks navigation on stale history.
  assert.equal(inspectorCalls.at(-1)?.runId, `run-${window}`);
  assert.match(document.body.textContent || "", new RegExp(`Turn ${window} of ${window}`));
  assert.equal(
    document.querySelector<HTMLButtonElement>("[aria-label='Previous turn']")?.disabled,
    false,
  );
  assert.equal(
    document.querySelector<HTMLButtonElement>("[aria-label='Next turn']")?.disabled,
    true,
  );
});

afterEach(async () => {
  await act(async () => root?.unmount());
  document.body.replaceChildren();
});

test("header icon toggles the panel while collection continues when closed or on mobile", async () => {
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
  const headerToggle = document.querySelector<HTMLButtonElement>("[data-testid='inspector-open']");
  assert.ok(headerToggle);
  assert.equal(headerToggle.closest("#page-header-inspector-action")?.id, "page-header-inspector-action");
  assert.equal(headerToggle.getAttribute("aria-pressed"), "false");
  assert.equal(inspectorCalls.at(-1)?.enabled, true);

  await act(async () =>
    headerToggle.click(),
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
  assert.equal(inspectorCalls.at(-1)?.enabled, true);
});
