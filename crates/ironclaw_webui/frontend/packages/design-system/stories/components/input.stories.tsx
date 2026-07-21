import type { Meta, StoryObj } from "@storybook/react-vite";
import { FormField, Input, Label, Select, Textarea } from "../../src/input";

const meta = {
  title: "Components/Input",
  component: Input,
  parameters: {
    docs: {
      description: {
        component:
          "Form controls on the shared control-density scale. The focus ring lands " +
          "instantly (never transitioned — keyboard traversal must feel immediate); " +
          "hover gets a fast border tint with no movement. `FormField` composes " +
          "Label + control + error/hint. `Select` here is the styled native element — " +
          "use `SelectMenu` for the Radix popover select.",
      },
    },
  },
  argTypes: {
    size: { control: "select", options: ["sm", "md", "lg"] },
    error: { control: "boolean" },
    disabled: { control: "boolean" },
    placeholder: { control: "text" },
  },
  args: { placeholder: "workspace-name", size: "md" },
} satisfies Meta<typeof Input>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  render: (args) => <Input {...args} className="w-72" />,
};

export const AllControls: Story = {
  render: () => (
    <div className="grid w-80 gap-4">
      <FormField label="Workspace name" hint="Shown in the sidebar and page titles.">
        <Input placeholder="Acme Ops" />
      </FormField>
      <FormField label="API token" error="This token has expired.">
        <Input error placeholder="icw_..." />
      </FormField>
      <FormField label="Region">
        <Select defaultValue="us-west">
          <option value="us-west">US West</option>
          <option value="us-east">US East</option>
          <option value="eu">Europe</option>
        </Select>
      </FormField>
      <div>
        <Label htmlFor="notes" required>Notes</Label>
        <Textarea id="notes" rows={3} placeholder="Anything the agent should know…" className="mt-1.5" />
      </div>
      <Input disabled placeholder="Disabled" />
    </div>
  ),
};
