import type { Meta, StoryObj } from "@storybook/react-vite";
import { EmptyPanel } from "../src/composites/empty-panel";
import { Button } from "../src/components/button";

const meta: Meta = { title: "Composites/EmptyPanel" };
export default meta;

type Story = StoryObj;

export const Default: Story = {
  render: () => (
    <div className="w-[32rem]">
      <EmptyPanel
        title="Pick a file"
        description="Select a file from the tree to preview its contents."
      >
        <Button variant="secondary" size="sm">Refresh</Button>
      </EmptyPanel>
    </div>
  ),
};

export const Plain: Story = {
  render: () => (
    <div className="w-[32rem]">
      <EmptyPanel
        variant="plain"
        title="No results"
        description="Adjust the filters to see more runs."
      />
    </div>
  ),
};

export const Dashed: Story = {
  render: () => (
    <div className="w-[24rem]">
      <EmptyPanel
        variant="dashed"
        description="No missions yet. Promote a thread to get started."
      />
    </div>
  ),
};
