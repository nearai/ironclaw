// @vitest-environment happy-dom

import assert from "node:assert/strict";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { test } from "vitest";

import { InlineNotice, type InlineNoticeTone } from "./inline-notice";

const EXPECTED_TOKENS: Record<InlineNoticeTone, string> = {
  info: "--v2-info-text",
  success: "--v2-positive-text",
  warning: "--v2-warning-text",
  danger: "--v2-danger-text",
};

for (const tone of Object.keys(EXPECTED_TOKENS) as InlineNoticeTone[]) {
  test(`InlineNotice renders the ${tone} semantic tone`, () => {
    const html = renderToStaticMarkup(
      <InlineNotice tone={tone} role="status">
        Saved
      </InlineNotice>,
    );

    assert.match(html, new RegExp(`data-tone="${tone}"`));
    assert.match(html, new RegExp(EXPECTED_TOKENS[tone]));
    assert.match(html, /role="status"/);
  });
}

test("InlineNotice renders optional actions and an accessible dismissal", () => {
  const html = renderToStaticMarkup(
    <InlineNotice
      tone="warning"
      role="alert"
      action={<button type="button">Retry</button>}
      onDismiss={() => {}}
      dismissLabel="Dismiss notice"
    >
      Some data is unavailable.
    </InlineNotice>,
  );

  assert.match(html, /role="alert"/);
  assert.match(html, />Retry</);
  assert.match(html, /aria-label="Dismiss notice"/);
  assert.match(html, /Some data is unavailable\./);
});

test("InlineNotice calls the consumer dismissal handler", () => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  let dismissals = 0;

  try {
    act(() => {
      root.render(
        <InlineNotice
          tone="info"
          role="status"
          onDismiss={() => {
            dismissals += 1;
          }}
          dismissLabel="Dismiss notice"
        >
          Ready
        </InlineNotice>,
      );
    });
    act(() => container.querySelector<HTMLButtonElement>("button[aria-label]")?.click());

    assert.equal(dismissals, 1);
  } finally {
    act(() => root.unmount());
    container.remove();
  }
});
