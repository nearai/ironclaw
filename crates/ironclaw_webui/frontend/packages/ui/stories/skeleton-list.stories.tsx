import type { Meta, StoryObj } from "@storybook/react-vite";
import { SkeletonList } from "../src/composites/skeleton-list";

const meta: Meta = { title: "Composites/SkeletonList" };
export default meta;

type Story = StoryObj;

export const Default: Story = {
  render: () => (
    <div className="w-[36rem]">
      <SkeletonList label="Loading automations" />
    </div>
  ),
};

export const CompactRows: Story = {
  render: () => (
    <div className="w-[24rem]">
      <SkeletonList count={5} itemClassName="h-10 rounded-[10px]" className="space-y-2" />
    </div>
  ),
};
