import { useState } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Avatar, AvatarFallback } from "../../src/avatar";
import { Badge } from "../../src/badge";
import { Button } from "../../src/button";
import { Card, CardBody, CardFooter, CardHeader, CardLabel } from "../../src/card";
import { Checkbox } from "../../src/checkbox";
import { Icon } from "../../src/icons";
import { FormField, Input, Label } from "../../src/input";
import { StatCard } from "../../src/primitives";
import { SelectMenu } from "../../src/select-menu";
import { Separator } from "../../src/separator";
import { Switch } from "../../src/switch";
import { Tabs } from "../../src/tabs";

/**
 * Applied compositions — real product surfaces built purely from
 * design-system components. The point is to see the primitives inside
 * app-like contexts (a chat thread, a full page with navigation, a run
 * detail, an activity feed), not just isolated fragments. If a
 * composition needs a class the system doesn't provide, that's a gap in
 * the system — not a license for one-off styling.
 *
 * Copy in these stories follows the Brand principles page: the agent is
 * proactive (it sets routines up and shows its work), receipts read as
 * receipts, and the steering verbs are Review / Adjust / Pause / Undo.
 */
const meta = {
  title: "Compositions/Touchpoints",
  parameters: {
    layout: "padded",
    docs: {
      description: {
        component:
          "Real product surfaces assembled exclusively from design-system " +
          "components: the agent chat thread, a full automations page with " +
          "navigation, a run detail panel, the activity feed of agent " +
          "receipts, a settings panel, a marketing hero fragment, and the " +
          "onboarding routine card. Copy follows the Brand principles page " +
          "(proactive agent, receipts, steering verbs).",
      },
    },
  },
} satisfies Meta;

export default meta;
type Story = StoryObj<typeof meta>;

/* ── Shared bits ──────────────────────────────────────────────────── */

const mono = "font-mono text-xs text-[var(--v2-text-faint)]";

function AgentAvatar() {
  return (
    <Avatar className="h-7 w-7">
      <AvatarFallback className="text-[var(--v2-accent-text)]">
        <Icon name="spark" className="h-3.5 w-3.5" />
      </AvatarFallback>
    </Avatar>
  );
}

/* ── 1 · Workspace — chat thread ──────────────────────────────────── */
/* The core surface: agent narrates what it set up (a receipt card in the
   thread), the user steers. Composer at the bottom like the real app. */

