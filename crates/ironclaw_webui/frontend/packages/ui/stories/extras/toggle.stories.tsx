import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Toggle } from "../../src/extras/toggle";
import { Icon } from "../../src/icons/icon";

const meta: Meta = { title: "Extras/Toggle" };
export default meta;

type Story = StoryObj;

function Demo() {
  const [pressed, setPressed] = React.useState(false);
  return (
    <Toggle pressed={pressed} onPressedChange={setPressed} aria-label="Pin thread">
      <Icon name="pin" className="h-4 w-4" />
      {pressed ? "Pinned" : "Pin"}
    </Toggle>
  );
}

export const Default: Story = { render: () => <Demo /> };

export const Sizes: Story = {
  render: () => (
    <div className="flex items-center gap-2">
      <Toggle size="sm" aria-label="Small">sm</Toggle>
      <Toggle size="md" aria-label="Medium">md</Toggle>
      <Toggle size="lg" aria-label="Large">lg</Toggle>
      <Toggle disabled aria-label="Disabled">disabled</Toggle>
      <Toggle defaultPressed aria-label="Pressed">pressed</Toggle>
    </div>
  ),
};
