import type { Meta, StoryObj } from "@storybook/react-vite";
import { DetailList, DetailRow } from "../src/composites/detail-list";
import { Badge } from "../src/components/badge";
import { Card } from "../src/components/card";

const meta: Meta = { title: "Composites/DetailList" };
export default meta;

type Story = StoryObj;

export const Rows: Story = {
  render: () => (
    <Card padding="md" className="w-[28rem]">
      <DetailList>
        <DetailRow term="ID">
          <span className="font-mono text-xs">usr-9f31c2</span>
        </DetailRow>
        <DetailRow term="Email">avery@ironclaw.dev</DetailRow>
        <DetailRow term="Status">
          <Badge tone="success" label="active" size="sm" />
        </DetailRow>
        <DetailRow term="Created">3 weeks ago</DetailRow>
      </DetailList>
    </Card>
  ),
};

export const Stacked: Story = {
  render: () => (
    <Card padding="md" className="w-[28rem]">
      <DetailList className="grid grid-cols-2 gap-x-6">
        <DetailRow layout="stacked" term="Queued">Jul 29, 01:10 PM</DetailRow>
        <DetailRow layout="stacked" term="Started">Jul 29, 01:18 PM</DetailRow>
        <DetailRow layout="stacked" term="Runner">runner-2</DetailRow>
        <DetailRow layout="stacked" term="Attempts">1 of 3</DetailRow>
      </DetailList>
    </Card>
  ),
};