export const ChatThread: Story = {
  name: "Workspace · Chat thread",
  render: function ChatThreadStory() {
    const [draft, setDraft] = useState("");
    return (
      <Card padding="none" className="flex h-[34rem] w-[42rem] flex-col">
        <CardHeader divider className="!py-3">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2.5">
              <AgentAvatar />
              <div>
                <div className="text-sm font-medium text-[var(--v2-text-strong)]">IronClaw</div>
                <div className="text-xs text-[var(--v2-text-faint)]">Connected · Gmail, Calendar, GitHub</div>
              </div>
            </div>
            <Badge tone="success" label="Online" size="sm" />
          </div>
        </CardHeader>

        <div className="flex-1 space-y-4 overflow-y-auto px-5 py-4 md:px-7">
          {/* Agent message */}
          <div className="flex gap-3">
            <AgentAvatar />
            <div className="max-w-[85%] text-sm leading-6 text-[var(--v2-text-muted)]">
              Morning. While you slept I went through the inbox — 34 newsletter
              threads and 12 recurring status emails were burying your real
              mail, so I set up a routine for them.
            </div>
          </div>

          {/* Inline receipt card — the agent acted; the user steers */}
          <div className="pl-10">
            <Card variant="subtle" radius="sm" padding="none" className="max-w-[85%]">
              <CardHeader className="!py-3">
                <div className="flex items-center justify-between gap-3">
                  <div className="flex items-center gap-2">
                    <Icon name="bolt" className="h-4 w-4 text-[var(--v2-accent-text)]" />
                    <span className="text-sm font-medium text-[var(--v2-text-strong)]">
                      Morning digest
                    </span>
                  </div>
                  <Badge tone="success" label="Scheduled" size="sm" />
                </div>
              </CardHeader>
              <CardBody className="!py-0 text-xs leading-5 text-[var(--v2-text-muted)]">
                Summarizes newsletters and status emails into one message.
                First run tomorrow · <span className="font-mono">8:00am</span>.
              </CardBody>
              <CardFooter divider={false} className="!pt-3 !pb-3">
                <div className="flex gap-2">
                  <Button variant="secondary" size="sm">Adjust</Button>
                  <Button variant="ghost" size="sm">Pause</Button>
                  <Button variant="ghost" size="sm">Undo</Button>
                </div>
              </CardFooter>
            </Card>
          </div>

          {/* User message */}
          <div className="flex justify-end">
            <div className="max-w-[75%] rounded-[var(--v2-radius-lg)] bg-[var(--v2-surface-muted)] px-4 py-2.5 text-sm leading-6 text-[var(--v2-text-strong)]">
              Move it to 7:30 and include the GitHub release notes too.
            </div>
          </div>

          {/* Agent confirms — past tense, exact numbers */}
          <div className="flex gap-3">
            <AgentAvatar />
            <div className="max-w-[85%] text-sm leading-6 text-[var(--v2-text-muted)]">
              Done — moved to <span className="font-mono text-xs">7:30am</span> and
              watching 3 repos for releases. You'll see the first digest tomorrow.
            </div>
          </div>
        </div>

        <CardFooter divider className="!py-3">
          <div className="flex items-center gap-2">
            <Button variant="ghost" size="icon-sm" aria-label="Attach a file">
              <Icon name="attach" className="h-4 w-4" />
            </Button>
            <Input
              placeholder="Tell your agent what to take on…"
              className="flex-1"
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
            />
            <Button size="icon-sm" aria-label="Send" disabled={!draft.trim()}>
              <Icon name="send" className="h-4 w-4" />
            </Button>
          </div>
        </CardFooter>
      </Card>
    );
  },
};

/* ── 2 · Automations — full page with navigation ──────────────────── */
/* The same table as the fragment below, but inside the app shell:
   sidebar nav, page header, stat strip. What the screen actually is. */

const NAV_ITEMS = [
  { icon: "chat", label: "Chat" },
  { icon: "bolt", label: "Automations", active: true, count: 8 },
  { icon: "logs", label: "Runs" },
  { icon: "plug", label: "Connections", count: 3 },
  { icon: "settings", label: "Settings" },
] as const;

const AUTOMATION_ROWS = [
  { name: "Morning digest", schedule: "Weekdays · 8:00am", status: "success", label: "Success" },
  { name: "Inbox triage", schedule: "Continuous", status: "info", label: "Running" },
  { name: "Invoice chaser", schedule: "Mondays · 9:00am", status: "muted", label: "Paused" },
  { name: "Standup notes", schedule: "Weekdays · 9:30am", status: "danger", label: "Failed" },
] as const;

