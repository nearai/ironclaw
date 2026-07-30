import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { StatStrip, StatTile } from "../src/composites/stat-strip";
import { StatCard } from "../src/composites/stat-card";

const meta: Meta = { title: "Composites/StatStrip" };
export default meta;

type Story = StoryObj;

export const Default: Story = {
  render: () => (
    <div className="w-[56rem]">
      <StatStrip columns={3}>
        <StatTile label="Scheduled" value={6} tone="muted" badgeLabel="idle" detail="Visible to this agent." />
        <StatTile label="Active" value={5} tone="success" badgeLabel="live" detail="Waiting for their next run." />
        <StatTile label="Failures" value={1} tone="danger" badgeLabel="failing" detail="Failed in recent history." />
      </StatStrip>
    </div>
  ),
};

function FilterableDemo() {
  const [filter, setFilter] = React.useState("all");
  return (
    <div className="w-[56rem]">
      <StatStrip columns={3}>
        <StatTile
          label="All"
          value={9}
          tone="muted"
          badgeLabel="idle"
          detail="Every tracked run."
          onSelect={() => setFilter("all")}
          isActive={filter === "all"}
          selectTitle="Show all"
        />
        <StatTile
          label="Running"
          value={2}
          tone="info"
          badgeLabel="info"
          detail="In progress right now."
          onSelect={() => setFilter("running")}
          isActive={filter === "running"}
          selectTitle="Show running"
        />
        <StatTile
          label="Next run"
          value="Jul 29, 06:06 PM"
          tone="info"
          badgeLabel="info"
          detail="Soonest scheduled run."
          valueClassName="text-lg md:text-xl"
        />
      </StatStrip>
    </div>
  );
}

export const Filterable: Story = { render: () => <FilterableDemo /> };
