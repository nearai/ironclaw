import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Card, CardBody, CardFooter, CardHeader, CardLabel } from "../src/components/card";
import { Button } from "../src/components/button";

const meta: Meta<typeof Card> = {
  title: "Components/Card",
  component: Card,
  argTypes: {
    variant: { control: "select", options: ["default", "bordered", "subtle", "inset"] },
    radius: { control: "select", options: ["sm", "md", "lg"] },
    padding: { control: "select", options: ["none", "sm", "md", "lg"] },
  },
};
export default meta;

type Story = StoryObj<typeof Card>;

export const Default: Story = {
  args: { padding: "md", children: "Card content", className: "w-80" },
};

export const Variants: Story = {
  render: () => (
    <div className="grid w-[28rem] gap-4">
      {(["default", "bordered", "subtle", "inset"] as const).map((variant) => (
        <Card key={variant} variant={variant} padding="md">
          <CardLabel>{variant}</CardLabel>
          <div className="mt-2 text-sm text-[var(--v2-text)]">
            Panel surface using the “{variant}” variant.
          </div>
        </Card>
      ))}
    </div>
  ),
};

export const Composed: Story = {
  render: () => (
    <Card className="w-[28rem]">
      <CardHeader divider>
        <CardLabel>Settings</CardLabel>
        <div className="mt-1 text-base font-semibold text-[var(--v2-text-strong)]">
          Workspace access
        </div>
      </CardHeader>
      <CardBody>
        <p className="text-sm text-[var(--v2-text-muted)]">
          Header, body, and footer sections compose freely with dividers.
        </p>
      </CardBody>
      <CardFooter>
        <Button variant="secondary" size="sm">Cancel</Button>
        <Button size="sm">Save</Button>
      </CardFooter>
    </Card>
  ),
};
