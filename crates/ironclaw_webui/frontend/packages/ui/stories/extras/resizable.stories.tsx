import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from "../../src/extras/resizable";

const meta: Meta = { title: "Extras/Resizable" };
export default meta;

type Story = StoryObj;

function Pane({ label }: { label: string }) {
  return (
    <div className="grid h-full w-full place-items-center text-ui text-[var(--v2-text-muted)]">
      {label}
    </div>
  );
}

export const Horizontal: Story = {
  render: () => (
    <div className="h-48 w-[28rem] overflow-hidden rounded-[12px] border border-[var(--v2-panel-border)]">
      <ResizablePanelGroup orientation="horizontal">
        <ResizablePanel defaultSize="30%" minSize="15%">
          <Pane label="Sidebar" />
        </ResizablePanel>
        <ResizableHandle withHandle />
        <ResizablePanel>
          <Pane label="Content" />
        </ResizablePanel>
      </ResizablePanelGroup>
    </div>
  ),
};

export const Vertical: Story = {
  render: () => (
    <div className="h-64 w-80 overflow-hidden rounded-[12px] border border-[var(--v2-panel-border)]">
      <ResizablePanelGroup orientation="vertical">
        <ResizablePanel defaultSize="60%">
          <Pane label="Editor" />
        </ResizablePanel>
        <ResizableHandle withHandle />
        <ResizablePanel>
          <Pane label="Terminal" />
        </ResizablePanel>
      </ResizablePanelGroup>
    </div>
  ),
};

export const Nested: Story = {
  render: () => (
    <div className="h-64 w-[28rem] overflow-hidden rounded-[12px] border border-[var(--v2-panel-border)]">
      <ResizablePanelGroup orientation="horizontal">
        <ResizablePanel defaultSize="35%">
          <Pane label="Nav" />
        </ResizablePanel>
        <ResizableHandle />
        <ResizablePanel>
          <ResizablePanelGroup orientation="vertical">
            <ResizablePanel>
              <Pane label="Main" />
            </ResizablePanel>
            <ResizableHandle />
            <ResizablePanel defaultSize="30%">
              <Pane label="Logs" />
            </ResizablePanel>
          </ResizablePanelGroup>
        </ResizablePanel>
      </ResizablePanelGroup>
    </div>
  ),
};
