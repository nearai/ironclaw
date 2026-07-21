import type { Meta, StoryObj } from "@storybook/react-vite";
import { Card, CardBody, CardFooter, CardHeader, CardLabel } from "../../src/card";
import { Button } from "../../src/button";
import { Badge } from "../../src/badge";

const meta = {
  title: "Components/Card",
  component: Card,
  parameters: {
    layout: "padded",
    docs: {
      description: {
        component:
          "The panel surface. Elevation is deliberately restrained: a hairline border " +
          "does the separation work and `--v2-card-shadow` only lifts the surface 1px. " +
          "Use `flat` for in-page cards (tables, stat grids) that should sit flush on " +
          "the canvas. Compose freely with CardHeader / CardBody / CardFooter / CardLabel.",
      },
    },
  },
  argTypes: {
    variant: {
      control: "select",
      options: ["default", "bordered", "flat", "subtle", "inset"],
    },
    radius: { control: "select", options: ["sm", "md", "lg"] },
    padding: { control: "select", options: ["none", "sm", "md", "lg"] },
  },
  args: { variant: "default", radius: "md", padding: "md" },
} satisfies Meta<typeof Card>;

export default meta;
type Story = StoryObj;

export const Default: Story = {
  render: (args) => (
    <Card {...args} className="w-96">
      <CardLabel>Workspace</CardLabel>
      <h3 className="mt-2 text-lg font-semibold text-[var(--v2-text-strong)]">
        Inbox triage
      </h3>
      <p className="mt-1 text-sm text-[var(--v2-text-muted)]">
        Drafts replies for routine email and flags anything that needs you.
      </p>
    </Card>
  ),
};

export const Variants: Story = {
  render: () => (
    <div className="grid w-[40rem] grid-cols-2 gap-4">
      {(["default", "bordered", "flat", "subtle", "inset"] as const).map((variant) => (
        <Card key={variant} variant={variant} padding="md">
          <CardLabel>{variant}</CardLabel>
          <p className="mt-2 text-sm text-[var(--v2-text-muted)]">
            Surface treatment for the {variant} variant.
          </p>
        </Card>
      ))}
    </div>
  ),
};

export const Composed: Story = {
  render: () => (
    <Card padding="none" className="w-[28rem]">
      <CardHeader divider>
        <div className="flex items-center justify-between">
          <div>
            <CardLabel>Automation</CardLabel>
            <h3 className="mt-1 font-semibold text-[var(--v2-text-strong)]">
              Morning digest
            </h3>
          </div>
          <Badge tone="success" label="Active" />
        </div>
      </CardHeader>
      <CardBody className="text-sm text-[var(--v2-text-muted)]">
        Summarizes overnight activity across your connected tools every weekday
        at 8:00am.
      </CardBody>
      <CardFooter divider>
        <div className="flex justify-end gap-2">
          <Button variant="ghost" size="sm">Pause</Button>
          <Button variant="secondary" size="sm">Edit schedule</Button>
        </div>
      </CardFooter>
    </Card>
  ),
};
