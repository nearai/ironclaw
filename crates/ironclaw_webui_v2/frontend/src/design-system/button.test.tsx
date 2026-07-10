import assert from "node:assert/strict";
import type { ReactElement } from "react";
import { test } from "vitest";

import { Button } from "./button";

type ButtonElementProps = {
  disabled?: boolean;
  "aria-disabled"?: boolean;
  tabIndex?: number;
  to?: string;
  onClick?: (event: { preventDefault: () => void; stopPropagation: () => void }) => void;
};

function LinkLike() {
  return null;
}

test("disabled Link-like Buttons use aria-disabled without native disabled", () => {
  const rendered = Button({
    as: LinkLike,
    to: "/chat/thread-a",
    disabled: true,
    children: "Open run",
  }) as ReactElement<ButtonElementProps>;

  assert.equal(rendered.type, LinkLike);
  assert.equal(rendered.props.to, "/chat/thread-a");
  assert.equal(rendered.props.disabled, undefined);
  assert.equal(rendered.props["aria-disabled"], true);
  assert.equal(rendered.props.tabIndex, -1);

  let prevented = false;
  let stopped = false;
  rendered.props.onClick?.({
    preventDefault: () => {
      prevented = true;
    },
    stopPropagation: () => {
      stopped = true;
    },
  });
  assert.equal(prevented, true);
  assert.equal(stopped, true);
});

test("disabled native Buttons keep the disabled attribute", () => {
  const rendered = Button({
    disabled: true,
    children: "Save",
  }) as ReactElement<ButtonElementProps>;

  assert.equal(rendered.type, "button");
  assert.equal(rendered.props.disabled, true);
  assert.equal(rendered.props["aria-disabled"], undefined);
});
