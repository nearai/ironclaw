import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { VerticalTabs, VerticalTabsMobile, type VerticalTabItem } from "../src/composites/vertical-tabs";

const meta: Meta = { title: "Composites/VerticalTabs" };
export default meta;

type Story = StoryObj;

const ITEMS: VerticalTabItem[] = [
  { id: "inference", label: "Inference", icon: "spark" },
  { id: "appearance", label: "Appearance", icon: "sun" },
  { id: "tools", label: "Tools", icon: "tool", count: 12 },
  { id: "skills", label: "Skills", icon: "file", count: 3 },
  { id: "language", label: "Language", icon: "globe" },
];

function DesktopDemo() {
  const [active, setActive] = React.useState("inference");
  return (
    <div className="w-60">
      <VerticalTabs items={ITEMS} activeId={active} onSelect={setActive} label="Settings sections" />
    </div>
  );
}

function MobileDemo() {
  const [active, setActive] = React.useState("inference");
  return (
    <div className="w-72">
      <VerticalTabsMobile items={ITEMS} activeId={active} onSelect={setActive} label="Settings sections" />
    </div>
  );
}

export const Default: Story = { render: () => <DesktopDemo /> };
export const Mobile: Story = { render: () => <MobileDemo /> };
