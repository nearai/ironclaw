import assert from "node:assert/strict";
import { type ReactElement } from "react";
import { test } from "vitest";

import { Callout } from "./callout";

type CalloutElementProps = {
  role?: string;
  className?: string;
  children?: unknown;
};

test("Callout danger tone renders role=alert; others render role=status", () => {
  const danger = Callout({ tone: "danger", children: "Boom" }) as ReactElement<CalloutElementProps>;
  assert.equal(danger.props.role, "alert");

  for (const tone of ["info", "success", "warning"] as const) {
    const rendered = Callout({ tone, children: "Note" }) as ReactElement<CalloutElementProps>;
    assert.equal(rendered.props.role, "status");
  }
});

test("Callout warning tone maps to the warning tokens", () => {
  const rendered = Callout({ tone: "warning", children: "Careful" }) as ReactElement<CalloutElementProps>;
  assert.match(rendered.props.className ?? "", /--v2-warning-soft/);
});

test("Callout renders title, body, actions and dismiss affordance", () => {
  const rendered = Callout({
    title: "Restart required",
    children: "Changes apply after restart.",
    actions: "ACTIONS_SLOT",
    onDismiss: () => {},
    dismissLabel: "Dismiss",
  }) as ReactElement<CalloutElementProps>;

  const markup = JSON.stringify(rendered);
  assert.match(markup, /Restart required/);
  assert.match(markup, /Changes apply after restart\./);
  assert.match(markup, /ACTIONS_SLOT/);
  assert.match(markup, /Dismiss/);
});