export const AutomationsPage: Story = {
  name: "Automations · Full page",
  render: function AutomationsPageStory() {
    const [status, setStatus] = useState("all");
    return (
      <Card padding="none" className="flex h-[36rem] w-[64rem] overflow-hidden">
        {/* Sidebar */}
        <div className="flex w-56 shrink-0 flex-col border-r border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]">
          <div className="flex items-center gap-2 px-4 py-4">
            <Icon name="spark" className="h-4.5 w-4.5 text-[var(--v2-accent-text)]" />
            <span className="text-sm font-medium text-[var(--v2-text-strong)]">IronClaw</span>
          </div>
          <nav className="grid gap-0.5 px-2">
            {NAV_ITEMS.map((item) => (
              <button
                key={item.label}
                type="button"
                className={
                  "flex items-center gap-2.5 rounded-[var(--v2-radius-sm)] px-2.5 py-1.5 text-left text-sm " +
                  ("active" in item && item.active
                    ? "bg-[var(--v2-surface-muted)] font-medium text-[var(--v2-text-strong)]"
                    : "text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]")
                }
              >
                <Icon name={item.icon} className="h-4 w-4 shrink-0" />
                <span className="flex-1">{item.label}</span>
                {"count" in item && item.count != null && (
                  <span className={mono}>{item.count}</span>
                )}
              </button>
            ))}
          </nav>
          <div className="mt-auto border-t border-[var(--v2-panel-border)] px-4 py-3">
            <div className="flex items-center gap-2.5">
              <Avatar className="h-7 w-7">
                <AvatarFallback>MK</AvatarFallback>
              </Avatar>
              <div className="min-w-0">
                <div className="truncate text-xs font-medium text-[var(--v2-text-strong)]">mira@acme.dev</div>
                <div className="text-[0.6875rem] text-[var(--v2-text-faint)]">Pro plan</div>
              </div>
            </div>
          </div>
        </div>

        {/* Main column */}
        <div className="flex min-w-0 flex-1 flex-col overflow-y-auto">
          <div className="flex items-center justify-between border-b border-[var(--v2-panel-border)] px-6 py-4">
            <div>
              <h2 className="text-lg font-medium text-[var(--v2-text-strong)]">Automations</h2>
              <p className="text-xs text-[var(--v2-text-faint)]">
                Set up by your agent from your tools — adjust or undo any of them.
              </p>
            </div>
            <div className="flex items-center gap-2">
              <SelectMenu
                prefix="Status"
                value={status}
                onChange={setStatus}
                options={[
                  { value: "all", label: "All" },
                  { value: "running", label: "Running", tone: "info" },
                  { value: "failed", label: "Failed", tone: "danger" },
                ]}
              />
              <Button size="sm">
                <Icon name="plus" className="mr-1.5 h-3.5 w-3.5" />
                New
              </Button>
            </div>
          </div>

          <div className="grid grid-cols-3 gap-4 px-6 pt-4">
            <StatCard label="Active" value="8" tone="success" badgeLabel="Healthy" />
            <StatCard label="Runs today" value="128" tone="info" badgeLabel="Running" />
            <StatCard label="Failures" value="1" tone="danger" badgeLabel="Attention" />
          </div>

          <div className="px-6 pb-6 pt-2">
            <Card variant="flat" padding="none">
              {AUTOMATION_ROWS.map((row) => (
                <div
                  key={row.name}
                  className="flex items-center gap-4 border-b border-[var(--v2-panel-border)] px-5 py-3 last:border-b-0 hover:bg-[var(--v2-surface-soft)]"
                >
                  <Checkbox aria-label={`Select ${row.name}`} />
                  <div className="min-w-0 flex-1">
                    <div className="text-sm font-medium text-[var(--v2-text-strong)]">{row.name}</div>
                    <div className="text-xs text-[var(--v2-text-faint)]">{row.schedule}</div>
                  </div>
                  <Badge tone={row.status} label={row.label} />
                  <Button variant="ghost" size="icon-sm" aria-label={`More actions for ${row.name}`}>
                    <Icon name="more" className="h-4 w-4" />
                  </Button>
                </div>
              ))}
            </Card>
          </div>
        </div>
      </Card>
    );
  },
};

/* ── 3 · Runs — run detail ────────────────────────────────────────── */
/* A single run opened from the runs list: step-by-step receipt with
   exact durations, the failure stated plainly, and the fix offered. */

const RUN_STEPS = [
  { icon: "search", title: "Fetched 46 threads from Gmail", duration: "1.2s", status: "success", label: "Done" },
  { icon: "layers", title: "Grouped 34 newsletters, 12 status emails", duration: "0.4s", status: "success", label: "Done" },
  { icon: "edit", title: "Drafted digest summary", duration: "6.8s", status: "success", label: "Done" },
  { icon: "send", title: "Deliver to Slack #personal", duration: "0.2s", status: "danger", label: "Failed" },
] as const;

