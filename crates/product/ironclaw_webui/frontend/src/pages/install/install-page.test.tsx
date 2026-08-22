// @vitest-environment happy-dom
// @ts-nocheck
import assert from "node:assert/strict";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { test, vi } from "vitest";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

let currentSearch = "";

vi.mock("react-router", () => ({
  useLocation: () => ({ search: currentSearch }),
}));
vi.mock("../../lib/i18n", () => ({ useT: () => (key) => key, registerPack: () => {} }));

const { InstallPage } = await import("./install-page");

const SIGNED =
  "?slug=attio&version=0.1.0&uid=u1&aid=a1&ts=1750000000&nonce=n1" +
  "&artifact_digest=sha256%3Aabcdef&sig=deadbeef";

function render(search) {
  currentSearch = search;
  const container = document.createElement("div");
  document.body.append(container);
  act(() => createRoot(container).render(<InstallPage />));
  return container;
}

test("the artifact digest is shown before anything can be installed", () => {
  const container = render(SIGNED);
  const text = container.textContent;

  assert.ok(text.includes("sha256:abcdef"), "the approved digest must be visible to the approver");
  assert.ok(text.includes("attio"), "the caller must see what is being installed");
  assert.ok(text.includes("0.1.0"));
});

test("nothing installs without a click", () => {
  const container = render(SIGNED);
  assert.ok(
    container.textContent.includes("ironhub.install.confirm"),
    "the install must wait on an explicit confirmation",
  );
});

test("a private manifest source is disclosed on the consent card", () => {
  const container = render(
    `${SIGNED}&private_manifest_url=${encodeURIComponent("https://hub.ironclaw.com/private/attio.json")}`,
  );

  assert.ok(
    container.textContent.includes("ironhub.install.privateSource"),
    "an install drawing from a private manifest must say so before it is approved",
  );
});

test("an incomplete link refuses rather than offering to install", () => {
  const container = render("?slug=attio&version=0.1.0");
  const text = container.textContent;

  assert.ok(text.includes("ironhub.install.linkInvalid"));
  assert.ok(
    !text.includes("ironhub.install.confirm"),
    "an unverifiable link must not present an install button",
  );
});
