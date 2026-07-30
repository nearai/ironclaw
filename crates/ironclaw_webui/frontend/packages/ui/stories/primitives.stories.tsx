import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Spinner } from "../src/primitives/spinner";
import { Skeleton } from "../src/primitives/skeleton";
import { StatusDot } from "../src/primitives/status-dot";

const meta: Meta = { title: "Primitives/Overview" };
export default meta;

export const SpinnerStory: StoryObj = {
  name: "Spinner",
  render: () => (
    <div className="flex items-center gap-4 text-[var(--v2-text)]">
      <Spinner />
      <Spinner className="h-6 w-6 text-[var(--v2-accent)]" />
      <Spinner className="h-8 w-8 text-[var(--v2-positive-text)]" />
    </div>
  ),
};

export const SkeletonStory: StoryObj = {
  name: "Skeleton",
  render: () => (
    <div className="grid w-80 gap-3">
      <Skeleton className="h-8" />
      <Skeleton className="h-16" />
      <Skeleton className="h-[120px]" />
    </div>
  ),
};

export const StatusDotStory: StoryObj = {
  name: "StatusDot",
  render: () => (
    <div className="grid gap-3 text-sm text-[var(--v2-text)]">
      <div className="flex items-center gap-2">
        <StatusDot tone="success" pulse /> Connected
      </div>
      <div className="flex items-center gap-2">
        <StatusDot tone="warning" size="md" /> Degraded
      </div>
      <div className="flex items-center gap-2">
        <StatusDot tone="danger" /> Offline
      </div>
      <div className="flex items-center gap-2 text-[var(--v2-accent-text)]">
        <StatusDot /> Inherits current color
      </div>
    </div>
  ),
};
