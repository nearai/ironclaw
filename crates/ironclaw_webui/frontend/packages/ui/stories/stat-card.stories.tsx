import type { Meta, StoryObj } from "@storybook/react-vite";
import { StatCard } from "../src/composites/stat-card";

const meta: Meta = { title: "Composites/StatCard" };
export default meta;

type Story = StoryObj;

export const Default: Story = {
  render: () => (
    <div className="grid w-[28rem]">
      <StatCard label="Active runs" value={12} tone="success" badgeLabel="live" showDivider={false} />
      <StatCard label="Failures (24h)" value={3} tone="danger" badgeLabel="failing" detail="Retry from the runs tab." />
      <StatCard label="Last deploy" value="Jul 26" tone="muted" badgeLabel="idle" valueClassName="text-[1.2rem]" />
    </div>
  ),
};
