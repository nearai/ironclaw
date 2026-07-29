import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { ToggleGroup, ToggleGroupItem } from "../../src/extras/toggle-group";

const meta: Meta = { title: "Extras/ToggleGroup" };
export default meta;

type Story = StoryObj;

function SingleDemo() {
  const [value, setValue] = React.useState("list");
  return (
    <ToggleGroup
      type="single"
      value={value}
      onValueChange={(next) => next && setValue(next)}
      aria-label="View"
    >
      <ToggleGroupItem value="list" aria-label="List view">List</ToggleGroupItem>
      <ToggleGroupItem value="grid" aria-label="Grid view">Grid</ToggleGroupItem>
      <ToggleGroupItem value="board" aria-label="Board view">Board</ToggleGroupItem>
    </ToggleGroup>
  );
}

export const Single: Story = { render: () => <SingleDemo /> };

export const Multiple: Story = {
  render: () => (
    <ToggleGroup type="multiple" defaultValue={["bold"]} aria-label="Formatting">
      <ToggleGroupItem value="bold" aria-label="Bold" className="font-semibold">B</ToggleGroupItem>
      <ToggleGroupItem value="italic" aria-label="Italic" className="italic">I</ToggleGroupItem>
      <ToggleGroupItem value="underline" aria-label="Underline" className="underline">U</ToggleGroupItem>
    </ToggleGroup>
  ),
};

export const Disabled: Story = {
  render: () => (
    <ToggleGroup type="single" defaultValue="list" aria-label="View (disabled)">
      <ToggleGroupItem value="list" aria-label="List view">List</ToggleGroupItem>
      <ToggleGroupItem value="grid" aria-label="Grid view" disabled>Grid</ToggleGroupItem>
      <ToggleGroupItem value="board" aria-label="Board view">Board</ToggleGroupItem>
    </ToggleGroup>
  ),
};
