import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import {
  Menubar,
  MenubarCheckboxItem,
  MenubarContent,
  MenubarItem,
  MenubarMenu,
  MenubarRadioGroup,
  MenubarRadioItem,
  MenubarSeparator,
  MenubarShortcut,
  MenubarSub,
  MenubarSubContent,
  MenubarSubTrigger,
  MenubarTrigger,
} from "../../src/extras/menubar";

const meta: Meta = { title: "Extras/Menubar" };
export default meta;

type Story = StoryObj;

function Demo() {
  const [wordWrap, setWordWrap] = React.useState(true);
  const [theme, setTheme] = React.useState("dark");
  return (
    <Menubar>
      <MenubarMenu>
        <MenubarTrigger>File</MenubarTrigger>
        <MenubarContent>
          <MenubarItem>
            New run<MenubarShortcut>⌘N</MenubarShortcut>
          </MenubarItem>
          <MenubarItem>Open…</MenubarItem>
          <MenubarItem disabled>Revert (no changes)</MenubarItem>
          <MenubarSeparator />
          <MenubarSub>
            <MenubarSubTrigger>Export</MenubarSubTrigger>
            <MenubarSubContent>
              <MenubarItem>JSON</MenubarItem>
              <MenubarItem>CSV</MenubarItem>
            </MenubarSubContent>
          </MenubarSub>
        </MenubarContent>
      </MenubarMenu>
      <MenubarMenu>
        <MenubarTrigger>View</MenubarTrigger>
        <MenubarContent>
          <MenubarCheckboxItem
            checked={wordWrap}
            onCheckedChange={(next) => setWordWrap(next === true)}
          >
            Word wrap
          </MenubarCheckboxItem>
          <MenubarSeparator />
          <MenubarRadioGroup value={theme} onValueChange={setTheme}>
            <MenubarRadioItem value="light">Light</MenubarRadioItem>
            <MenubarRadioItem value="dark">Dark</MenubarRadioItem>
          </MenubarRadioGroup>
        </MenubarContent>
      </MenubarMenu>
      <MenubarMenu>
        <MenubarTrigger>Help</MenubarTrigger>
        <MenubarContent>
          <MenubarItem>Documentation</MenubarItem>
        </MenubarContent>
      </MenubarMenu>
    </Menubar>
  );
}

export const Default: Story = { render: () => <Demo /> };
