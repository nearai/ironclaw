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
  setPolicy: vi.fn(),
}));

vi.mock("../lib/settings-api", () => ({
  setUserModelPolicy: requests.setPolicy,
}));

import { ModelSelectionPolicyEditor } from "./model-selection-policy-editor";

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
    listModels: vi.fn(),
    ...overrides,
  };
}

async function renderEditor(state) {
  const container = document.createElement("div");
  document.body.append(container);
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const root = createRoot(container);
  await act(async () => {
    root.render(
      <QueryClientProvider client={queryClient}>
        <I18nProvider>
          <ModelSelectionPolicyEditor providerState={state} />
        </I18nProvider>
      </QueryClientProvider>
    );
  });
  return { container, queryClient, root };
}

function setInputValue(input, value) {
  const setter = Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    "value"
  )?.set;
  setter?.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

test("admin can enable selection from the active model and a manually added model", async () => {
  const savedCatalog = {
    selection_enabled: true,
    workspace_default: "canonical-model",
    models: ["canonical-model"],
  };
  requests.setPolicy.mockResolvedValue(savedCatalog);
  const rendered = await renderEditor(providerState());
  try {
    rendered.queryClient.setQueryData(["llm-providers"], {
      user_model_policy: null,
    });
    const activeModel = rendered.container.querySelector<HTMLInputElement>(
      '[data-testid="settings-model-policy-model-mock-model"]'
    );
    assert.equal(activeModel?.checked, true, "the active model seeds a safe initial policy");

    const input = rendered.container.querySelector<HTMLInputElement>(
      '[data-testid="settings-model-policy-model-input"]'
    );
    assert.ok(input);
    await act(async () => {
      setInputValue(input, "e2e-selected-model");
    });
    const add = rendered.container.querySelector<HTMLButtonElement>(
      '[data-testid="settings-model-policy-add-model"]'
    );
    assert.ok(add);
    await act(async () => add.click());

    const added = rendered.container.querySelector<HTMLInputElement>(
      '[data-testid="settings-model-policy-model-e2e-selected-model"]'
    );
    assert.equal(added?.checked, true);

    const save = rendered.container.querySelector<HTMLButtonElement>(
      '[data-testid="settings-model-policy-save"]'
    );
    assert.ok(save);
    await act(async () => save.click());

    assert.deepEqual(requests.setPolicy.mock.calls[0]?.[0], {
      workspace_default: "mock-model",
      allowed_models: ["mock-model", "e2e-selected-model"],
    });
    assert.deepEqual(
      rendered.queryClient.getQueryData(["user-model-catalog"]),
      savedCatalog
    );
    assert.deepEqual(rendered.queryClient.getQueryData(["llm-providers"]), {
      user_model_policy: {
        provider_id: "openai_compatible",
        workspace_default: "canonical-model",
        allowed_models: ["canonical-model"],
      },
    });
    const canonicalModel = rendered.container.querySelector<HTMLInputElement>(
      '[data-testid="settings-model-policy-model-canonical-model"]'
    );
    assert.equal(canonicalModel?.checked, true);
    assert.match(
      rendered.container.querySelector('[data-testid="settings-model-policy-status"]')
        ?.textContent ?? "",
      /Model selection enabled/
    );
  } finally {
    act(() => rendered.root.unmount());
    rendered.container.remove();
    requests.setPolicy.mockReset();
  }
});

test("policy editor fails closed without an active provider", async () => {
  const rendered = await renderEditor(
    providerState({ activeProviderId: null, selectedModel: "", providers: [] })
  );
  try {
    const save = rendered.container.querySelector<HTMLButtonElement>(
      '[data-testid="settings-model-policy-save"]'
    );
    assert.equal(save, null, "policy mutation controls stay hidden without a provider");
    assert.match(rendered.container.textContent ?? "", /Configure an active provider first/);
  } finally {
    act(() => rendered.root.unmount());
    rendered.container.remove();
  }
});

test("policy editor does not expose native request errors", async () => {
  requests.setPolicy.mockRejectedValue(new Error("browser save details"));
  const state = providerState({
    listModels: vi.fn().mockRejectedValue(new Error("browser network details")),
  });
  const rendered = await renderEditor(state);
  try {
    const fetchModels = Array.from(
      rendered.container.querySelectorAll<HTMLButtonElement>("button")
    ).find((button) => button.textContent === "Fetch models");
    assert.ok(fetchModels);
    await act(async () => fetchModels.click());

    const status = rendered.container.querySelector(
      '[data-testid="settings-model-policy-status"]'
    );
    assert.equal(status?.textContent, "Could not load models.");
    assert.doesNotMatch(status?.textContent ?? "", /browser network details/);

    const save = rendered.container.querySelector<HTMLButtonElement>(
      '[data-testid="settings-model-policy-save"]'
    );
    assert.ok(save);
    await act(async () => save.click());

    assert.equal(status?.textContent, "Could not save the model selection policy.");
    assert.doesNotMatch(status?.textContent ?? "", /browser save details/);
  } finally {
    act(() => rendered.root.unmount());
    rendered.container.remove();
    requests.setPolicy.mockReset();
  }
});

test("policy editor preserves sanitized API errors", async () => {
  requests.setPolicy.mockRejectedValue(new ApiError("policy rejected"));
  const rendered = await renderEditor(providerState());
  try {
    const save = rendered.container.querySelector<HTMLButtonElement>(
      '[data-testid="settings-model-policy-save"]'
    );
    assert.ok(save);
    await act(async () => save.click());

    assert.equal(
      rendered.container.querySelector('[data-testid="settings-model-policy-status"]')
        ?.textContent,
      "policy rejected"
    );
  } finally {
    act(() => rendered.root.unmount());
    rendered.container.remove();
    requests.setPolicy.mockReset();
  }
});
