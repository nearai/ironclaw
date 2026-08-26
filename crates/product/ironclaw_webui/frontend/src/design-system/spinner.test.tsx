import assert from "node:assert/strict";
import type { ReactElement } from "react";
import { test, vi } from "vitest";

vi.mock("../lib/i18n", () => ({
  useT: () => (key: string) => `localized:${key}`,
}));

import { Spinner } from "./spinner";

test("Spinner localizes its default assistive label", () => {
  const rendered = Spinner() as ReactElement<{ "aria-label"?: string }>;

  assert.equal(rendered.props["aria-label"], "localized:common.loading");
});

test("Spinner preserves a caller-supplied assistive label", () => {
  const rendered = Spinner({
    "aria-label": "Loading account",
  }) as ReactElement<{ "aria-label"?: string }>;

  assert.equal(rendered.props["aria-label"], "Loading account");
});
