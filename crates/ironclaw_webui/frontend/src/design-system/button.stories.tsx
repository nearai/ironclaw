import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect } from "storybook/test";

import { Button } from "./button";

const meta = {
  title: "Primitives/Button",
  component: Button,
  args: { children: "Continue" },
  tags: ["ai-generated"],
} satisfies Meta<typeof Button>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Primary: Story = { args: { variant: "primary", children: "Save changes" } };
export const Secondary: Story = { args: { variant: "secondary", children: "Cancel" } };
export const Outline: Story = { args: { variant: "outline", children: "Configure" } };
export const Ghost: Story = { args: { variant: "ghost", children: "Dismiss" } };
export const Danger: Story = { args: { variant: "danger", children: "Delete" } };

export const Sizes: Story = {
  render: (args) => (
    <div className="flex items-center gap-3">
      <Button {...args} size="sm">Small</Button>
      <Button {...args} size="md">Medium</Button>
      <Button {...args} size="lg">Large</Button>
    </div>
  ),
};

export const Disabled: Story = { args: { disabled: true, children: "Unavailable" } };
export const FullWidth: Story = { args: { fullWidth: true, children: "Full width" } };

export const Loading: Story = {
  args: { loading: true, children: "Connecting" },
  // Loading is a state the render alone doesn't prove: it must disable the
  // control and expose aria-busy so assistive tech announces the wait.
  play: async ({ canvas }) => {
    const button = canvas.getByRole("button", { name: /connecting/i });
    await expect(button).toBeDisabled();
    await expect(button).toHaveAttribute("aria-busy", "true");
  },
};

// The single project-wide CssCheck. `font-semibold` (weight 600) is applied to
// every Button variant via BASE; if app.css / the compiled Tailwind layer did
// not load in the preview, the computed weight would fall back to 400. A green
// assertion here is the proof that stories render with the real stylesheet.
export const CssCheck: Story = {
  args: { children: "Styled" },
  play: async ({ canvas }) => {
    const button = canvas.getByRole("button", { name: /styled/i });
    await expect(getComputedStyle(button).fontWeight).toBe("600");
  },
};
