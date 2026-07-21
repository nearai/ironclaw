import type { Meta, StoryObj } from "@storybook/react-vite";
import { Button } from "../../src/button";
import { Icon } from "../../src/icons";

const meta = {
  title: "Components/Button",
  component: Button,
  parameters: {
    docs: {
      description: {
        component:
          "The action control. Five variants on the compact control-density scale " +
          "(`--v2-control-h-*`: 28/32/36px). **Primary** is the brand-blue radial ramp " +
          "with the signature hover glow — one per view. **Secondary** is the workhorse. " +
          "**Ghost** for toolbars and rows, **outline** for hero/marketing moments, " +
          "**danger** for destructive actions. `loading` keeps the label visible so the " +
          "button holds its width.",
      },
    },
  },
  argTypes: {
    variant: {
      control: "select",
      options: ["primary", "outline", "secondary", "ghost", "danger"],
    },
    size: {
      control: "select",
      options: ["sm", "md", "lg", "icon", "icon-sm"],
    },
    fullWidth: { control: "boolean" },
    loading: { control: "boolean" },
    disabled: { control: "boolean" },
    children: { control: "text" },
  },
  args: {
    children: "Save changes",
    variant: "primary",
    size: "md",
  },
} satisfies Meta<typeof Button>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Primary: Story = {};

export const Variants: Story = {
  render: () => (
    <div className="flex flex-wrap items-center gap-3">
      <Button variant="primary">Primary</Button>
      <Button variant="secondary">Secondary</Button>
      <Button variant="outline">Outline</Button>
      <Button variant="ghost">Ghost</Button>
      <Button variant="danger">Delete</Button>
    </div>
  ),
};

export const Sizes: Story = {
  render: () => (
    <div className="flex flex-wrap items-center gap-3">
      <Button variant="secondary" size="sm">Small (28px)</Button>
      <Button variant="secondary" size="md">Medium (32px)</Button>
      <Button variant="secondary" size="lg">Large (36px)</Button>
      <Button variant="secondary" size="icon" aria-label="Settings">
        <Icon name="settings" className="h-4 w-4" />
      </Button>
      <Button variant="secondary" size="icon-sm" aria-label="Edit">
        <Icon name="edit" className="h-3.5 w-3.5" />
      </Button>
    </div>
  ),
};

export const States: Story = {
  render: () => (
    <div className="flex flex-wrap items-center gap-3">
      <Button loading>Saving…</Button>
      <Button disabled>Disabled</Button>
      <Button variant="secondary" loading>Refreshing</Button>
      <Button variant="danger" disabled>Delete</Button>
    </div>
  ),
};

export const WithIcon: Story = {
  render: () => (
    <div className="flex flex-wrap items-center gap-3">
      <Button>
        <Icon name="plus" className="mr-1.5 h-4 w-4" />
        New automation
      </Button>
      <Button variant="secondary">
        <Icon name="download" className="mr-1.5 h-4 w-4" />
        Export
      </Button>
    </div>
  ),
};

export const AsLink: Story = {
  args: {
    as: "a",
    href: "https://ironclaw.com",
    target: "_blank",
    children: "Open ironclaw.com",
    variant: "outline",
  },
};
