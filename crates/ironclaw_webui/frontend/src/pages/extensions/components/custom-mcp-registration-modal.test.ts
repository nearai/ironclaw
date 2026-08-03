// @vitest-environment happy-dom
import assert from "node:assert/strict";
import React, { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, test, vi } from "vitest";
import type { CustomMcpRegistrationPayload } from "./custom-mcp-registration-modal";

vi.mock("../../../lib/i18n", () => ({
  useT: () => (key: string) => key,
}));

const { CustomMcpRegistrationModal, customMcpIdFromName } = await import("./custom-mcp-registration-modal");

let root: Root | null = null;
let container: HTMLDivElement | null = null;

afterEach(() => {
  act(() => root?.unmount());
  container?.remove();
  root = null;
  container = null;
});

function renderModal(overrides: Partial<React.ComponentProps<typeof CustomMcpRegistrationModal>> = {}) {
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  const props: React.ComponentProps<typeof CustomMcpRegistrationModal> = {
    open: true,
    onClose: vi.fn(),
    onRegister: vi.fn(),
    isRegistering: false,
    ...overrides,
  };
  act(() => root?.render(React.createElement(CustomMcpRegistrationModal, props)));
  return props;
}

function setInput(input: HTMLInputElement, value: string) {
  act(() => {
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
    setter?.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

function clickButton(label: string) {
  const button = Array.from(document.querySelectorAll("button")).find(
    (candidate) => candidate.textContent === label,
  );
  assert.ok(button, `button ${label} should be rendered`);
  act(() => button.click());
}

function advanceToReview() {
  setInput(document.querySelectorAll<HTMLInputElement>("input")[0], "Linear MCP");
  setInput(document.querySelectorAll<HTMLInputElement>("input")[2], "https://mcp.linear.app/mcp");
  clickButton("common.continue");
}

test("derives a stable lowercase ID from the human-facing extension name", () => {
  assert.equal(customMcpIdFromName("Notion MCP / Team"), "notion-mcp-team");
  assert.equal(customMcpIdFromName("  !!!  "), "extension");
});

test("renders connection then review and submits no credentials or schemas", () => {
  let payload: CustomMcpRegistrationPayload | null = null;
  renderModal({ onRegister: (request) => { payload = request; } });

  assert.match(document.body.textContent || "", /customMcpPhase\.connection/);
  advanceToReview();
  assert.match(document.body.textContent || "", /customMcpPhase\.review/);
  clickButton("extensions.customMcpRegister");

  assert.ok(payload);
  assert.deepEqual(
    {
      desiredId: payload.desiredId,
      desiredName: payload.desiredName,
      endpoint: payload.endpoint,
      authSelection: payload.authSelection,
    },
    {
      desiredId: "linear-mcp",
      desiredName: "Linear MCP",
      endpoint: "https://mcp.linear.app/mcp",
      authSelection: { kind: "auto" },
    },
  );
  assert.equal(document.querySelector('input[type="password"]'), null);
  assert.equal(document.querySelector("textarea"), null);
});

test("keeps the generated ID hidden until Advanced options and validates each connection field inline", () => {
  renderModal();
  setInput(document.querySelectorAll<HTMLInputElement>("input")[0], "Bad\u0000name");
  setInput(document.querySelectorAll<HTMLInputElement>("input")[2], "http://example.test/mcp");
  clickButton("common.continue");

  assert.equal(document.querySelectorAll('[role="alert"]').length, 2);
  assert.match(document.body.textContent || "", /customMcpNameControls/);
  assert.match(document.body.textContent || "", /customMcpEndpointHttps/);
  assert.equal(document.querySelector('input[value="extension"]'), null);

  const advanced = document.querySelector("details");
  assert.ok(advanced);
  act(() => { advanced.open = true; });
  const advancedInput = Array.from(document.querySelectorAll<HTMLInputElement>("input")).find(
    (input) => input.value === "bad-name",
  );
  assert.ok(advancedInput);
  setInput(advancedInput, "Bad_ID");
  assert.match(document.body.textContent || "", /customMcpIdInvalid/);
  setInput(advancedInput, "bad..id");
  assert.match(document.body.textContent || "", /customMcpIdInvalid/);
});

test("review submits automatic authentication without asking the user to classify the server", () => {
  let payload: CustomMcpRegistrationPayload | null = null;
  renderModal({ onRegister: (request) => { payload = request; } });

  advanceToReview();
  assert.match(document.body.textContent || "", /customMcpReviewHint/);
  assert.equal(document.querySelector('input[type="radio"]'), null);
  clickButton("extensions.customMcpRegister");

  assert.ok(payload);
  assert.deepEqual(payload.authSelection, { kind: "auto" });
});

test("an ambiguous automatic probe stays in registration and retries with OAuth or bearer", () => {
  const payloads: CustomMcpRegistrationPayload[] = [];
  renderModal({ onRegister: (request) => { payloads.push(request); } });

  advanceToReview();
  clickButton("extensions.customMcpRegister");
  assert.deepEqual(payloads[0]?.authSelection, { kind: "auto" });

  act(() => payloads[0]?.onAuthSelectionRequired());
  const radios = Array.from(document.querySelectorAll<HTMLInputElement>('input[type="radio"]'));
  assert.deepEqual(radios.map((radio) => radio.value), ["oauth", "bearer"]);
  assert.equal(radios[0]?.checked, true);
  assert.doesNotMatch(document.body.textContent || "", /customMcpAuth\.no_auth/);

  act(() => radios[1]?.click());
  clickButton("extensions.customMcpRegister");
  assert.deepEqual(payloads[1]?.authSelection, { kind: "bearer" });
  assert.match(document.body.textContent || "", /customMcpPhase\.review/);

  clickButton("common.back");
  setInput(document.querySelectorAll<HTMLInputElement>("input")[2], "https://mcp.example.com/mcp");
  clickButton("common.continue");
  assert.equal(document.querySelector('input[type="radio"]'), null);
  clickButton("extensions.customMcpRegister");
  assert.deepEqual(payloads[2]?.authSelection, { kind: "auto" });
});

test("registration completion renders step three and Done closes", () => {
  let payload: CustomMcpRegistrationPayload | null = null;
  const onClose = vi.fn();
  renderModal({
    onRegister: (request) => { payload = request; },
    onClose,
  });
  advanceToReview();
  clickButton("extensions.customMcpRegister");
  assert.ok(payload);

  act(() => payload?.onRegistered());
  assert.match(document.body.textContent || "", /customMcpPhase\.result/);
  assert.match(document.body.textContent || "", /customMcpReady/);
  clickButton("common.done");
  assert.equal(onClose.mock.calls.length, 1);
});

test("registration errors stay in the registration modal", () => {
  let payload: CustomMcpRegistrationPayload | null = null;
  renderModal({ onRegister: (request) => { payload = request; } });
  advanceToReview();
  clickButton("extensions.customMcpRegister");
  assert.ok(payload);

  act(() => payload?.onRegistrationError("Server rejected registration"));
  assert.equal(document.querySelector('[role="alert"]')?.textContent, "Server rejected registration");
  assert.ok(document.querySelector('[role="dialog"]'));
  clickButton("common.back");
  const inputs = Array.from(document.querySelectorAll<HTMLInputElement>("input"));
  assert.deepEqual(inputs.map((input) => input.value), ["Linear MCP", "linear-mcp", "https://mcp.linear.app/mcp"]);
});
