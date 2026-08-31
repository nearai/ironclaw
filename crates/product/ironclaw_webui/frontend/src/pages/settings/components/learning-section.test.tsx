// @ts-nocheck

// @vitest-environment happy-dom

import assert from "node:assert/strict";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider, notifyManager } from "@tanstack/react-query";
import { test, vi } from "vitest";
import "../../../i18n/en";
import { ApiError } from "../../../lib/api";
import { I18nProvider } from "../../../lib/i18n";

notifyManager.setScheduler((callback) => callback());
globalThis.IS_REACT_ACT_ENVIRONMENT = true;

const requests = vi.hoisted(() => ({
  setLearning: vi.fn(),
  listLlmProviderModels: vi.fn(),
}));

vi.mock("../lib/settings-api", () => ({
  setLearning: requests.setLearning,
}));

import { LearningSection } from "./learning-section";

function providerState(overrides = {}) {
  return {
    activeProviderId: "openai_compatible",
    selectedModel: "mock-model",
    providers: [
      {
        id: "openai_compatible",
        adapter: "open_ai_completions",
        base_url: "http://127.0.0.1:1234/v1",
        default_model: "mock-model",
        can_list_models: true,
      },
    ],
    userModelPolicy: null,
    listModels: requests.listLlmProviderModels,
    hasActiveProvider: true,
    learning: {
      enabled: false,
      model: null,
      memory_write_policy: "staged",
      status: "disabled",
      reason: null,
    },
    ...overrides,
  };
}

async function renderSection(state) {
  const container = document.createElement("div");
  document.body.append(container);
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const root = createRoot(container);
  await act(async () => {
    root.render(
      <QueryClientProvider client={queryClient}>
        <I18nProvider>
          <LearningSection providerState={state} />
        </I18nProvider>
      </QueryClientProvider>
    );
  });
  // Let react-query and component effects settle before interaction.
  await act(async () => {});
  return { container, queryClient, root };
}

async function rerenderSection(rendered, state) {
  await act(async () => {
    rendered.root.render(
      <QueryClientProvider client={rendered.queryClient}>
        <I18nProvider>
          <LearningSection providerState={state} />
        </I18nProvider>
      </QueryClientProvider>
    );
  });
  await act(async () => {});
}

async function clickSwitch(rendered) {
  const toggle = rendered.container.querySelector<HTMLButtonElement>(
    '[data-testid="settings-learning-switch"]'
  );
  assert.ok(toggle);
  await act(async () => toggle.click());
}

function switchChecked(rendered) {
  const toggle = rendered.container.querySelector<HTMLButtonElement>(
    '[data-testid="settings-learning-switch"]'
  );
  return toggle?.getAttribute("aria-checked") === "true";
}

async function openModelMenu(rendered) {
  const trigger = rendered.container.querySelector<HTMLButtonElement>(
    '[data-testid="settings-learning-model"] [aria-haspopup="listbox"]'
  );
  assert.ok(trigger, "the learning model selector renders an accessible trigger");
  await act(async () => trigger.click());
}

async function fetchProviderModels(rendered) {
  const button = rendered.container.querySelector<HTMLButtonElement>(
    '[data-testid="settings-learning-fetch-models"]'
  );
  assert.ok(button, "the Learning model fetch action is available");
  await act(async () => button.click());
}

function menuOptionLabels(rendered) {
  return [...rendered.container.querySelectorAll('[role="option"]')].map(
    (option) => option.textContent ?? ""
  );
}

