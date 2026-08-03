import type { Meta, StoryObj } from "@storybook/react-vite";

import { Panel, StatCard } from "./primitives";

const meta = {
  title: "Composites/StatCard",
  component: StatCard,
  args: { label: "Active jobs", value: 128, tone: "success" },
  tags: ["ai-generated"],
} satisfies Meta<typeof StatCard>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};
export const WithDetail: Story = {
  args: { label: "Credits", value: "$42.10", tone: "accent", detail: "Renews in 12 days" },
};
export const Warning: Story = { args: { label: "Failures", value: 3, tone: "warning" } };

export const InPanel: Story = {
  render: () => (
    <Panel className="px-4">
      <StatCard label="Total runs" value={1284} tone="muted" showDivider={false} />
      <StatCard label="Succeeded" value={1201} tone="success" />
      <StatCard label="Failed" value={83} tone="danger" />
    </Panel>
  ),
};
