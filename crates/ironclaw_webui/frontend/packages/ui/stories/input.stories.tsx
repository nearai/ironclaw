import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Input, Label, Select, Textarea } from "../src/components/input";
import { FormField } from "../src/composites/form-field";

const meta: Meta<typeof Input> = {
  title: "Components/Input",
  component: Input,
  args: { placeholder: "Type here…" },
  argTypes: {
    size: { control: "select", options: ["sm", "md", "lg"] },
  },
};
export default meta;

type Story = StoryObj<typeof Input>;

export const Default: Story = { render: (args) => <div className="w-80"><Input {...args} /></div> };
export const Error: Story = {
  render: (args) => <div className="w-80"><Input {...args} error /></div>,
};
export const Sizes: Story = {
  render: () => (
    <div className="grid w-80 gap-3">
      <Input size="sm" placeholder="Small" />
      <Input size="md" placeholder="Medium" />
      <Input size="lg" placeholder="Large" />
    </div>
  ),
};

export const TextareaStory: Story = {
  name: "Textarea",
  render: () => <div className="w-80"><Textarea placeholder="Longer content…" /></div>,
};

export const SelectStory: Story = {
  name: "Select",
  render: () => (
    <div className="w-80">
      <Select defaultValue="b">
        <option value="a">Option A</option>
        <option value="b">Option B</option>
      </Select>
    </div>
  ),
};

export const WithFormField: Story = {
  render: () => (
    <div className="grid w-80 gap-5">
      <FormField label="Display name" hint="Shown in the sidebar." htmlFor="sb-name" required>
        <Input id="sb-name" placeholder="Ada" />
      </FormField>
      <FormField label="API key" error="This key is invalid." htmlFor="sb-key">
        <Input id="sb-key" error defaultValue="sk-…" />
      </FormField>
      <Label>Standalone label</Label>
    </div>
  ),
};
