import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Popover, PopoverContent, PopoverTrigger } from "../../src/extras/popover";
import { Button } from "../../src/components/button";
import { FormField, Input } from "../../src/components/input";

const meta: Meta = { title: "Extras/Popover" };
export default meta;

type Story = StoryObj;

export const Default: Story = {
  render: () => (
    <Popover>
      <PopoverTrigger asChild>
        <Button variant="secondary" size="sm">Dimensions</Button>
      </PopoverTrigger>
      <PopoverContent>
        <div className="flex flex-col gap-3">
          <span className="text-ui font-semibold text-[var(--v2-text-strong)]">
            Set dimensions
          </span>
          <FormField label="Width" htmlFor="pop-width">
            <Input id="pop-width" size="sm" defaultValue="320" />
          </FormField>
          <FormField label="Height" htmlFor="pop-height">
            <Input id="pop-height" size="sm" defaultValue="180" />
          </FormField>
        </div>
      </PopoverContent>
    </Popover>
  ),
};

export const Alignments: Story = {
  render: () => (
    <div className="flex gap-4">
      {(["start", "center", "end"] as const).map((align) => (
        <Popover key={align}>
          <PopoverTrigger asChild>
            <Button variant="ghost" size="sm">{align}</Button>
          </PopoverTrigger>
          <PopoverContent align={align} className="w-44">
            Aligned {align}.
          </PopoverContent>
        </Popover>
      ))}
    </div>
  ),
};
