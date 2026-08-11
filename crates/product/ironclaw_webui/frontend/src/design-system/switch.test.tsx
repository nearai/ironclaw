import assert from "node:assert/strict";
import type { ReactElement } from "react";
import { test } from "vitest";

import { Switch } from "./switch";

type SwitchElementProps = {
  "aria-checked"?: boolean;
  "aria-label"?: string;
  "aria-labelledby"?: string;
  children?: ReactElement<{ "aria-hidden"?: string; className?: string }>;
  className?: string;
  disabled?: boolean;
  onClick?: () => void;
  role?: string;
  type?: string;
};

test("Switch exposes controlled switch semantics and toggles from the current value", () => {
  const changes: boolean[] = [];
  const rendered = Switch({
    checked: false,
    "aria-label": "Enable planning",
    onChange: (checked) => changes.push(checked),
  }) as ReactElement<SwitchElementProps>;

  assert.equal(rendered.type, "button");
  assert.equal(rendered.props.type, "button");
  assert.equal(rendered.props.role, "switch");
  assert.equal(rendered.props["aria-checked"], false);
  assert.equal(rendered.props["aria-label"], "Enable planning");
  assert.match(rendered.props.className ?? "", /\bh-7\b/);
  assert.match(rendered.props.className ?? "", /\bw-12\b/);

  rendered.props.onClick?.();
  assert.deepEqual(changes, [true]);
});

test("Switch supports an aria-labelledby accessible name", () => {
  const rendered = Switch({
    checked: true,
    "aria-labelledby": "planning-label",
    onChange: () => {},
  }) as ReactElement<SwitchElementProps>;

  assert.equal(rendered.props["aria-label"], undefined);
  assert.equal(rendered.props["aria-labelledby"], "planning-label");
});

test("Switch disabled state blocks changes and uses native disabled behavior", () => {
  const changes: boolean[] = [];
  const rendered = Switch({
    checked: true,
    disabled: true,
    "aria-label": "Enable planning",
    onChange: (checked) => changes.push(checked),
  }) as ReactElement<SwitchElementProps>;

  rendered.props.onClick?.();
  assert.equal(rendered.props.disabled, true);
  assert.match(rendered.props.className ?? "", /cursor-not-allowed/);
  assert.deepEqual(changes, []);
});

for (const {
  size,
  trackHeight,
  trackWidth,
  trackPadding,
} of [
  { size: "sm", trackHeight: "h-6", trackWidth: "w-11", trackPadding: "p-px" },
  { size: "md", trackHeight: "h-7", trackWidth: "w-12", trackPadding: "p-[3px]" },
] as const) {
  test(`Switch ${size} size centers the thumb at both endpoints`, () => {
    const render = (checked: boolean) =>
      Switch({
        checked,
        size,
        "aria-label": "Enable planning",
        onChange: () => {},
      }) as ReactElement<SwitchElementProps>;
    const unchecked = render(false);
    const checked = render(true);

    assert.match(unchecked.props.className ?? "", new RegExp(`\\b${trackHeight}\\b`));
    assert.match(unchecked.props.className ?? "", new RegExp(`\\b${trackWidth}\\b`));
    assert.ok(unchecked.props.className?.includes(trackPadding));
    assert.match(unchecked.props.children?.props.className ?? "", /\bh-5\b/);
    assert.match(unchecked.props.children?.props.className ?? "", /\bw-5\b/);
    assert.match(
      unchecked.props.children?.props.className ?? "",
      /translate-x-0/
    );
    assert.match(checked.props.children?.props.className ?? "", /translate-x-5/);
    assert.equal(unchecked.props.children?.props["aria-hidden"], "true");
  });
}
