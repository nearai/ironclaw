import type { Meta, StoryObj } from "@storybook/react-vite";
import { Heading, Text } from "../../src/text";

const meta = {
  title: "Components/Primitives/Text & Heading",
  component: Text,
  parameters: {
    docs: {
      description: {
        component:
          "The typography primitives: one place where font-size / weight / " +
          "color combinations are decided, so pages stop re-deriving " +
          "`text-xs text-[var(--v2-text-muted)]` by hand. Variants map 1:1 " +
          "onto the TYPE_TOKENS scale (Tokens/Reference); tones map onto the " +
          "semantic text colors. `Heading` renders h1–h6 on the same scale.",
      },
    },
  },
  argTypes: {
    variant: {
      control: "select",
      options: [
        "display",
        "display-sm",
        "heading",
        "title",
        "body-lg",
        "body",
        "body-sm",
        "caption",
        "eyebrow",
        "label",
        "mono",
      ],
    },
    tone: {
      control: "select",
      options: [
        "default",
        "strong",
        "muted",
        "faint",
        "accent",
        "positive",
        "warning",
        "danger",
        "info",
        "inherit",
      ],
    },
    weight: { control: "select", options: ["inherit", "normal", "medium", "semibold"] },
  },
  args: { variant: "body", tone: "default", children: "Every run is scoped to your project." },
} satisfies Meta<typeof Text>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Playground: Story = {};

const SCALE: Array<{ variant: any; sample: string; note: string }> = [
  { variant: "display", sample: "Automations", note: "36px — page h1" },
  { variant: "display-sm", sample: "1,284", note: "28px — stat values" },
  { variant: "heading", sample: "Recent activity", note: "24px — section headings" },
  { variant: "title", sample: "Configure extension", note: "20px — modal/panel titles" },
  { variant: "body-lg", sample: "Connect a channel to start routing messages.", note: "16px — descriptions" },
  { variant: "body", sample: "Every run is scoped to your project.", note: "14px — body copy" },
  { variant: "body-sm", sample: "Every run is scoped to your project.", note: "13px — controls + mobile body" },
  { variant: "caption", sample: "Last synced 2 minutes ago", note: "12px — hints, meta" },
  { variant: "eyebrow", sample: "Trace commons", note: "11px — section eyebrow" },
  { variant: "label", sample: "Running", note: "11px — pixel tag face" },
  { variant: "mono", sample: "run_9f3k2 · 8.6s", note: "12px mono — data" },
];

export const Scale: Story = {
  render: () => (
    <div className="flex flex-col gap-4">
      {SCALE.map(({ variant, sample, note }) => (
        <div key={variant} className="grid grid-cols-[8rem_1fr_auto] items-baseline gap-4">
          <Text variant="mono" tone="faint">{variant}</Text>
          <Text variant={variant}>{sample}</Text>
          <Text variant="caption" tone="muted">{note}</Text>
        </div>
      ))}
    </div>
  ),
};

export const Tones: Story = {
  render: () => (
    <div className="flex flex-col gap-2">
      {(["strong", "default", "muted", "faint", "accent", "positive", "warning", "danger", "info"] as const).map(
        (tone) => (
          <Text key={tone} variant="body" tone={tone}>
            {tone} — the agent narrates what it did and why.
          </Text>
        )
      )}
    </div>
  ),
};

export const Headings: Story = {
  render: () => (
    <div className="flex flex-col gap-3">
      <Heading level={1}>Level 1 — display</Heading>
      <Heading level={2}>Level 2 — heading</Heading>
      <Heading level={3}>Level 3 — title</Heading>
      <Heading level={4}>Level 4 — body-lg</Heading>
    </div>
  ),
};
