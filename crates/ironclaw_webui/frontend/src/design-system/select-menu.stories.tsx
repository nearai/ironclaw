import type { Meta, StoryObj } from "@storybook/react-vite";
import { useState } from "react";
import { expect } from "storybook/test";

import { SelectMenu } from "./select-menu";

type Option = { value: string; label?: string; disabled?: boolean; tone?: string };

const OPTIONS: Option[] = [
  { value: "anthropic", label: "Anthropic", tone: "accent" },
  { value: "openai", label: "OpenAI", tone: "positive" },
  { value: "ollama", label: "Ollama", tone: "neutral" },
  { value: "bedrock", label: "Bedrock (unavailable)", disabled: true },
];

type SelectMenuDemoProps = {
  initialValue?: string;
  disabled?: boolean;
  align?: "left" | "right";
  placeholder?: string;
};

/** SelectMenu is controlled (`value` + `onChange`); this demo owns the state. */
function SelectMenuDemo({
  initialValue = "anthropic",
  disabled = false,
  align = "right",
  placeholder = "",
}: SelectMenuDemoProps) {
  const [value, setValue] = useState(initialValue);
  return (
    <SelectMenu
      value={value}
      options={OPTIONS}
      onChange={setValue}
      disabled={disabled}
      align={align}
      placeholder={placeholder}
      aria-label="Model provider"
    />
  );
}

const meta = {
  title: "Primitives/SelectMenu",
  component: SelectMenuDemo,
  args: { initialValue: "anthropic", disabled: false, align: "right", placeholder: "" },
  tags: ["ai-generated"],
} satisfies Meta<typeof SelectMenuDemo>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};
export const Disabled: Story = { args: { disabled: true } };
export const AlignLeft: Story = { args: { align: "left" } };
export const Placeholder: Story = { args: { initialValue: "", placeholder: "Select a provider" } };

export const Selecting: Story = {
  // The listbox open/close + selection is the behavior worth proving.
  play: async ({ canvas, userEvent }) => {
    const trigger = canvas.getByRole("button", { name: /model provider/i });
    await expect(trigger).toHaveAttribute("aria-expanded", "false");
    await userEvent.click(trigger);
    await expect(trigger).toHaveAttribute("aria-expanded", "true");
    await userEvent.click(canvas.getByRole("option", { name: /openai/i }));
    await expect(trigger).toHaveTextContent(/openai/i);
  },
};
