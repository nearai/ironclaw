import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { RadioGroup, RadioGroupItem } from "../../src/extras/radio-group";
import { Label } from "../../src/components/input";

const meta: Meta = { title: "Extras/RadioGroup" };
export default meta;

type Story = StoryObj;

function Demo() {
  const [value, setValue] = React.useState("balanced");
  const options = [
    { value: "fast", label: "Fast — lower quality" },
    { value: "balanced", label: "Balanced" },
    { value: "thorough", label: "Thorough — slower" },
  ];
  return (
    <RadioGroup value={value} onValueChange={setValue} aria-label="Run mode">
      {options.map((option) => (
        <div key={option.value} className="flex items-center gap-2">
          <RadioGroupItem value={option.value} id={`mode-${option.value}`} />
          <Label htmlFor={`mode-${option.value}`} className="cursor-pointer font-normal">
            {option.label}
          </Label>
        </div>
      ))}
    </RadioGroup>
  );
}

export const Default: Story = { render: () => <Demo /> };

export const Disabled: Story = {
  render: () => (
    <RadioGroup defaultValue="a" disabled aria-label="Disabled group">
      <div className="flex items-center gap-2">
        <RadioGroupItem value="a" id="dis-a" />
        <Label htmlFor="dis-a" className="font-normal opacity-60">Selected, disabled</Label>
      </div>
      <div className="flex items-center gap-2">
        <RadioGroupItem value="b" id="dis-b" />
        <Label htmlFor="dis-b" className="font-normal opacity-60">Disabled</Label>
      </div>
    </RadioGroup>
  ),
};