export const RunDetail: Story = {
  name: "Runs · Run detail",
  render: () => (
    <Card padding="none" className="w-[38rem]">
      <CardHeader divider>
        <div className="flex items-start justify-between gap-3">
          <div>
            <CardLabel>Run detail</CardLabel>
            <h3 className="mt-1 text-lg font-medium text-[var(--v2-text-strong)]">Morning digest</h3>
            <div className={"mt-1 " + mono}>run_9f3k2 · today · 8:00:04am · 8.6s total</div>
          </div>
          <Badge tone="danger" label="Failed" />
        </div>
      </CardHeader>
      <CardBody className="!py-0">
        {RUN_STEPS.map((step, i) => (
          <div
            key={step.title}
            className={
              "flex items-center gap-3 py-3" +
              (i < RUN_STEPS.length - 1 ? " border-b border-[var(--v2-panel-border)]" : "")
            }
          >
            <span
              className={
                "grid h-7 w-7 shrink-0 place-items-center rounded-[var(--v2-radius-sm)] border border-[var(--v2-panel-border)] " +
                (step.status === "danger"
                  ? "text-[var(--v2-danger-text)]"
                  : "text-[var(--v2-text-muted)]")
              }
            >
              <Icon name={step.icon} className="h-3.5 w-3.5" />
            </span>
            <div className="min-w-0 flex-1 text-sm text-[var(--v2-text-strong)]">{step.title}</div>
            <span className={mono}>{step.duration}</span>
            <Badge tone={step.status} label={step.label} size="sm" />
          </div>
        ))}
        <Card variant="inset" radius="sm" padding="sm" className="mb-5 mt-2">
          <div className="font-mono text-xs leading-5 text-[var(--v2-text-muted)]">
            SlackDeliveryError: token for #personal has expired.
            <br />
            Generate a new token in Settings → Connections, then retry.
          </div>
        </Card>
      </CardBody>
      <CardFooter divider>
        <div className="flex items-center justify-between">
          <Button variant="ghost" size="sm">
            <Icon name="logs" className="mr-1.5 h-3.5 w-3.5" />
            View full log
          </Button>
          <div className="flex gap-2">
            <Button variant="secondary" size="sm">Open connections</Button>
            <Button size="sm">
              <Icon name="retry" className="mr-1.5 h-3.5 w-3.5" />
              Retry run
            </Button>
          </div>
        </div>
      </CardFooter>
    </Card>
  ),
};

/* ── 4 · Activity — agent receipts feed ───────────────────────────── */
/* The Trust principle wearing UI: everything the agent did on its own,
   past tense + reason + escape hatch. */

const ACTIVITY = [
  {
    icon: "bolt",
    time: "7:30am",
    text: "Sent your morning digest — 34 newsletters, 3 GitHub releases summarized.",
    action: "View",
  },
  {
    icon: "folder",
    time: "7:26am",
    text: "Archived 12 newsletters — they matched your triage rules.",
    action: "Undo",
  },
  {
    icon: "calendar",
    time: "6:15am",
    text: "Blocked 45 minutes before your 10:00 board call for prep — your notes doc is linked in the invite.",
    action: "Adjust",
  },
  {
    icon: "shield",
    time: "2:40am",
    text: "Blocked an outbound request to an unlisted endpoint from the scraper tool.",
    action: "Review",
  },
] as const;

export const ActivityFeed: Story = {
  name: "Activity · Agent receipts",
  render: () => (
    <Card padding="none" className="w-[32rem]">
      <CardHeader divider>
        <div className="flex items-center justify-between">
          <div>
            <CardLabel>While you were away</CardLabel>
            <h3 className="mt-1 font-medium text-[var(--v2-text-strong)]">4 actions overnight</h3>
          </div>
          <Badge tone="success" label="All reversible" size="sm" />
        </div>
      </CardHeader>
      <CardBody className="!py-1">
        {ACTIVITY.map((item, i) => (
          <div
            key={item.text}
            className={
              "flex items-start gap-3 py-3.5" +
              (i < ACTIVITY.length - 1 ? " border-b border-[var(--v2-panel-border)]" : "")
            }
          >
            <span className="mt-0.5 grid h-7 w-7 shrink-0 place-items-center rounded-full border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)]">
              <Icon name={item.icon} className="h-3.5 w-3.5" />
            </span>
            <div className="min-w-0 flex-1">
              <p className="text-sm leading-6 text-[var(--v2-text-strong)]">{item.text}</p>
              <span className={mono}>{item.time}</span>
            </div>
            <Button variant="ghost" size="sm" className="shrink-0">
              {item.action}
            </Button>
          </div>
        ))}
      </CardBody>
      <CardFooter divider>
        <div className="flex items-center justify-between">
          <span className="text-xs text-[var(--v2-text-faint)]">
            Every action is logged and reversible.
          </span>
          <Button variant="ghost" size="sm">Open full activity log</Button>
        </div>
      </CardFooter>
    </Card>
  ),
};

