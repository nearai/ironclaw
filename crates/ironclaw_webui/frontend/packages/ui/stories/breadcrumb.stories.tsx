import type { Meta, StoryObj } from "@storybook/react-vite";
import { Breadcrumb } from "../src/components/breadcrumb";

const meta: Meta = { title: "Components/Breadcrumb" };
export default meta;

type Story = StoryObj;

export const Default: Story = {
  render: () => (
    <Breadcrumb
      label="Workspace"
      items={[
        { label: "workspace", onSelect: () => {} },
        { label: "home", onSelect: () => {} },
        { label: "reports", onSelect: () => {} },
        { label: "2026-q3-summary.md", onSelect: () => {} },
      ]}
    />
  ),
};
