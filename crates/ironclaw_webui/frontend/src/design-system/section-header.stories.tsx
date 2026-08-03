import type { Meta, StoryObj } from "@storybook/react-vite";

import { SectionHeader } from "./primitives";

const meta = {
  title: "Composites/SectionHeader",
  component: SectionHeader,
  args: { title: "Settings", subtitle: "Manage your workspace, channels, and providers." },
  tags: ["ai-generated"],
} satisfies Meta<typeof SectionHeader>;

export default meta;
type Story = StoryObj<typeof meta>;

// Note: SectionHeader is `hidden md:block`, so it only shows at the md breakpoint and up.
export const Default: Story = {};
export const TitleOnly: Story = { args: { subtitle: undefined } };
