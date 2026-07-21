import type { Meta, StoryObj } from "@storybook/react-vite";
import { Icon } from "../../src/icons";

const ICON_NAMES = [
  "attach", "bolt", "bell", "bookOpen", "calendar", "check", "chat", "close",
  "clock", "download", "edit", "file", "flag", "pin", "pause", "play",
  "folder", "layers", "list", "logs", "lock", "logout", "moon", "plug",
  "plus", "pulse", "send", "search", "settings", "spark", "sun", "shield",
  "tool", "terminal", "trash", "upload", "chevron", "more", "copy",
  "arrowDown", "retry",
] as const;

const meta = {
  title: "Components/Icon",
  component: Icon,
  parameters: {
    docs: {
      description: {
        component:
          "The stroke icon set (24px viewBox, 1.7 default stroke, `currentColor`). " +
          "Icons are decorative (`aria-hidden`) — pair them with visible text or an " +
          "`aria-label` on the interactive parent.",
      },
    },
  },
  argTypes: {
    name: { control: "select", options: ICON_NAMES },
    strokeWidth: { control: { type: "range", min: 1, max: 3, step: 0.1 } },
  },
  args: { name: "spark", className: "h-6 w-6", strokeWidth: 1.7 },
} satisfies Meta<typeof Icon>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Playground: Story = {};

export const AllIcons: Story = {
  render: () => (
    <div className="grid grid-cols-6 gap-2">
      {ICON_NAMES.map((name) => (
        <div
          key={name}
          className="flex flex-col items-center gap-2 rounded-[var(--v2-radius-sm)] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] p-3"
        >
          <Icon name={name} className="h-5 w-5 text-[var(--v2-text)]" />
          <span className="text-[10px] text-[var(--v2-text-faint)]">{name}</span>
        </div>
      ))}
    </div>
  ),
};
