import type { Meta, StoryObj } from "@storybook/react-vite";
import { useState } from "react";
import { NavItem, NavList } from "../../src/nav";
import { Separator } from "../../src/separator";

const meta = {
  title: "Components/Navigation",
  component: NavItem,
  parameters: {
    docs: {
      description: {
        component:
          "`NavList` + `NavItem`: the sidebar and rail vocabulary. One atomic control per " +
          "destination (icon, label, optional count, active state); the active item carries " +
          "`aria-current=\"page\"`. Renders a button by default and any link element via " +
          "`as`. App shells compose these instead of hand-rolling nav rows, so a generated " +
          "sidebar and a hand-written one are the same three props per destination.",
      },
    },
  },
  argTypes: {
    icon: { control: "text" },
    label: { control: "text" },
    count: { control: "text" },
    active: { control: "boolean" },
  },
  args: { icon: "bolt", label: "Automations", count: "8", active: true },
} satisfies Meta<typeof NavItem>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  render: (args) => (
    <div className="w-56">
      <NavList label="Example">
        <NavItem {...args} />
      </NavList>
    </div>
  ),
};

export const Sidebar: Story = {
  render: function SidebarStory() {
    const [current, setCurrent] = useState("automations");
    const items = [
      { id: "chat", icon: "chat", label: "Chat" },
      { id: "automations", icon: "bolt", label: "Automations", count: "8" },
      { id: "activity", icon: "pulse", label: "Activity", count: "3 new" },
      { id: "connections", icon: "plug", label: "Connections" },
    ];
    return (
      <div className="w-60 rounded-[var(--v2-radius-lg)] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] p-3">
        <NavList label="Workspace">
          {items.map((item) => (
            <NavItem
              key={item.id}
              icon={item.icon}
              label={item.label}
              count={item.count}
              active={current === item.id}
              onClick={() => setCurrent(item.id)}
            />
          ))}
        </NavList>
        <Separator className="my-3" />
        <NavList label="Account">
          <NavItem icon="settings" label="Settings" />
          <NavItem icon="logout" label="Sign out" />
        </NavList>
      </div>
    );
  },
};

export const AsLink: Story = {
  render: () => (
    <div className="w-56">
      <NavList label="Docs">
        <NavItem as="a" href="#tokens" icon="layers" label="Tokens" />
        <NavItem as="a" href="#components" icon="tool" label="Components" active />
      </NavList>
    </div>
  ),
};
