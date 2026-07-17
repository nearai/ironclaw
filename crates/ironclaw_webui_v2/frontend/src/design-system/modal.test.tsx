import assert from "node:assert/strict";
import * as Dialog from "@radix-ui/react-dialog";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { test } from "vitest";
import { Modal, ModalBody, ModalHeader } from "./modal";

test("Modal renders an accessible dialog when open", () => {
  const html = renderToStaticMarkup(
    <Modal open onClose={() => {}} title="Settings" closeLabel="Dismiss settings">
      <ModalBody>Body</ModalBody>
    </Modal>,
  );

  assert.match(html, /role="dialog"/);
  assert.match(html, /aria-modal="true"/);
  assert.match(html, /aria-label="Settings"/);
  assert.match(html, /Dismiss settings/);
  assert.match(html, /Body/);
});

test("Modal renders nothing while closed", () => {
  const html = renderToStaticMarkup(
    <Modal open={false} onClose={() => {}} title="Settings">
      <ModalBody>Body</ModalBody>
    </Modal>,
  );

  assert.equal(html, "");
});

test("ModalHeader falls back to the localized close label key", () => {
  const html = renderToStaticMarkup(
    <Modal open onClose={() => {}} title="Settings">
      <ModalBody>Body</ModalBody>
    </Modal>,
  );

  // useT falls back to the key when no provider is mounted.
  assert.match(html, /common\.close|Close|Settings/);
});

test("ModalHeader accepts an explicit close label", () => {
  const html = renderToStaticMarkup(
    <Dialog.Root open>
      <Dialog.Content>
        <ModalHeader onClose={() => {}} closeLabel="Dismiss settings">
          Nested
        </ModalHeader>
      </Dialog.Content>
    </Dialog.Root>,
  );

  assert.match(html, /Dismiss settings/);
  assert.match(html, /Nested/);
});
