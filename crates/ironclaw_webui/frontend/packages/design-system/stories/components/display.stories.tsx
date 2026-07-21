import type { Meta, StoryObj } from "@storybook/react-vite";
import { Avatar, AvatarFallback, AvatarImage } from "../../src/avatar";
import { Separator } from "../../src/separator";
import { Skeleton } from "../../src/skeleton";
import { ScrollArea } from "../../src/scroll-area";
import { Spinner } from "../../src/spinner";

const meta = {
  title: "Components/Display",
  parameters: {
    docs: {
      description: {
        component:
          "Small display primitives: Avatar (Radix, with graceful image fallback), " +
          "Separator (Radix, semantic where it matters), Skeleton (loading " +
          "placeholder), ScrollArea (Radix, custom themed scrollbars), and Spinner " +
          "(the in-flight indicator, suppressed under reduced motion).",
      },
    },
  },
} satisfies Meta;

export default meta;
type Story = StoryObj<typeof meta>;

export const AvatarStory: Story = {
  name: "Avatar",
  render: () => (
    <div className="flex items-center gap-3">
      <Avatar>
        <AvatarImage src="https://github.com/nearai.png" alt="NEAR AI" />
        <AvatarFallback>NA</AvatarFallback>
      </Avatar>
      <Avatar>
        <AvatarFallback>IC</AvatarFallback>
      </Avatar>
    </div>
  ),
};

export const SeparatorStory: Story = {
  name: "Separator",
  render: () => (
    <div className="w-72 text-sm text-[var(--v2-text-muted)]">
      <p>Connected integrations</p>
      <Separator className="my-3" />
      <div className="flex h-5 items-center gap-3 text-[var(--v2-text)]">
        <span>Gmail</span>
        <Separator orientation="vertical" />
        <span>Calendar</span>
        <Separator orientation="vertical" />
        <span>Slack</span>
      </div>
    </div>
  ),
};

export const SkeletonStory: Story = {
  name: "Skeleton",
  render: () => (
    <div className="w-80 space-y-3">
      <div className="flex items-center gap-3">
        <Skeleton className="h-10 w-10 rounded-full" />
        <div className="flex-1 space-y-2">
          <Skeleton className="h-3.5 w-2/3" />
          <Skeleton className="h-3 w-1/3" />
        </div>
      </div>
      <Skeleton className="h-24 w-full" />
    </div>
  ),
};

export const ScrollAreaStory: Story = {
  name: "ScrollArea",
  render: () => (
    <ScrollArea className="h-48 w-72 rounded-[var(--v2-radius-md)] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] p-4">
      <div className="space-y-2 text-sm text-[var(--v2-text)]">
        {Array.from({ length: 24 }, (_, i) => (
          <p key={i}>Run #{2140 - i} — completed in {(4 + (i % 7) * 1.3).toFixed(1)}s</p>
        ))}
      </div>
    </ScrollArea>
  ),
};

export const SpinnerStory: Story = {
  name: "Spinner",
  render: () => (
    <div className="flex items-center gap-4 text-[var(--v2-text-muted)]">
      <Spinner />
      <Spinner className="h-6 w-6 text-[var(--v2-accent)]" />
    </div>
  ),
};
