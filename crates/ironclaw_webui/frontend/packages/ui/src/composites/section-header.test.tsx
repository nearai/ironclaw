import assert from "node:assert/strict";
import { type ReactElement } from "react";
import { test } from "vitest";

import { SectionHeader } from "./section-header";

test("SectionHeader renders eyebrow, title, description and actions", () => {
  const rendered = SectionHeader({
    eyebrow: "Explorer",
    title: "Job queue",
    description: "Search and stop active work.",
    actions: "ACTIONS_SLOT",
  }) as ReactElement;

  const markup = JSON.stringify(rendered);
  assert.match(markup, /Explorer/);
  assert.match(markup, /Job queue/);
  assert.match(markup, /Search and stop active work\./);
  assert.match(markup, /ACTIONS_SLOT/);
  // Default heading level is h2; pages pass titleAs="h1" at the top level.
  assert.match(markup, /"h2"/);
});

test("SectionHeader titleAs switches the heading element", () => {
  const rendered = SectionHeader({ title: "Jobs", titleAs: "h1" }) as ReactElement;
  assert.match(JSON.stringify(rendered), /"h1"/);
});
