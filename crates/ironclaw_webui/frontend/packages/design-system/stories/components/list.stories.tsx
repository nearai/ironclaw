import type { Meta, StoryObj } from "@storybook/react-vite";
import { Badge } from "../../src/badge";
import { Button } from "../../src/button";
import { Card } from "../../src/card";
import { Checkbox } from "../../src/checkbox";
import { Icon } from "../../src/icons";
import { ListRow } from "../../src/list";

const meta = {
  title: "Components/ListRow",
  component: ListRow,
  parameters: {
    docs: {
      description: {
        component:
          "The one row shape for tables, feeds, run steps, and pickers. Slot-based: " +
          "`leading` (checkbox, icon chip, avatar), `title`, `description`, `meta` (mono), " +
          "`trailing` (badge, actions). Passing `onClick` turns the whole row into an " +
          "accessible button; rows divide themselves and the last row drops its divider. " +
          "Every list surface in the Compositions section is built from this component.",
      },
    },
  },
  argTypes: {
    size: { control: "select", options: ["sm", "md"] },
    divider: { control: "boolean" },
  },
  args: {
    title: "Morning digest",
    description: "Summarizes newsletters and status emails into one message.",
    divider: false,
  },
} satisfies Meta<typeof ListRow>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const AutomationList: Story = {
  render: () => (
    <Card variant="flat" padding="none" className="w-[36rem]">
      <ListRow
        leading={<Checkbox aria-label="Select Morning digest" />}
        title="Morning digest"
        description="Every weekday at 8:00am · Gmail, GitHub"
        trailing={<Badge tone="success" label="Active" size="sm" />}
        onClick={() => {}}
      />
      <ListRow
        leading={<Checkbox aria-label="Select PR review queue" />}
        title="PR review queue"
        description="On new pull request · GitHub"
        trailing={<Badge tone="info" label="Running" size="sm" />}
        onClick={() => {}}
      />
      <ListRow
        leading={<Checkbox aria-label="Select Invoice chaser" />}
        title="Invoice chaser"
        description="1st of the month · Stripe, Gmail"
        trailing={<Badge tone="muted" label="Paused" size="sm" />}
        onClick={() => {}}
      />
    </Card>
  ),
};

export const ActivityReceipts: Story = {
  render: () => (
    <Card variant="flat" padding="none" className="w-[36rem]">
      <ListRow
        leading={
          <span className="grid h-8 w-8 place-items-center rounded-[var(--v2-radius-sm)] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]">
            <Icon name="check" className="h-4 w-4 text-[var(--v2-positive-text)]" />
          </span>
        }
        title="Archived 12 newsletters"
        description="They matched your triage rules."
        meta="9:02am"
        trailing={
          <Button variant="ghost" size="sm">
            Undo
          </Button>
        }
      />
      <ListRow
        leading={
          <span className="grid h-8 w-8 place-items-center rounded-[var(--v2-radius-sm)] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]">
            <Icon name="bolt" className="h-4 w-4 text-[var(--v2-accent-text)]" />
          </span>
        }
        title='Set up "Release notes watch"'
        description="3 repos publish releases you keep opening manually."
        meta="8:47am"
        trailing={
          <Button variant="ghost" size="sm">
            Adjust
          </Button>
        }
      />
    </Card>
  ),
};
