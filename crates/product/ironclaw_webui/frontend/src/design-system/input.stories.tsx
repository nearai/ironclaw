import type { Meta, StoryObj } from "@storybook/react-vite";

import { FormField, Input, Select, Textarea } from "./input";

const meta = {
  title: "Primitives/Input",
  component: Input,
  args: { placeholder: "you@example.com", size: "md" },
  tags: ["ai-generated"],
} satisfies Meta<typeof Input>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};
export const WithValue: Story = { args: { defaultValue: "ada@ironclaw.dev" } };
export const Error: Story = { args: { error: true, defaultValue: "not-an-email" } };
export const Disabled: Story = { args: { disabled: true, placeholder: "Disabled" } };

export const Sizes: Story = {
  render: (args) => (
    <div className="flex max-w-sm flex-col gap-3">
      <Input {...args} size="sm" placeholder="Small" />
      <Input {...args} size="md" placeholder="Medium" />
      <Input {...args} size="lg" placeholder="Large" />
    </div>
  ),
};

export const TextareaField: Story = {
  render: () => (
    <div className="max-w-sm">
      <Textarea placeholder="Add a note…" rows={4} />
    </div>
  ),
};

export const SelectField: Story = {
  render: () => (
    <div className="max-w-sm">
      <Select aria-label="Model provider" defaultValue="anthropic">
        <option value="anthropic">Anthropic</option>
        <option value="openai">OpenAI</option>
        <option value="ollama">Ollama</option>
      </Select>
    </div>
  ),
};

export const LabelledField: Story = {
  render: () => (
    <div className="max-w-sm">
      <FormField label="Workspace name" htmlFor="workspace" required hint="Shown to your team.">
        <Input id="workspace" placeholder="Research workspace" />
      </FormField>
    </div>
  ),
};

export const FieldWithError: Story = {
  render: () => (
    <div className="max-w-sm">
      <FormField label="API token" htmlFor="token" error="This token has expired.">
        <Input id="token" error defaultValue="sk-expired" />
      </FormField>
    </div>
  ),
};
