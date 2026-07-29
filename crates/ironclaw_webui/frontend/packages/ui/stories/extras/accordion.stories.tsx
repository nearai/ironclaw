import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger,
} from "../../src/extras/accordion";

const meta: Meta = { title: "Extras/Accordion" };
export default meta;

type Story = StoryObj;

export const Single: Story = {
  render: () => (
    <Accordion type="single" collapsible className="w-80">
      <AccordionItem value="tokens">
        <AccordionTrigger>What are the v2 tokens?</AccordionTrigger>
        <AccordionContent>
          CSS custom properties defined in tokens.css. Both light and dark
          themes redefine them, so components restyle automatically.
        </AccordionContent>
      </AccordionItem>
      <AccordionItem value="motion">
        <AccordionTrigger>Is it animated?</AccordionTrigger>
        <AccordionContent>
          Only the chevron rotates; the app runs a static-motion policy.
        </AccordionContent>
      </AccordionItem>
      <AccordionItem value="disabled" disabled>
        <AccordionTrigger>Disabled section</AccordionTrigger>
        <AccordionContent>Never visible.</AccordionContent>
      </AccordionItem>
    </Accordion>
  ),
};

export const Multiple: Story = {
  render: () => (
    <Accordion type="multiple" defaultValue={["a", "b"]} className="w-80">
      <AccordionItem value="a">
        <AccordionTrigger>First (open)</AccordionTrigger>
        <AccordionContent>Multiple sections can stay open.</AccordionContent>
      </AccordionItem>
      <AccordionItem value="b">
        <AccordionTrigger>Second (open)</AccordionTrigger>
        <AccordionContent>Independently collapsible.</AccordionContent>
      </AccordionItem>
    </Accordion>
  ),
};