test("provider-advertised models appear when no tenant selection policy exists", async () => {
  requests.listLlmProviderModels.mockResolvedValue({
    ok: true,
    models: ["mock-model", "claude-opus-4"],
  });
  const rendered = await renderSection(providerState());
  try {
    assert.equal(switchChecked(rendered), false, "learning must not claim to run when disabled");
    assert.equal(
      rendered.container.querySelector('[role="alert"]')?.textContent ?? "",
      "",
      "no error should be announced on first render"
    );
    const text = rendered.container.textContent ?? "";
    assert.match(text, /Review successful runs/);
    assert.match(text, /background review/, "supporting copy explains what enabling does");
    assert.equal(
      requests.listLlmProviderModels.mock.calls.length,
      0,
      "rendering Settings must not start provider network discovery"
    );
    await fetchProviderModels(rendered);
    await openModelMenu(rendered);
    const options = menuOptionLabels(rendered);
    for (const model of ["mock-model", "claude-opus-4"]) {
      assert.ok(options.includes(model), `${model} should be offered by the active provider`);
    }
    assert.deepEqual(requests.listLlmProviderModels.mock.calls[0]?.[0], {
      provider_id: "openai_compatible",
      adapter: "open_ai_completions",
      base_url: "http://127.0.0.1:1234/v1",
      model: "mock-model",
    });
  } finally {
    act(() => rendered.root.unmount());
    rendered.container.remove();
    requests.listLlmProviderModels.mockReset();
  }
});
test("fallback models remain available when provider listing fails", async () => {
  requests.listLlmProviderModels.mockRejectedValue(new Error("provider unavailable"));
  const rendered = await renderSection(
    providerState({
      selectedModel: "active-model",
      userModelPolicy: { allowed_models: ["policy-model", "active-model"] },
      providers: [
        {
          id: "openai_compatible",
          adapter: "open_ai_completions",
          base_url: "http://127.0.0.1:1234/v1",
          default_model: "provider-default",
          can_list_models: true,
        },
      ],
      learning: {
        enabled: false,
        model: "stored-learning-model",
        status: "disabled",
        reason: null,
      },
    })
  );
  try {
    await fetchProviderModels(rendered);
    await openModelMenu(rendered);
    const options = menuOptionLabels(rendered);
    for (const model of [
      "active-model",

      "provider-default",
      "stored-learning-model",
      "policy-model",
    ]) {
      assert.ok(options.includes(model), `${model} should remain available as a fallback`);
    }
    assert.equal(
      options.filter((model) => model === "active-model").length,
      1,
      "fallback options should be deduplicated"
    );
  } finally {
    act(() => rendered.root.unmount());
    rendered.container.remove();
    requests.listLlmProviderModels.mockReset();
  }
});
test("provider changes discard pending model-list results", async () => {
  let resolveFirstRequest;
  requests.listLlmProviderModels.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        resolveFirstRequest = resolve;
      })
  );
  const rendered = await renderSection(providerState());
  try {
    await fetchProviderModels(rendered);
    await rerenderSection(
      rendered,
      providerState({
        activeProviderId: "provider-b",
        selectedModel: "provider-b-default",
        providers: [
          {
            id: "provider-b",
            adapter: "open_ai_completions",
            base_url: "http://127.0.0.1:2345/v1",
            default_model: "provider-b-default",
            can_list_models: true,
          },
        ],
      })
    );
    await act(async () => {
      resolveFirstRequest({ ok: true, models: ["stale-provider-a-model"] });
    });
    await openModelMenu(rendered);
    const options = menuOptionLabels(rendered);
    assert.ok(options.includes("provider-b-default"));
    assert.ok(!options.includes("stale-provider-a-model"));
  } finally {
    act(() => rendered.root.unmount());
    rendered.container.remove();
    requests.listLlmProviderModels.mockReset();
  }
});

test("enabling requires choosing a learning model first and sends nothing", async () => {
  const rendered = await renderSection(
    providerState({
      learning: { enabled: false, model: null, status: "disabled", reason: null },
    })
  );
  try {
    await clickSwitch(rendered);
    assert.equal(requests.setLearning.mock.calls.length, 0, "no request without a model");
    const alert = rendered.container.querySelector('[role="alert"]');
    assert.match(alert?.textContent ?? "", /Choose a learning model/);
    assert.equal(switchChecked(rendered), false, "the switch must not flip before saving");
  } finally {
    act(() => rendered.root.unmount());
    rendered.container.remove();
  }
});

test("enabling with a retained model sends one coherent PUT body", async () => {
  const authoritativeSnapshot = {
    providers: [],
    active: { provider_id: "openai_compatible", model: "mock-model" },
    user_model_policy: null,
    learning: { enabled: true, model: "mock-model", status: "ready", reason: null },
  };
  requests.setLearning.mockResolvedValue(authoritativeSnapshot);
  const rendered = await renderSection(
    providerState({
      learning: { enabled: false, model: "mock-model", status: "disabled", reason: null },
    })
  );
  try {
    await clickSwitch(rendered);
    assert.deepEqual(requests.setLearning.mock.calls[0]?.[0], {
      enabled: true,
      model: "mock-model",
      memory_write_policy: "staged",
    });
    assert.equal(requests.setLearning.mock.calls.length, 1, "exactly one request per gesture");
    assert.equal(switchChecked(rendered), true, "the adopted snapshot drives the switch");
    // The response is authoritative — every consumer of the providers cache
    // sees the applied snapshot.
    assert.equal(rendered.queryClient.getQueryData(["llm-providers"]), authoritativeSnapshot);
    assert.match(
      rendered.container.querySelector('[data-testid="settings-learning-status"]')
        ?.textContent ?? "",
      /On/
    );
  } finally {
    act(() => rendered.root.unmount());
    rendered.container.remove();
    requests.setLearning.mockReset();
  }
});

