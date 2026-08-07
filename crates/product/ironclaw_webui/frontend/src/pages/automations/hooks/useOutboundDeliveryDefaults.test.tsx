// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { test, vi } from "vitest";

vi.mock("../../../lib/i18n", () => ({
  useT: () => (key: string) => key,
}));

import { AutomationDeliveryDefaultsPanel } from "../components/automation-delivery-defaults-panel";
import { useOutboundDeliveryDefaults } from "./useOutboundDeliveryDefaults";

const preferencesQueryKey = ["outbound-delivery", "preferences"];
const targetsQueryKey = ["outbound-delivery", "targets"];

test("the handler's unavailable JSON shape renders a clearable Web App fallback", () => {
  const queryClient = new QueryClient({
    defaultOptions: {
      mutations: { retry: false },
      queries: { retry: false, staleTime: Infinity },
    },
  });
  // RebornOutboundPreferencesResponse omits final_reply_target when the saved
  // binding no longer resolves; status is the only recovery signal.
  queryClient.setQueryData(preferencesQueryKey, {
    final_reply_target_status: "unavailable",
    default_modality: "text",
  });
  queryClient.setQueryData(targetsQueryKey, { targets: [] });

  function Harness() {
    const deliveryState = useOutboundDeliveryDefaults();
    return <AutomationDeliveryDefaultsPanel deliveryState={deliveryState} />;
  }

  try {
    const html = renderToStaticMarkup(
      <QueryClientProvider client={queryClient}>
        <Harness />
      </QueryClientProvider>,
    );
    const markup = document.createElement("div");
    markup.innerHTML = html;

    assert.ok(
      markup.querySelector('[data-delivery-target-status="unavailable"]'),
    );
    const webFallback = markup.querySelector<HTMLInputElement>(
      '[role="radiogroup"] input[value=""]',
    );
    assert.ok(webFallback);
    assert.equal(webFallback.checked, true);
    assert.equal(webFallback.disabled, false);
    const buttons = Array.from(markup.querySelectorAll("button"));
    assert.equal(
      buttons.find(
        (button) =>
          button.textContent?.trim() === "automations.delivery.save",
      )?.disabled,
      false,
    );
    assert.equal(
      buttons.find(
        (button) =>
          button.textContent?.trim() === "automations.delivery.clear",
      )?.disabled,
      false,
    );
  } finally {
    queryClient.clear();
  }
});
