// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";

import { runVmModuleForTest } from "../test-support/vm-module-harness.ts";

function setupModalContext(translate = (key) => key) {
  const context = {
    React: {
      useEffect: () => {},
    },
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    cn: (...classes) => classes.filter(Boolean).join(" "),
    Icon() {},
    useT: () => translate,
    document: { body: { style: {} } },
    window: {
      addEventListener() {},
      removeEventListener() {},
    },
    globalThis: {},
  };
  runVmModuleForTest("./modal.ts", ["Modal", "ModalHeader"], context, import.meta.url);
  return context;
}

test("ModalHeader falls back to the localized close label", () => {
  const context = setupModalContext((key) =>
    key === "common.close" ? "Localized close" : key,
  );
  const { ModalHeader } = context.globalThis.__testExports;

  const rendered = ModalHeader({
    children: "Settings",
    onClose: () => {},
  });

  assert.match(JSON.stringify(rendered), /Localized close/);
});

test("Modal title shortcut passes an explicit close label to ModalHeader", () => {
  const context = setupModalContext();
  const { Modal, ModalHeader } = context.globalThis.__testExports;

  const rendered = Modal({
    open: true,
    onClose: () => {},
    title: "Settings",
    closeLabel: "Dismiss settings",
    children: "Body",
  });
  const header = rendered.values.find(
    (value) => value?.values?.[0] === ModalHeader,
  );

  assert.ok(header, "Modal should render ModalHeader for the title shortcut");
  assert.equal(header.values[2], "Dismiss settings");
});