/* ── 5 · Agent workspace — settings panel ─────────────────────────── */

export const WorkspaceSettings: Story = {
  name: "Workspace · Settings panel",
  render: function WorkspaceSettingsStory() {
    const [tab, setTab] = useState("agent");
    const [autoRoutines, setAutoRoutines] = useState(true);
    const [askIrreversible, setAskIrreversible] = useState(true);
    return (
      <Card padding="none" className="w-[36rem]">
        <CardHeader divider>
          <CardLabel>Settings</CardLabel>
          <h3 className="mt-1 text-lg font-medium text-[var(--v2-text-strong)]">
            Agent behavior
          </h3>
        </CardHeader>
        <CardBody>
          <Tabs
            ariaLabel="Settings sections"
            value={tab}
            onChange={setTab}
            bordered
            tabs={[
              { value: "agent", label: "Agent" },
              { value: "channels", label: "Channels", count: 3 },
              { value: "skills", label: "Skills", count: 12 },
              { value: "billing", label: "Billing" },
            ]}
          />
          <div className="mt-5 grid gap-5">
            <FormField label="Agent name" hint="Shown in chat and notifications.">
              <Input defaultValue="IronClaw" />
            </FormField>
            <div className="flex items-center justify-between">
              <div>
                <Label htmlFor="auto-routines" className="!mb-0">Set up routines automatically</Label>
                <p className="mt-0.5 text-xs text-[var(--v2-text-faint)]">
                  The agent derives routines from your tools and schedules them.
                  Every one arrives with Adjust, Pause, and Undo.
                </p>
              </div>
              <Switch
                id="auto-routines"
                checked={autoRoutines}
                onCheckedChange={setAutoRoutines}
              />
            </div>
            <div className="flex items-center justify-between">
              <div>
                <Label htmlFor="ask-irreversible" className="!mb-0">Ask before irreversible actions</Label>
                <p className="mt-0.5 text-xs text-[var(--v2-text-faint)]">
                  Money, first-time sends as you, and new data shares always ask first.
                </p>
              </div>
              <Switch
                id="ask-irreversible"
                checked={askIrreversible}
                onCheckedChange={setAskIrreversible}
              />
            </div>
          </div>
        </CardBody>
        <CardFooter divider>
          <div className="flex justify-end gap-2">
            <Button variant="ghost" size="sm">Reset</Button>
            <Button size="sm">Save changes</Button>
          </div>
        </CardFooter>
      </Card>
    );
  },
};

/* ── 6 · Automations — table card (fragment) ──────────────────────── */

export const AutomationsTable: Story = {
  name: "Automations · Table card",
  render: function AutomationsTableStory() {
    const [status, setStatus] = useState("all");
    return (
      <div className="w-[44rem]">
        <div className="mb-4 grid grid-cols-3 gap-4">
          <StatCard label="Active" value="8" tone="success" badgeLabel="Healthy" />
          <StatCard label="Runs today" value="128" tone="info" badgeLabel="Running" />
          <StatCard label="Failures" value="1" tone="danger" badgeLabel="Attention" />
        </div>
        <Card variant="flat" padding="none">
          <CardHeader divider className="!py-3">
            <div className="flex items-center justify-between gap-3">
              <h3 className="font-medium text-[var(--v2-text-strong)]">Automations</h3>
              <div className="flex items-center gap-2">
                <SelectMenu
                  prefix="Status"
                  value={status}
                  onChange={setStatus}
                  options={[
                    { value: "all", label: "All" },
                    { value: "running", label: "Running", tone: "info" },
                    { value: "failed", label: "Failed", tone: "danger" },
                  ]}
                />
                <Button size="sm">
                  <Icon name="plus" className="mr-1.5 h-3.5 w-3.5" />
                  New
                </Button>
              </div>
            </div>
          </CardHeader>
          <div>
            {AUTOMATION_ROWS.map((row) => (
              <div
                key={row.name}
                className="flex items-center gap-4 border-b border-[var(--v2-panel-border)] px-5 py-3 last:border-b-0 hover:bg-[var(--v2-surface-soft)]"
              >
                <Checkbox aria-label={`Select ${row.name}`} />
                <div className="min-w-0 flex-1">
                  <div className="text-sm font-medium text-[var(--v2-text-strong)]">{row.name}</div>
                  <div className="text-xs text-[var(--v2-text-faint)]">{row.schedule}</div>
                </div>
                <Badge tone={row.status} label={row.label} />
                <Button variant="ghost" size="icon-sm" aria-label={`More actions for ${row.name}`}>
                  <Icon name="more" className="h-4 w-4" />
                </Button>
              </div>
            ))}
          </div>
        </Card>
      </div>
    );
  },
};

