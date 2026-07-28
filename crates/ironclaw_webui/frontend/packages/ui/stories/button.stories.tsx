import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Button } from "../src/components/button";
import { Icon } from "../src/primitives/icon";

const meta: Meta<typeof Button> = {
  title: "Components/Button",
  component: Button,
  args: { children: "Button" },
  argTypes: {
    variant: {
      control: "select",
      options: ["primary", "outline", "secondary", "ghost", "danger"],
    },
    size: {
      control: "select",
      options: ["sm", "md", "lg", "icon", "icon-sm"],
    },
  },
};
export default meta;

type Story = StoryObj<typeof Button>;

export const Primary: Story = { args: { variant: "primary" } };
export const Outline: Story = { args: { variant: "outline" } };
export const Secondary: Story = { args: { variant: "secondary" } };
export const Ghost: Story = { args: { variant: "ghost" } };
export const Danger: Story = { args: { variant: "danger" } };
export const Loading: Story = { args: { loading: true, children: "Saving" } };
export const Disabled: Story = { args: { disabled: true } };
export const AsLink: Story = {
  args: { as: "a", href: "https://example.com", children: "Open docs" },
};

export const Sizes: Story = {
  render: () => (
    <div className="flex items-center gap-3">
      <Button size="sm">Small</Button>
      <Button size="md">Medium</Button>
      <Button size="lg">Large</Button>
      <Button size="icon" aria-label="Add">
        <Icon name="plus" className="h-4 w-4" />
      </Button>
      <Button size="icon-sm" aria-label="Add">
        <Icon name="plus" className="h-4 w-4" />
      </Button>
    </div>
  ),
};

export const AllVariants: Story = {
  render: () => (
    <div className="flex flex-col gap-3">
      {(["primary", "outline", "secondary", "ghost", "danger"] as const).map((variant) => (
        <div key={variant} className="flex items-center gap-3">
          <Button variant={variant} size="sm">{variant}</Button>
          <Button variant={variant} size="sm" loading>loading</Button>
          <Button variant={variant} size="sm" disabled>disabled</Button>
        </div>
      ))}
    </div>
  ),
};
