// @vitest-environment happy-dom

import assert from "node:assert/strict";
import { test } from "vitest";
import React from "react";
import { renderIntoDocument } from "./test-helpers";
import { Accordion, AccordionContent, AccordionItem, AccordionTrigger } from "./accordion";

test("Accordion renders an open item with trigger + region semantics", () => {
  const rendered = renderIntoDocument(
    <Accordion type="single" defaultValue="a" collapsible>
      <AccordionItem value="a">
        <AccordionTrigger>Section A</AccordionTrigger>
        <AccordionContent>Body A</AccordionContent>
      </AccordionItem>
      <AccordionItem value="b">
        <AccordionTrigger>Section B</AccordionTrigger>
        <AccordionContent>Body B</AccordionContent>
      </AccordionItem>
    </Accordion>
  );
  try {
    const triggers = rendered.container.querySelectorAll("button");
    assert.equal(triggers.length, 2);
    assert.equal(triggers[0].getAttribute("aria-expanded"), "true");
    assert.equal(triggers[1].getAttribute("aria-expanded"), "false");
    assert.match(rendered.container.textContent ?? "", /Body A/);
    assert.ok(rendered.container.querySelector('[role="region"]'));
  } finally {
    rendered.unmount();
  }
});