/* ── 7 · Marketing — hero fragment ────────────────────────────────── */

export const MarketingHero: Story = {
  name: "Marketing · Hero fragment",
  render: () => (
    <div className="flex w-[40rem] flex-col items-center py-10 text-center">
      <Badge tone="accent" label="Private by design" />
      <h1 className="mt-5 text-[length:var(--v2-font-size-display)] font-medium leading-tight tracking-[var(--v2-tracking-display)] text-[var(--v2-text-strong)]">
        An agent that runs
        <br />
        your routine work.
      </h1>
      <p className="mt-4 max-w-md text-[length:var(--v2-font-size-body-lg)] text-[var(--v2-text-muted)]">
        Connect your tools once. IronClaw finds the routine work, sets it up,
        and shows every action — reversible, adjustable, yours.
      </p>
      <div className="mt-7 flex w-full max-w-md items-center gap-2">
        <Input placeholder="What should IronClaw take off your plate?" className="flex-1" size="lg" />
        <Button size="lg">Start</Button>
      </div>
      <div className="mt-6 flex items-center gap-3 text-xs text-[var(--v2-text-faint)]">
        <span>Gmail</span>
        <Separator orientation="vertical" className="!h-3" />
        <span>Calendar</span>
        <Separator orientation="vertical" className="!h-3" />
        <span>Slack</span>
        <Separator orientation="vertical" className="!h-3" />
        <span>GitHub</span>
      </div>
    </div>
  ),
};

/* ── 8 · Onboarding — routine card ────────────────────────────────── */
/* First-session magic moment: the agent already set the routine up.
   The card is a receipt with steering verbs, not a permission prompt. */

export const OnboardingRoutine: Story = {
  name: "Onboarding · Routine card",
  render: () => (
    <Card className="w-[26rem]" padding="none">
      <CardHeader>
        <div className="flex items-start justify-between gap-3">
          <div className="flex items-center gap-2.5">
            <span className="grid h-9 w-9 place-items-center rounded-[var(--v2-radius-md)] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]">
              <Icon name="spark" className="h-4.5 w-4.5 text-[var(--v2-accent-text)]" />
            </span>
            <div>
              <CardLabel>Set up for you</CardLabel>
              <h3 className="mt-0.5 font-medium text-[var(--v2-text-strong)]">
                Morning digest
              </h3>
            </div>
          </div>
          <Badge tone="success" label="Scheduled" size="sm" />
        </div>
      </CardHeader>
      <CardBody className="!pt-0 text-sm leading-6 text-[var(--v2-text-muted)]">
        I found 34 newsletter threads and 12 recurring status emails burying
        your real mail, so I set up a morning digest. The first one arrives
        tomorrow at 8:00am — adjust the schedule or undo anytime.
      </CardBody>
      <CardFooter divider>
        <div className="flex items-center justify-between">
          <Button variant="ghost" size="sm">View reasoning</Button>
          <div className="flex gap-2">
            <Button variant="ghost" size="sm">Undo</Button>
            <Button variant="secondary" size="sm">Adjust schedule</Button>
            <Button size="sm">
              <Icon name="check" className="mr-1.5 h-3.5 w-3.5" />
              Keep it
            </Button>
          </div>
        </div>
      </CardFooter>
    </Card>
  ),
};
