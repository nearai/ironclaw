import assert from "node:assert/strict";
import { type ReactElement } from "react";
import { test } from "vitest";

import { StatusDot } from "./status-dot";

type DotProps = { className?: string; "aria-hidden"?: string | boolean };

test("StatusDot without a tone inherits the current text color", () => {
  const rendered = StatusDot({}) as ReactElement<DotProps>;
  assert.match(rendered.props.className ?? "", /bg-current/);
  assert.equal(rendered.props["aria-hidden"], "true");
});

test("StatusDot maps tones to tokens and unknown tones fall back to muted", () => {
  const success = StatusDot({ tone: "success" }) as ReactElement<DotProps>;
  assert.match(success.props.className ?? "", /--v2-positive-text/);

  // Presenter data can carry stale tone strings; render muted, never crash.
  const unknown = StatusDot({ tone: "bogus" as never }) as ReactElement<DotProps>;
  assert.match(unknown.props.className ?? "", /--v2-text-faint/);
});

test("StatusDot pulse opts into the sanctioned breathe keyframe", () => {
  const rendered = StatusDot({ pulse: true }) as ReactElement<DotProps>;
  assert.match(rendered.props.className ?? "", /v2-breathe/);
});