test("disabling preserves the chosen model for later re-enable", async () => {
  requests.setLearning.mockResolvedValue({
    providers: [],
    active: null,
    user_model_policy: null,
    learning: { enabled: false, model: "mock-model", status: "disabled", reason: null },
  });
  const rendered = await renderSection(
    providerState({
      learning: { enabled: true, model: "mock-model", status: "ready", reason: null },
    })
  );
  try {
    await clickSwitch(rendered);
    assert.deepEqual(requests.setLearning.mock.calls[0]?.[0], {
      enabled: false,
      model: "mock-model",
      memory_write_policy: "staged",
    });
    assert.equal(switchChecked(rendered), false);
    assert.equal(
      rendered.queryClient.getQueryData(["llm-providers"])?.learning?.model,
      "mock-model"
    );
  } finally {
    act(() => rendered.root.unmount());
    rendered.container.remove();
    requests.setLearning.mockReset();
  }
});

test("selecting a provider-listed model sends current enabled state and memory policy", async () => {
  requests.listLlmProviderModels.mockResolvedValue({
    ok: true,
    models: ["mock-model", "claude-opus-4"],
  });
  requests.setLearning.mockResolvedValue({
    providers: [],
    active: { provider_id: "openai_compatible", model: "claude-opus-4" },
    user_model_policy: null,
    learning: {
      enabled: true,
      model: "claude-opus-4",
      memory_write_policy: "automatic",
      status: "ready",
      reason: null,
    },
  });
  const rendered = await renderSection(
    providerState({
      learning: {
        enabled: true,
        model: "mock-model",
        memory_write_policy: "automatic",
        status: "ready",
        reason: null,
      },
    })
  );
  try {
    await fetchProviderModels(rendered);
    await openModelMenu(rendered);
    const option = rendered.container.querySelector<HTMLButtonElement>(
      '[role="option"][aria-selected="false"]'
    );
    assert.ok(option, "the unselected provider-listed option appears in the menu");
    await act(async () => option.click());
    assert.deepEqual(requests.setLearning.mock.calls[0]?.[0], {
      enabled: true,
      model: "claude-opus-4",
      memory_write_policy: "automatic",
    });
  } finally {
    act(() => rendered.root.unmount());
    rendered.container.remove();
    requests.setLearning.mockReset();
    requests.listLlmProviderModels.mockReset();
  }
});

test("changing the memory policy saves one coherent learning snapshot", async () => {
  const authoritativeSnapshot = {
    providers: [],
    active: { provider_id: "openai_compatible", model: "mock-model" },
    user_model_policy: null,
    learning: {
      enabled: false,
      model: "mock-model",
      memory_write_policy: "automatic",
      status: "disabled",
      reason: null,
    },
  };
  requests.setLearning.mockResolvedValue(authoritativeSnapshot);
  const rendered = await renderSection(
    providerState({
      learning: {
        enabled: false,
        model: "mock-model",
        memory_write_policy: "staged",
        status: "disabled",
        reason: null,
      },
    })
  );
  try {
    const trigger = rendered.container.querySelector<HTMLButtonElement>(
      '[data-testid="settings-learning-memory-policy"] [aria-haspopup="listbox"]'
    );
    assert.ok(trigger, "the memory policy selector renders an accessible trigger");
    await act(async () => trigger.click());
    const option = [...rendered.container.querySelectorAll<HTMLButtonElement>('[role="option"]')].find(
      (candidate) => candidate.textContent?.includes("Apply automatically")
    );
    assert.ok(option, "the automatic policy is available");
    await act(async () => option.click());
    assert.deepEqual(requests.setLearning.mock.calls[0]?.[0], {
      enabled: false,
      model: "mock-model",
      memory_write_policy: "automatic",
    });
    assert.equal(rendered.queryClient.getQueryData(["llm-providers"]), authoritativeSnapshot);
  } finally {
    act(() => rendered.root.unmount());
    rendered.container.remove();
    requests.setLearning.mockReset();
  }
});

test("invalid deployments surface the backend reason without claiming learning runs", async () => {
  const rendered = await renderSection(
    providerState({
      learning: {
        enabled: true,
        model: "mock-model",
        status: "invalid",
        reason: "model missing from provider catalog",
      },
    })
  );
  try {
    const text = rendered.container.textContent ?? "";
    assert.match(text, /model missing from provider catalog/, "the reason must be readable");
    assert.doesNotMatch(text, /On — /, "invalid state must not render the ready copy");
    assert.ok(switchChecked(rendered), "the saved setting is what the deployment stored");
  } finally {
    act(() => rendered.root.unmount());
    rendered.container.remove();
  }
});

