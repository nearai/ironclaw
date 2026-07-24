import type { Meta, StoryObj } from "@storybook/react-vite";
import { Badge } from "../../src/badge";
import { Card } from "../../src/card";
import { ListRow } from "../../src/list";
import { ToolIcon } from "../../src/tool-icon";

const meta = {
  title: "Components/ToolIcon",
  component: ToolIcon,
  parameters: {
    docs: {
      description: {
        component:
          "The chip that identifies a tool, service, or project. Known tools resolve to a " +
          "glyph from the system icon set; unknown ones fall back to a pixel-face monogram " +
          "so a new integration never renders blank. Pass `icon` to force a glyph. Designed " +
          "for the `leading` slot of ListRow in connection lists, feeds, and run steps.",
      },
    },
  },
  argTypes: {
    size: { control: "select", options: ["sm", "md", "lg"] },
    shape: { control: "select", options: ["square", "circle"] },
  },
  args: { name: "GitHub", size: "md", shape: "square" },
} satisfies Meta<typeof ToolIcon>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const KnownAndFallback: Story = {
  render: () => (
    <div className="flex items-center gap-2">
      <ToolIcon name="Gmail" />
      <ToolIcon name="Calendar" />
      <ToolIcon name="GitHub" />
      <ToolIcon name="Slack" />
      <ToolIcon name="Terminal" />
      <ToolIcon name="Stripe" />
      <ToolIcon name="Linear" />
      <ToolIcon name="Notion" />
    </div>
  ),
};

export const InAConnectionList: Story = {
  render: () => (
    <Card variant="flat" padding="none" className="w-72">
      <ListRow
        size="sm"
        leading={<ToolIcon name="Gmail" size="sm" />}
        title={<span className="text-xs">Gmail</span>}
        trailing={<Badge tone="success" label="OK" size="sm" />}
        onClick={() => {}}
      />
      <ListRow
        size="sm"
        leading={<ToolIcon name="Calendar" size="sm" />}
        title={<span className="text-xs">Calendar</span>}
        trailing={<Badge tone="success" label="OK" size="sm" />}
        onClick={() => {}}
      />
      <ListRow
        size="sm"
        leading={<ToolIcon name="GitHub" size="sm" />}
        title={<span className="text-xs">GitHub</span>}
        trailing={<Badge tone="warning" label="Token" size="sm" />}
        onClick={() => {}}
      />
    </Card>
  ),
};
