import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../../src/extras/tabs";

const meta: Meta = { title: "Extras/Tabs" };
export default meta;

type Story = StoryObj;

export const Default: Story = {
  render: () => (
    <Tabs defaultValue="overview" className="w-96">
      <TabsList>
        <TabsTrigger value="overview">Overview</TabsTrigger>
        <TabsTrigger value="logs">Logs</TabsTrigger>
        <TabsTrigger value="settings">Settings</TabsTrigger>
        <TabsTrigger value="danger" disabled>Danger</TabsTrigger>
      </TabsList>
      <TabsContent value="overview">
        High-level run metrics and current status.
      </TabsContent>
      <TabsContent value="logs">Structured log stream for the run.</TabsContent>
      <TabsContent value="settings">Per-run configuration.</TabsContent>
      <TabsContent value="danger">Never reachable.</TabsContent>
    </Tabs>
  ),
};