test("save failures keep sanitized API errors and never flip the switch", async () => {
  requests.setLearning.mockRejectedValue(new ApiError("learning model rejected by provider"));
  const rendered = await renderSection(
    providerState({
      learning: { enabled: false, model: "mock-model", status: "disabled", reason: null },
    })
  );
  try {
    await clickSwitch(rendered);
    const alert = rendered.container.querySelector('[role="alert"]');
    assert.match(alert?.textContent ?? "", /learning model rejected by provider/);
    assert.equal(switchChecked(rendered), false);
  } finally {
    act(() => rendered.root.unmount());
    rendered.container.remove();
    requests.setLearning.mockReset();
  }
});

test("unexpected save failures fall back to generic copy, not raw errors", async () => {
  requests.setLearning.mockRejectedValue(new TypeError("network down"));
  const rendered = await renderSection(
    providerState({
      learning: { enabled: false, model: "mock-model", status: "disabled", reason: null },
    })
  );
  try {
    await clickSwitch(rendered);
    const alert = rendered.container.querySelector('[role="alert"]');
    assert.match(alert?.textContent ?? "", /Could not update learning/);
    assert.doesNotMatch(alert?.textContent ?? "", /network down/);
  } finally {
    act(() => rendered.root.unmount());
    rendered.container.remove();
    requests.setLearning.mockReset();
  }
});

test("controls lock down without an active provider", async () => {
  requests.listLlmProviderModels.mockReset();
  requests.setLearning.mockResolvedValue({});
  const rendered = await renderSection(providerState({ hasActiveProvider: false }));
  try {
    const toggle = rendered.container.querySelector<HTMLButtonElement>(
      '[data-testid="settings-learning-switch"]'
    );
    assert.ok(toggle?.disabled, "the switch must be disabled without an active provider");
    const trigger = rendered.container.querySelector<HTMLButtonElement>(
      '[data-testid="settings-learning-model"] [aria-haspopup="listbox"]'
    );
    assert.ok(trigger?.disabled, "the model selector must be disabled without an active provider");
    assert.equal(
      requests.listLlmProviderModels.mock.calls.length,
      0,
      "model listing must not be requested without an active provider"
    );
    await clickSwitch(rendered);
    assert.equal(requests.setLearning.mock.calls.length, 0);
  } finally {
    act(() => rendered.root.unmount());
    rendered.container.remove();
    requests.setLearning.mockReset();
    requests.listLlmProviderModels.mockReset();
  }
});
test("an enabled invalid learning setting can be disabled after its provider disappears", async () => {
  requests.setLearning.mockResolvedValue({
    providers: [],
    active: null,
    user_model_policy: null,
    learning: {
      enabled: false,
      model: "mock-model",
      memory_write_policy: "automatic",
      status: "disabled",
      reason: null,
    },
  });
  const rendered = await renderSection(
    providerState({
      hasActiveProvider: false,
      learning: {
        enabled: true,
        model: "mock-model",
        memory_write_policy: "automatic",
        status: "invalid",
        reason: "provider unavailable",
      },
    })
  );
  try {
    const toggle = rendered.container.querySelector<HTMLButtonElement>(
      '[data-testid="settings-learning-switch"]'
    );
    assert.ok(toggle);
    assert.equal(toggle.disabled, false, "disabling remains available without a provider");
    await clickSwitch(rendered);
    assert.deepEqual(requests.setLearning.mock.calls[0]?.[0], {
      enabled: false,
      model: "mock-model",
      memory_write_policy: "automatic",
    });
    assert.equal(switchChecked(rendered), false);
  } finally {
    act(() => rendered.root.unmount());
    rendered.container.remove();
    requests.setLearning.mockReset();
    requests.listLlmProviderModels.mockReset();
  }
});

test("controls are pending while the save is in flight", async () => {
  const { promise, resolve } = Promise.withResolvers();
  requests.setLearning.mockReturnValue(promise);
  const rendered = await renderSection(
    providerState({
      learning: { enabled: false, model: "mock-model", status: "disabled", reason: null },
    })
  );
  try {
    await clickSwitch(rendered);
    const toggle = rendered.container.querySelector<HTMLButtonElement>(
      '[data-testid="settings-learning-switch"]'
    );
    assert.ok(toggle?.disabled, "controls lock while saving");
    const status = rendered.container.querySelector(
      '[data-testid="settings-learning-status"]'
    );
    assert.match(status?.textContent ?? "", /Saving/);
    await act(async () =>
      resolve({
        providers: [],
        active: null,
        user_model_policy: null,
        learning: { enabled: true, model: "mock-model", status: "ready", reason: null },
      })
    );
    assert.equal(switchChecked(rendered), true);
  } finally {
    act(() => rendered.root.unmount());
    rendered.container.remove();
    requests.setLearning.mockReset();
  }
});
