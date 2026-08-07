// @vitest-environment happy-dom

import assert from "node:assert/strict";
import React, { act } from "react";
import { hydrateRoot } from "react-dom/client";
import { renderToStaticMarkup, renderToString } from "react-dom/server";
import { test, vi } from "vitest";

vi.mock("../../../lib/i18n", () => ({
  useT: () => (key: string) => key,
}));

const { AutomationDeliveryDefaultsPanel } = await import(
  "./automation-delivery-defaults-panel"
);

function target(id: string, overrides: Record<string, unknown> = {}) {
  return {
    target: {
      target_id: id,
      channel: "slack",
      display_name: `${id} name`,
      description: `${id} description`,
      ...overrides,
    },
    capabilities: {
      final_replies: true,
      gate_prompts: true,
      auth_prompts: true,
    },
  };
}

function renderPanel({
  targets,
  currentTarget = null,
  currentStatus = "none_configured",
  saveError = null,
}: {
  targets: ReturnType<typeof target>[];
  currentTarget?: Record<string, unknown> | null;
  currentStatus?: "none_configured" | "available" | "unavailable";
  saveError?: Error | null;
}) {
  return renderToStaticMarkup(
    <AutomationDeliveryDefaultsPanel
      deliveryState={deliveryState({
        targets,
        currentTarget,
        currentStatus,
        saveError,
      })}
    />,
  );
}

function deliveryState({
  targets,
  currentTarget = null,
  currentStatus = "none_configured",
  saveError = null,
  saveFinalReplyTarget = vi.fn(() => Promise.resolve()),
}: {
  targets: ReturnType<typeof target>[];
  currentTarget?: Record<string, unknown> | null;
  currentStatus?: "none_configured" | "available" | "unavailable";
  saveError?: Error | null;
  saveFinalReplyTarget?: ReturnType<typeof vi.fn>;
}) {
  return {
    targets,
    finalReplyTargets: targets.filter((option) => option.capabilities.final_replies),
    currentTarget,
    currentStatus,
    isLoading: false,
    isSaving: false,
    saveError,
    saveFinalReplyTarget,
  };
}

function parseMarkup(html: string) {
  const container = document.createElement("div");
  container.innerHTML = html;
  return container;
}

function buttonByLabel(markup: HTMLElement, label: string) {
  return Array.from(markup.querySelectorAll("button")).find(
    (button) => button.textContent?.trim() === label,
  );
}

test("available delivery targets remain selectable", () => {
  const html = renderPanel({
    targets: [target("slack-ready")],
  });

  assert.match(
    html,
    /<input[^>]*type="radio"[^>]*value="slack-ready"[^>]*>/,
  );
  assert.match(html, /slack-ready name/);
  assert.match(html, /automations\.delivery\.pill\.ready/);
  assert.match(html, /data-delivery-external-target-hint/);
});

test("save errors surface while Web-only deployments hide the external hint", () => {
  const markup = parseMarkup(
    renderPanel({
      targets: [],
      saveError: new Error("backend detail must not be rendered"),
    }),
  );

  assert.ok(markup.querySelector('[data-delivery-save-error=""]'));
  assert.match(
    markup.querySelector('[role="alert"]')?.textContent ?? "",
    /automations\.delivery\.saveFailed/,
  );
  assert.doesNotMatch(markup.textContent ?? "", /backend detail/);
  assert.equal(
    markup.querySelector('[data-delivery-external-target-hint=""]'),
    null,
  );
});

test("the real unavailable preference response exposes a Web App recovery path", () => {
  const html = renderPanel({
    targets: [],
    currentTarget: null,
    currentStatus: "unavailable",
  });

  const markup = parseMarkup(html);
  const unavailableStatus = markup.querySelector(
    '[data-delivery-target-status="unavailable"]',
  );
  assert.ok(unavailableStatus);
  assert.match(
    unavailableStatus.textContent ?? "",
    /automations\.delivery\.unavailableNotice.*automations\.delivery\.unavailableDesc/,
  );
  const webFallback = markup.querySelector<HTMLInputElement>(
    '[role="radiogroup"] input[value=""]',
  );
  assert.ok(webFallback);
  assert.equal(webFallback.checked, true);
  assert.equal(webFallback.disabled, false);
  assert.equal(
    buttonByLabel(markup, "automations.delivery.save")?.disabled,
    false,
  );
  assert.equal(
    buttonByLabel(markup, "automations.delivery.clear")?.disabled,
    false,
  );
  assert.ok(
    markup.querySelector('[data-delivery-external-target-hint=""]'),
  );
});

test("available targets remain selectable while an unresolved preference is recovered", () => {
  const html = renderPanel({
    targets: [target("slack-replacement")],
    currentTarget: null,
    currentStatus: "unavailable",
  });

  const markup = parseMarkup(html);
  const replacement = markup.querySelector<HTMLInputElement>(
    '[role="radiogroup"] input[value="slack-replacement"]',
  );
  assert.ok(replacement);
  assert.equal(replacement.disabled, false);
  assert.ok(
    markup.querySelector('[data-delivery-target-status="unavailable"]'),
  );
});

test("a hydrated panel recovers when the saved target no longer resolves", async () => {
  const currentTarget = {
    target_id: "slack-current",
    display_name: "Current Slack DM",
  };
  const saveFinalReplyTarget = vi.fn((_targetId: string | null) => Promise.resolve());
  const initialState = deliveryState({
    targets: [target("slack-current")],
    currentTarget,
    currentStatus: "available",
    saveFinalReplyTarget,
  });
  const container = document.createElement("div");
  container.innerHTML = renderToString(
    <AutomationDeliveryDefaultsPanel deliveryState={initialState} />,
  );
  document.body.append(container);
  const root = hydrateRoot(
    container,
    <AutomationDeliveryDefaultsPanel deliveryState={initialState} />,
  );

  try {
    await act(async () => {});
    const unavailableState = deliveryState({
      // This is the production handler shape when the stored binding no
      // longer resolves: no target summary/list option, only the status.
      targets: [],
      currentTarget: null,
      currentStatus: "unavailable",
      saveFinalReplyTarget,
    });
    await act(async () => {
      root.render(
        <AutomationDeliveryDefaultsPanel deliveryState={unavailableState} />,
      );
    });

    assert.equal(container.querySelector('input[value="slack-current"]'), null);
    assert.ok(
      container.querySelector('[data-delivery-target-status="unavailable"]'),
    );
    const saveButton = buttonByLabel(
      container,
      "automations.delivery.save",
    );
    assert.ok(saveButton);
    assert.equal(saveButton.disabled, false);
    const webFallback = container.querySelector<HTMLInputElement>(
      '[role="radiogroup"] input[value=""]',
    );
    assert.ok(webFallback);
    assert.equal(webFallback.checked, true);
    assert.equal(webFallback.disabled, false);
    assert.equal(
      buttonByLabel(container, "automations.delivery.clear")?.disabled,
      false,
    );
    await act(async () => saveButton.click());
    assert.deepEqual(saveFinalReplyTarget.mock.calls, [[null]]);
  } finally {
    await act(async () => root.unmount());
    container.remove();
  }
});
