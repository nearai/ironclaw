import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import {
  NavigationMenu,
  NavigationMenuContent,
  NavigationMenuItem,
  NavigationMenuLink,
  NavigationMenuList,
  NavigationMenuTrigger,
} from "../../src/extras/navigation-menu";

const meta: Meta = { title: "Extras/NavigationMenu" };
export default meta;

type Story = StoryObj;

function PanelLink({ title, description }: { title: string; description: string }) {
  return (
    <NavigationMenuLink
      href="#"
      className="flex w-56 flex-col items-start gap-0.5 rounded-[8px] px-3 py-2.5"
    >
      <span className="text-ui font-medium text-[var(--v2-text-strong)]">{title}</span>
      <span className="text-ui-sm font-normal text-[var(--v2-text-muted)]">{description}</span>
    </NavigationMenuLink>
  );
}

export const Default: Story = {
  render: () => (
    <div className="h-64">
      <NavigationMenu>
        <NavigationMenuList>
          <NavigationMenuItem>
            <NavigationMenuTrigger>Product</NavigationMenuTrigger>
            <NavigationMenuContent>
              <div className="flex flex-col gap-1">
                <PanelLink title="Agents" description="Autonomous run orchestration" />
                <PanelLink title="Tools" description="Extend agents with skills" />
              </div>
            </NavigationMenuContent>
          </NavigationMenuItem>
          <NavigationMenuItem>
            <NavigationMenuTrigger>Resources</NavigationMenuTrigger>
            <NavigationMenuContent>
              <div className="flex flex-col gap-1">
                <PanelLink title="Guides" description="Step-by-step walkthroughs" />
                <PanelLink title="Changelog" description="What shipped recently" />
              </div>
            </NavigationMenuContent>
          </NavigationMenuItem>
          <NavigationMenuItem>
            <NavigationMenuLink href="#">Docs</NavigationMenuLink>
          </NavigationMenuItem>
        </NavigationMenuList>
      </NavigationMenu>
    </div>
  ),
};
