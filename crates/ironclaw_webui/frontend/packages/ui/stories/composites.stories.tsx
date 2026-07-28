import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Breadcrumb } from "../src/composites/breadcrumb";
import { ConfirmDialog } from "../src/composites/confirm-dialog";
import { EmptyPanel } from "../src/composites/empty-panel";
import { FlowList } from "../src/composites/flow-list";
import { SectionHeader, SubLabel } from "../src/composites/section-header";
import { StatCard } from "../src/composites/stat-card";
import { Button } from "../src/components/button";

const meta: Meta = { title: "Composites/Overview" };
export default meta;

export const BreadcrumbStory: StoryObj = {
  name: "Breadcrumb",
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

function ConfirmDialogDemo() {
  const [open, setOpen] = React.useState(false);
  return (
    <>
      <Button variant="danger" size="sm" onClick={() => setOpen(true)}>Delete chat</Button>
      <ConfirmDialog
        open={open}
        title="Delete chat"
        description="This permanently removes the conversation."
        confirmLabel="Delete"
        onConfirm={() => setOpen(false)}
        onCancel={() => setOpen(false)}
      />
    </>
  );
}

export const ConfirmDialogStory: StoryObj = {
  name: "ConfirmDialog",
  render: () => <ConfirmDialogDemo />,
};

export const EmptyPanelStory: StoryObj = {
  name: "EmptyPanel",
  render: () => (
    <div className="w-[32rem]">
      <EmptyPanel
        title="Pick a file"
        description="Select a file from the tree to preview its contents."
      >
        <Button variant="secondary" size="sm">Refresh</Button>
      </EmptyPanel>
    </div>
  ),
};

export const StatCards: StoryObj = {
  render: () => (
    <div className="grid w-[28rem]">
      <StatCard label="Active runs" value={12} tone="success" badgeLabel="live" showDivider={false} />
      <StatCard label="Failures (24h)" value={3} tone="danger" badgeLabel="failing" detail="Retry from the runs tab." />
      <StatCard label="Last deploy" value="Jul 26" tone="muted" badgeLabel="idle" valueClassName="text-[1.2rem]" />
    </div>
  ),
};

export const FlowListStory: StoryObj = {
  name: "FlowList",
  render: () => (
    <div className="w-[28rem]">
      <FlowList
        items={[
          { title: "Connect a provider", description: "Add an LLM provider key in settings." },
          { title: "Start a chat", description: "The agent gets a workspace and tool access." },
          { title: "Automate", description: "Promote a recurring prompt into an automation." },
        ]}
      />
    </div>
  ),
};

export const Headings: StoryObj = {
  render: () => (
    <div className="grid w-[32rem] gap-4">
      <SectionHeader title="Automations" subtitle="Recurring agent runs and their history." />
      <SubLabel>Delivery defaults</SubLabel>
    </div>
  ),
};
