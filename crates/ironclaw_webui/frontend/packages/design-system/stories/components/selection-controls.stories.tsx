import { useState } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Checkbox } from "../../src/checkbox";
import { Switch } from "../../src/switch";
import { RadioGroup, RadioGroupItem } from "../../src/radio-group";
import { Slider } from "../../src/slider";
import { Label } from "../../src/input";

const meta = {
  title: "Components/Selection controls",
  parameters: {
    docs: {
      description: {
        component:
          "Checkbox, Switch, RadioGroup, and Slider — each a thin IronClaw skin over " +
          "its Radix primitive, so keyboard operation (Space/arrows), form " +
          "integration, and aria states come from the primitive. All share the accent " +
          "checked state and the fast, non-moving focus ring.",
      },
    },
  },
} satisfies Meta;

export default meta;
type Story = StoryObj<typeof meta>;

export const CheckboxStory: Story = {
  name: "Checkbox",
  render: () => (
    <div className="grid gap-3">
      <label className="flex items-center gap-2.5 text-sm text-[var(--v2-text)]">
        <Checkbox defaultChecked /> Suggest automations from my inbox
      </label>
      <label className="flex items-center gap-2.5 text-sm text-[var(--v2-text)]">
        <Checkbox /> Auto-approve routine replies
      </label>
      <label className="flex items-center gap-2.5 text-sm text-[var(--v2-text-muted)]">
        <Checkbox disabled /> Requires a connected calendar
      </label>
    </div>
  ),
};

export const SwitchStory: Story = {
  name: "Switch",
  render: function SwitchDemo() {
    const [on, setOn] = useState(true);
    return (
      <div className="flex items-center gap-3">
        <Switch id="notifications" checked={on} onCheckedChange={setOn} />
        <Label htmlFor="notifications" className="!mb-0">
          Notify me when a run needs approval
        </Label>
      </div>
    );
  },
};

export const RadioStory: Story = {
  name: "RadioGroup",
  render: () => (
    <RadioGroup defaultValue="suggest" aria-label="Automation mode">
      {[
        { value: "suggest", label: "Suggest only — I approve every run" },
        { value: "scheduled", label: "Run on schedule" },
        { value: "auto", label: "Fully automatic" },
      ].map((option) => (
        <label
          key={option.value}
          className="flex items-center gap-2.5 text-sm text-[var(--v2-text)]"
        >
          <RadioGroupItem value={option.value} />
          {option.label}
        </label>
      ))}
    </RadioGroup>
  ),
};

export const SliderStory: Story = {
  name: "Slider",
  render: () => (
    <div className="w-72">
      <Label className="mb-3">Daily run budget</Label>
      <Slider defaultValue={[40]} max={100} step={5} aria-label="Daily run budget" />
    </div>
  ),
};
