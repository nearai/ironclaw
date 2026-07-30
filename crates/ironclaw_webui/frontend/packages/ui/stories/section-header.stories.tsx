import type { Meta, StoryObj } from "@storybook/react-vite";
import { SectionHeader, SubLabel } from "../src/composites/section-header";
import { Button } from "../src/components/button";
import { Card } from "../src/components/card";
import { SegmentedControl } from "../src/composites/segmented-control";

const meta: Meta = { title: "Composites/SectionHeader" };
export default meta;

type Story = StoryObj;

export const Default: Story = {
  render: () => (
    <div className="w-[44rem]">
      <SectionHeader
        eyebrow="Automations"
        title="Recurring runs"
        description="Saved schedules and event handlers, with their run history."
      />
    </div>
  ),
};

export const WithActions: Story = {
  render: () => (
    <Card padding="sm" className="w-[44rem] sm:p-5">
      <SectionHeader
        eyebrow="Explorer"
        title="Job queue"
        description="Search by title or ID, jump into a run, and stop active work."
        actions={
          <>
            <SegmentedControl
              label="Filter"
              value="all"
              onChange={() => {}}
              options={[
                { value: "all", label: "All" },
                { value: "active", label: "Active" },
                { value: "failed", label: "Failed" },
              ]}
            />
            <Button variant="secondary" size="sm">Refresh</Button>
          </>
        }
      />
    </Card>
  ),
};

export const SubLabelStory: Story = {
  name: "SubLabel",
  render: () => (<SubLabel>Delivery defaults</SubLabel>),
};
