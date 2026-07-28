import React from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Icon } from "../src/primitives/icon";
import { Spinner } from "../src/primitives/spinner";
import { Skeleton } from "../src/primitives/skeleton";

const ICON_NAMES = [
  "attach", "bolt", "bell", "bookOpen", "calendar", "check", "chat", "close",
  "clock", "download", "edit", "file", "flag", "pin", "pause", "play",
  "folder", "layers", "list", "logs", "lock", "logout", "moon", "plug",
  "plus", "pulse", "send", "search", "settings", "spark", "sun", "shield",
  "tool", "terminal", "trash", "upload", "chevron", "more", "copy",
  "arrowDown", "retry",
];

const meta: Meta = { title: "Primitives/Overview" };
export default meta;

export const Icons: StoryObj = {
  render: () => (
    <div className="grid grid-cols-6 gap-3">
      {ICON_NAMES.map((name) => (
        <div
          key={name}
          className="flex flex-col items-center gap-1.5 rounded-[10px] border border-[var(--v2-panel-border)] p-3"
        >
          <Icon name={name} className="h-5 w-5 text-[var(--v2-text)]" />
          <span className="font-mono text-[10px] text-[var(--v2-text-faint)]">{name}</span>
        </div>
      ))}
    </div>
  ),
};

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
