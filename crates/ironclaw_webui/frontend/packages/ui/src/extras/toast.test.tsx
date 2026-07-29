// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React, { act } from "react";
import { renderIntoDocument } from "./test-helpers";
import {
  Toast,
  ToastDescription,
  ToastProvider,
  ToastTitle,
  ToastViewport,
  Toaster,
  dismissToast,
  toast,
} from "./toast";

test("composed Toast renders title and description in a status region", () => {
  const rendered = renderIntoDocument(
    <ToastProvider>
      <Toast open tone="positive" duration={60000}>
        <div>
          <ToastTitle>Deploy finished</ToastTitle>
          <ToastDescription>webui-v2 → production</ToastDescription>
        </div>
      </Toast>
      <ToastViewport />
    </ToastProvider>
  );
  try {
    const status = document.body.querySelector('[role="status"]');
    assert.ok(status, "toast root announces via role=status");
    assert.match(document.body.textContent ?? "", /Deploy finished/);
    assert.match(document.body.textContent ?? "", /webui-v2/);
  } finally {
    rendered.unmount();
  }
});

test("imperative toast() shows through a mounted Toaster and dismisses", () => {
  const rendered = renderIntoDocument(<Toaster />);
  try {
    let id = 0;
    act(() => {
      id = toast({ title: "Run started", description: "run-42", tone: "default" });
    });
    assert.match(document.body.textContent ?? "", /Run started/);
    act(() => {
      dismissToast(id);
    });
    // Radix keeps the closed root mounted briefly; state must be closed.
    const roots = document.body.querySelectorAll('[data-state="open"][role="status"]');
    assert.equal(roots.length, 0);
  } finally {
    rendered.unmount();
  }
});
