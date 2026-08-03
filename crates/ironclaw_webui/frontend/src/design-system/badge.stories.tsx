import type { Meta, StoryObj } from "@storybook/react-vite";

import { Badge } from "./badge";

const meta = {
  title: "Primitives/Badge",
  component: Badge,
  args: { label: "Active", tone: "muted", size: "md", dot: true },
  tags: ["ai-generated"],
} satisfies Meta<typeof Badge>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Muted: Story = { args: { tone: "muted", label: "Draft" } };
export const Success: Story = { args: { tone: "success", label: "Live" } };
export const Warning: Story = { args: { tone: "warning", label: "Pending" } };
export const Danger: Story = { args: { tone: "danger", label: "Failed" } };
export const Info: Story = { args: { tone: "info", label: "Info" } };
export const Accent: Story = { args: { tone: "accent", label: "Beta" } };

export const WithoutDot: Story = { args: { tone: "accent", label: "No dot", dot: false } };
export const Small: Story = { args: { tone: "success", label: "Sm", size: "sm" } };

export const AllTones: Story = {
  render: () => (
    <div className="flex flex-wrap items-center gap-2">
      <Badge tone="success" label="Success" />
      <Badge tone="warning" label="Warning" />
      <Badge tone="danger" label="Danger" />
      <Badge tone="info" label="Info" />
      <Badge tone="accent" label="Accent" />
      <Badge tone="muted" label="Muted" />
    </div>
  ),
};
