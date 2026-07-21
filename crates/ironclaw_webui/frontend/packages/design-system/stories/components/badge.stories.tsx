import type { Meta, StoryObj } from "@storybook/react-vite";
import { Badge } from "../../src/badge";

const meta = {
  title: "Components/Badge",
  component: Badge,
  parameters: {
    docs: {
      description: {
        component:
          "Labelled chip with a status dot, set in the pixel tag face. Tones follow " +
          "`STATUS_CANON` (tokens.ts): success/positive/signal are green and get the " +
          "breathing live-dot; info is the running/active blue; warning and danger map " +
          "to their semantic tokens; muted is the resting state. `StatusPill` is a " +
          "backwards-compat alias.",
      },
    },
  },
  argTypes: {
    tone: {
      control: "select",
      options: ["success", "info", "warning", "danger", "accent", "muted"],
    },
    size: { control: "select", options: ["sm", "md"] },
    dot: { control: "boolean" },
    label: { control: "text" },
  },
  args: { tone: "success", label: "Running", size: "md", dot: true },
} satisfies Meta<typeof Badge>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const Tones: Story = {
  render: () => (
    <div className="flex flex-wrap items-center gap-3">
      <Badge tone="success" label="Success" />
      <Badge tone="info" label="Running" />
      <Badge tone="warning" label="Degraded" />
      <Badge tone="danger" label="Failed" />
      <Badge tone="accent" label="Beta" />
      <Badge tone="muted" label="Paused" />
    </div>
  ),
};

export const Sizes: Story = {
  render: () => (
    <div className="flex flex-wrap items-center gap-3">
      <Badge tone="info" size="sm" label="sm" />
      <Badge tone="info" size="md" label="md" />
      <Badge tone="muted" size="md" dot={false} label="No dot" />
    </div>
  ),
};
