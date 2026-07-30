import assert from "node:assert/strict";
import { type ReactElement } from "react";
import { test } from "vitest";

import { FormField } from "./form-field";

test("FormField error wins over hint and carries role=alert", () => {
  const rendered = FormField({
    label: "API key",
    error: "This key is invalid.",
    hint: "Find it in the dashboard.",
    children: "CONTROL",
  }) as ReactElement;

  const markup = JSON.stringify(rendered);
  assert.match(markup, /This key is invalid\./);
  assert.match(markup, /"role":"alert"/);
  assert.doesNotMatch(markup, /Find it in the dashboard\./);
});

test("FormField renders the hint when there is no error", () => {
  const rendered = FormField({
    label: "API key",
    hint: "Find it in the dashboard.",
    children: "CONTROL",
  }) as ReactElement;

  assert.match(JSON.stringify(rendered), /Find it in the dashboard\./);
});
