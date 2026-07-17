// @ts-nocheck
import assert from "node:assert/strict";
import { test } from "vitest";

import { runVmModuleForTest } from "../test-support/vm-module-harness";

function setupModalContext(translate = (key) => key) {
  const context = {
    React: {
      useEffect: () => {},
    },
    html: (strings, ...values) => ({ strings: Array.from(strings), values }),
    cn: (...classes) => classes.filter(Boolean).join(" "),
    Icon() {},
    useT: () => translate,
    // motion/react + ./motion stubs (imports are stripped by the vm
    // harness; the identifiers must exist).
    motion: { div: "motion.div", span: "motion.span" },
    AnimatePresence: function AnimatePresence() {},
    useReducedMotion: () => false,
    MOTION_DURATION: { instant: 0.1, exit: 0.12, fast: 0.15, menu: 0.18, base: 0.25 },
    MOTION_EASE_OUT: [0.16, 1, 0.3, 1],
    document: { body: { style: {} } },
    window: {
      addEventListener() {},
      removeEventListener() {},
    },
    globalThis: {},
  };
  runVmModuleForTest("./modal.tsx", ["Modal", "ModalHeader"], context, import.meta.url);
  return context;
}

function renderedNodeOfType(rendered, type) {
  if (!rendered || typeof rendered !== "object") return undefined;
  if (rendered.type === type) return rendered;
  const values = [
    ...(Array.isArray(rendered.values) ? rendered.values : []),
    ...(Array.isArray(rendered.children) ? rendered.children : []),
  ];
  for (const value of values) {
    const found = renderedNodeOfType(value, type);
    if (found) return found;
  }
  return undefined;
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
  const header = renderedNodeOfType(rendered, ModalHeader);

  assert.ok(header, "Modal should render ModalHeader for the title shortcut");
  assert.equal(header.props.closeLabel, "Dismiss settings");
});
