import type { Meta, StoryObj } from "@storybook/react-vite";
import { Tooltip, TooltipProvider } from "../../src/tooltip";
import { Button } from "../../src/button";
import { Icon } from "../../src/icons";

const meta = {
  title: "Components/Tooltip",
  component: Tooltip,
  decorators: [
    (Story) => (
      <TooltipProvider>
        <Story />
      </TooltipProvider>
    ),
  ],
  parameters: {
    docs: {
      description: {
        component:
          "`@radix-ui/react-tooltip` with menu elevation: opens on hover *and* " +
          "keyboard focus, positions itself, and never traps the pointer. Wrap the " +
          "app (or story) in `TooltipProvider` once to share the delay.",
      },
    },
  },
  argTypes: {
    content: { control: "text" },
    side: { control: "select", options: ["top", "right", "bottom", "left"] },
    align: { control: "select", options: ["start", "center", "end"] },
  },
  args: { content: "Run this automation now", side: "top", align: "center" },
} satisfies Meta<typeof Tooltip>;

export default meta;

type TooltipArgs = {
  content?: React.ReactNode;
  side?: "top" | "right" | "bottom" | "left";
  align?: "start" | "center" | "end";
};

export const Default: StoryObj = {
  render: (args: TooltipArgs) => (
    <div className="p-16">
      <Tooltip content={args.content} side={args.side} align={args.align}>
        <Button variant="secondary" size="icon" aria-label="Run now">
          <Icon name="play" className="h-4 w-4" />
        </Button>
      </Tooltip>
    </div>
  ),
};

export const Placements: StoryObj = {
  render: () => (
    <div className="flex items-center gap-3 p-12">
      <Tooltip content="Run this automation now">
        <Button variant="secondary" size="icon" aria-label="Run now">
          <Icon name="play" className="h-4 w-4" />
        </Button>
      </Tooltip>
      <Tooltip content="Download run logs" side="bottom">
        <Button variant="ghost" size="icon" aria-label="Download logs">
          <Icon name="download" className="h-4 w-4" />
        </Button>
      </Tooltip>
      <Tooltip content="Delete" side="right">
        <Button variant="ghost" size="icon" aria-label="Delete">
          <Icon name="trash" className="h-4 w-4" />
        </Button>
      </Tooltip>
    </div>
  ),
};
