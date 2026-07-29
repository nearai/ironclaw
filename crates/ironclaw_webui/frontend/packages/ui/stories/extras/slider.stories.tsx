import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Slider } from "../../src/extras/slider";

const meta: Meta = { title: "Extras/Slider" };
export default meta;

type Story = StoryObj;

function Labeled() {
  const [value, setValue] = React.useState([40]);
  return (
    <div className="flex w-72 flex-col gap-2">
      <div className="flex justify-between text-ui-sm text-[var(--v2-text-muted)]">
        <span>Concurrency</span>
        <span>{value[0]}</span>
      </div>
      <Slider value={value} onValueChange={setValue} max={100} step={5} aria-label="Concurrency" />
    </div>
  );
}

export const Default: Story = { render: () => <Labeled /> };

export const Range: Story = {
  render: () => (
    <Slider
      defaultValue={[20, 70]}
      max={100}
      step={1}
      className="w-72"
      aria-label="Budget range"
    />
  ),
};

export const Disabled: Story = {
  render: () => (
    <Slider defaultValue={[60]} disabled className="w-72" aria-label="Disabled" />
  ),
};

export const Vertical: Story = {
  render: () => (
    <Slider
      defaultValue={[30]}
      orientation="vertical"
      max={100}
      aria-label="Volume"
    />
  ),
};
