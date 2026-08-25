import type { Meta, StoryObj } from "@storybook/react-vite";

import { Button } from "./button";
import { Card, CardBody, CardFooter, CardHeader, CardLabel } from "./card";

const meta = {
  title: "Primitives/Card",
  component: Card,
  // `children` is a required Card prop; supplying it here lets the render-only
  // stories below omit `args`. Each story's explicit JSX children still win.
  args: { variant: "default", radius: "md", padding: "md", children: "Card content" },
  tags: ["ai-generated"],
} satisfies Meta<typeof Card>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  render: (args) => (
    <Card {...args} className="max-w-md">
      <CardLabel>Workspace</CardLabel>
      <p className="mt-2 text-sm text-[var(--v2-text)]">
        A solid panel surface backed by theme tokens, so it adapts to light and dark automatically.
      </p>
    </Card>
  ),
};

export const Variants: Story = {
  render: () => (
    <div className="grid max-w-3xl gap-4 sm:grid-cols-2">
      {(["default", "bordered", "subtle", "inset"] as const).map((variant) => (
        <Card key={variant} variant={variant} padding="md">
          <CardLabel>{variant}</CardLabel>
          <p className="mt-2 text-sm text-[var(--v2-text)]">Card variant: {variant}</p>
        </Card>
      ))}
    </div>
  ),
};

export const Composed: Story = {
  render: () => (
    <Card className="max-w-md" padding="none">
      <CardHeader divider>
        <CardLabel>Billing</CardLabel>
        <h3 className="mt-1 text-base font-semibold text-[var(--v2-text-strong)]">Current plan</h3>
      </CardHeader>
      <CardBody>
        <p className="text-sm text-[var(--v2-text)]">
          You are on the Team plan. Manage seats and usage from the settings page.
        </p>
      </CardBody>
      <CardFooter>
        <Button variant="secondary" size="sm">Manage plan</Button>
      </CardFooter>
    </Card>
  ),
};
