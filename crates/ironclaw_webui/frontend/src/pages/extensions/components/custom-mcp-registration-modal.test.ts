// @vitest-environment happy-dom
import assert from "node:assert/strict";
import React, { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, test, vi } from "vitest";
import type { CustomMcpRegistrationPayload } from "./custom-mcp-registration-modal";

vi.mock("../../../lib/i18n", () => ({
  useT: () => (key: string) => key,
}));

const { CustomMcpRegistrationModal } = await import("./custom-mcp-registration-modal");

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
    onSetup: vi.fn(),
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

function advanceToAuthentication() {
  const inputs = Array.from(document.querySelectorAll<HTMLInputElement>("input"));
  setInput(inputs[0], "Linear MCP");
  setInput(inputs[1], "linear");
  setInput(inputs[2], "https://mcp.linear.app/mcp");
  clickButton("common.continue");
}

test("renders connection then authentication and submits no credentials or schemas", () => {
  let payload: CustomMcpRegistrationPayload | null = null;
  renderModal({ onRegister: (request) => { payload = request; } });

  assert.match(document.body.textContent || "", /customMcpPhase\.connection/);
  advanceToAuthentication();
  assert.match(document.body.textContent || "", /customMcpPhase\.authentication/);
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
      desiredId: "linear",
      desiredName: "Linear MCP",
      endpoint: "https://mcp.linear.app/mcp",
      authSelection: { kind: "no_auth" },
    },
  );
  assert.equal(document.querySelector('input[type="password"]'), null);
  assert.equal(document.querySelector("textarea"), null);
});

test("OAuth registration submits without an unsupported client profile", () => {
  let payload: CustomMcpRegistrationPayload | null = null;
  renderModal({ onRegister: (request) => { payload = request; } });

  advanceToAuthentication();
  const oauth = Array.from(document.querySelectorAll<HTMLInputElement>('input[type="radio"]')).find(
    (input) => input.parentElement?.textContent === "extensions.customMcpAuth.oauth",
  );
  assert.ok(oauth);
  act(() => oauth.click());
  clickButton("extensions.customMcpRegister");

  assert.ok(payload);
  assert.deepEqual(payload.authSelection, { kind: "oauth" });
  assert.equal(document.body.textContent?.includes("customMcpProfile"), false);
});

test("authoritative active result renders step three and Done closes", () => {
  let payload: CustomMcpRegistrationPayload | null = null;
  const onClose = vi.fn();
  renderModal({
    onRegister: (request) => { payload = request; },
    onClose,
  });
  advanceToAuthentication();
  clickButton("extensions.customMcpRegister");
  assert.ok(payload);

  act(() => payload?.onRegistered(null));
  assert.match(document.body.textContent || "", /customMcpPhase\.result/);
  assert.match(document.body.textContent || "", /customMcpReady/);
  clickButton("common.done");
  assert.equal(onClose.mock.calls.length, 1);
});

test("authoritative setup result renders step three and hands off to existing setup", () => {
  let payload: CustomMcpRegistrationPayload | null = null;
  const setupResults: unknown[] = [];
  renderModal({
    onRegister: (request) => { payload = request; },
    onSetup: (extension) => setupResults.push(extension),
  });
  advanceToAuthentication();
  clickButton("extensions.customMcpRegister");
  assert.ok(payload);

  const setupExtension = { packageRef: { kind: "extension", id: "mcp-linear" } };
  act(() => payload?.onNeedsSetup(setupExtension));
  assert.match(document.body.textContent || "", /customMcpSetupRequired/);
  clickButton("extensions.customMcpContinueSetup");
  assert.deepEqual(setupResults, [setupExtension]);
});

test("registration errors stay in the registration modal", () => {
  let payload: CustomMcpRegistrationPayload | null = null;
  renderModal({ onRegister: (request) => { payload = request; } });
  advanceToAuthentication();
  clickButton("extensions.customMcpRegister");
  assert.ok(payload);

  act(() => payload?.onRegistrationError("Server rejected registration"));
  assert.equal(document.querySelector('[role="alert"]')?.textContent, "Server rejected registration");
  assert.ok(document.querySelector('[role="dialog"]'));
  clickButton("common.back");
  const inputs = Array.from(document.querySelectorAll<HTMLInputElement>("input"));
  assert.deepEqual(inputs.map((input) => input.value), [
    "Linear MCP",
    "linear",
    "https://mcp.linear.app/mcp",
  ]);
});
