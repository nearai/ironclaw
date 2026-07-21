import type { Meta, StoryObj } from "@storybook/react-vite";
import {
  EmptyPanel,
  FlowList,
  SectionHeader,
  StatCard,
  SubLabel,
} from "../../src/primitives";
import { Button } from "../../src/button";

const meta = {
  title: "Components/Primitives",
  parameters: {
    layout: "padded",
    docs: {
      description: {
        component:
          "Higher-level composites built from Card, Badge, and Button: StatCard for " +
          "summary strips, SectionHeader for page sections, EmptyPanel for zero " +
          "states, FlowList for step sequences, SubLabel for mono-caps eyebrows.",
      },
    },
  },
} satisfies Meta;

export default meta;
type Story = StoryObj<typeof meta>;

export const StatCards: Story = {
  render: () => (
    <div className="grid w-[44rem] grid-cols-3 gap-4">
      <StatCard label="Runs today" value="128" tone="success" badgeLabel="Healthy" detail="+12% vs yesterday" />
      <StatCard label="Needs approval" value="4" tone="warning" badgeLabel="Waiting" detail="Oldest: 2h ago" />
      <StatCard label="Failures" value="1" tone="danger" badgeLabel="Attention" detail="Retry scheduled" />
    </div>
  ),
};

export const SectionHeaderStory: Story = {
  name: "SectionHeader",
  render: () => (
    <div className="w-[36rem]">
      <SectionHeader
        title="Recent activity"
        subtitle="Everything the agent did in the last 24 hours."
      />
    </div>
  ),
};

export const EmptyPanelStory: Story = {
  name: "EmptyPanel",
  render: () => (
    <div className="w-[32rem]">
      <EmptyPanel
        title="No automations yet"
        description="Connect a tool and the agent will start suggesting automations based on what it sees."
      >
        <Button size="sm">Connect Google</Button>
      </EmptyPanel>
    </div>
  ),
};

export const FlowListStory: Story = {
  name: "FlowList",
  render: () => (
    <div className="w-[28rem]">
      <SubLabel className="mb-3">How it works</SubLabel>
      <FlowList
        items={[
          {
            title: "Connect",
            description: "One sign-in links your tools; the agent reads your world.",
          },
          {
            title: "Review suggestions",
            description: "The agent drafts suggest-only automations from what it sees.",
          },
          {
            title: "Approve",
            description: "Nothing runs until you approve it — every run stays scoped.",
          },
        ]}
      />
    </div>
  ),
};
