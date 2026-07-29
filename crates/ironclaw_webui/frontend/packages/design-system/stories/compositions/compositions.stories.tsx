import { useState } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { Avatar, AvatarFallback } from "../../src/avatar";
import { Badge } from "../../src/badge";
import { Button } from "../../src/button";
import { Card, CardBody, CardFooter, CardHeader, CardLabel } from "../../src/card";
import { AgentAvatar, ChatMessage } from "../../src/chat";
import { Checkbox } from "../../src/checkbox";
import { Icon } from "../../src/icons";
import { FormField, Input, Label } from "../../src/input";
import { ListRow } from "../../src/list";
import { NavItem, NavList } from "../../src/nav";
import { StatCard } from "../../src/primitives";
import { SelectMenu } from "../../src/select-menu";
import { Separator } from "../../src/separator";
import { Switch } from "../../src/switch";
import { Tabs } from "../../src/tabs";
import { ToolIcon } from "../../src/tool-icon";

/**
 * Applied compositions — real product surfaces built purely from
 * design-system components. The point is to see the primitives inside
 * app-like contexts (the full workspace, a chat thread, an automations
 * page, a run detail, an activity feed), not just isolated fragments.
 * If a composition needs a class the system doesn't provide, that's a
 * gap in the system — not a license for one-off styling. NavItem,
 * ListRow, ChatMessage, and Callout all exist because these stories
 * demanded them.
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
          "components: the full agent workspace (sidebar, thread, context " +
          "rail), the chat thread, a full automations page with navigation, " +
          "a run detail panel, the activity feed of agent receipts, a " +
          "settings panel, a marketing hero fragment, and the onboarding " +
          "routine card. Copy follows the Brand principles page (proactive " +
          "agent, receipts, steering verbs). These double as ground truth " +
          "for generative UI: every surface is reachable from the published " +
          "component set alone (see Docs → Generative UI).",
      },
    },
  },
} satisfies Meta;

export default meta;
type Story = StoryObj<typeof meta>;

/* ── Shared bits ──────────────────────────────────────────────────── */

const mono = "font-mono text-xs text-[var(--v2-text-faint)]";

const NAV_PRIMARY = [
  { id: "chat", icon: "chat", label: "Chat" },
  { id: "automations", icon: "bolt", label: "Automations", count: "8" },
  { id: "runs", icon: "logs", label: "Runs" },
  { id: "connections", icon: "plug", label: "Connections", count: "3" },
  { id: "settings", icon: "settings", label: "Settings" },
] as const;

function WorkspaceSidebar({ active }: { active: string }) {
  return (
    <div className="flex w-56 shrink-0 flex-col border-r border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]">
      <div className="flex items-center gap-2 px-4 py-4">
        <Icon name="spark" className="h-4.5 w-4.5 text-[var(--v2-accent-text)]" />
        <span className="text-sm font-medium text-[var(--v2-text-strong)]">IronClaw</span>
      </div>
      <NavList label="Workspace" className="px-2">
        {NAV_PRIMARY.map((item) => (
          <NavItem
            key={item.id}
            icon={item.icon}
            label={item.label}
            count={"count" in item ? item.count : undefined}
            active={item.id === active}
          />
        ))}
      </NavList>
      <div className="mt-auto border-t border-[var(--v2-panel-border)] px-4 py-3">
        <div className="flex items-center gap-2.5">
          <Avatar className="h-7 w-7">
            <AvatarFallback>MK</AvatarFallback>
          </Avatar>
          <div className="min-w-0">
            <div className="truncate text-xs font-medium text-[var(--v2-text-strong)]">
              mira@acme.dev
            </div>
            <div className="text-[0.6875rem] text-[var(--v2-text-faint)]">Pro plan</div>
          </div>
        </div>
      </div>
    </div>
  );
}

/** Inline routine receipt — the agent acted; the user steers. */
function RoutineReceiptCard() {
  return (
    <Card variant="subtle" radius="sm" padding="none">
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
        Summarizes newsletters and status emails into one message. First run
        tomorrow · <span className="font-mono">8:00am</span>.
      </CardBody>
      <CardFooter divider={false} className="!pt-3 !pb-3">
        <div className="flex gap-2">
          <Button variant="secondary" size="sm">Adjust</Button>
          <Button variant="ghost" size="sm">Pause</Button>
          <Button variant="ghost" size="sm">Undo</Button>
        </div>
      </CardFooter>
    </Card>
  );
}

function ThreadMessages() {
  return (
    <>
      <ChatMessage role="agent">
        <p>
          Morning. While you slept I went through the inbox — 34 newsletter
          threads and 12 recurring status emails were burying your real mail,
          so I set up a routine for them.
        </p>
        <RoutineReceiptCard />
      </ChatMessage>
      <ChatMessage role="user">
        Move it to 7:30 and include the GitHub release notes too.
      </ChatMessage>
      <ChatMessage role="agent">
        Done — moved to <span className="font-mono text-xs">7:30am</span> and
        watching 3 repos for releases. You'll see the first digest tomorrow.
      </ChatMessage>
    </>
  );
}

function Composer() {
  const [draft, setDraft] = useState("");
  return (
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
  );
}

/* ── 1 · Agent workspace — full page ──────────────────────────────── */
/* The whole product in one frame: sidebar navigation, the live thread,
   and the context rail (agent status, today's receipts, connections).
   Everything here is a published component; nothing is hand-rolled. */

const TODAY_RECEIPTS = [
  { icon: "bolt", text: "Sent your morning digest", time: "7:30am" },
  { icon: "folder", text: "Archived 12 newsletters", time: "7:26am" },
  { icon: "calendar", text: "Blocked prep time before your board call", time: "6:15am" },
] as const;

const CONNECTIONS = [
  { name: "Gmail", status: "success", label: "OK" },
  { name: "Calendar", status: "success", label: "OK" },
  { name: "GitHub", status: "warning", label: "Token" },
] as const;

const railTitle = "text-xs";

export const WorkspacePage: Story = {
  name: "Workspace · Full page",
  render: () => (
    <Card padding="none" className="flex h-[40rem] w-[76rem] overflow-hidden">
      <WorkspaceSidebar active="chat" />

      {/* Thread column */}
      <div className="flex min-w-0 flex-1 flex-col">
        <div className="flex items-center justify-between border-b border-[var(--v2-panel-border)] px-5 py-2.5">
          <div className="flex items-center gap-2.5">
            <AgentAvatar />
            <div>
              <div className="text-sm font-medium text-[var(--v2-text-strong)]">IronClaw</div>
              <div className="text-xs text-[var(--v2-text-faint)]">
                Connected · Gmail, Calendar, GitHub
              </div>
            </div>
          </div>
          <Badge tone="success" label="Online" size="sm" />
        </div>
        <div className="flex-1 space-y-3.5 overflow-y-auto px-5 py-3.5">
          <ThreadMessages />
        </div>
        <div className="border-t border-[var(--v2-panel-border)] px-5 py-2.5">
          <Composer />
        </div>
      </div>

      {/* Context rail */}
      <div className="flex w-72 shrink-0 flex-col gap-3 overflow-y-auto border-l border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-3">
        <Card variant="flat" radius="sm" padding="none">
          <CardHeader className="!py-2.5">
            <div className="flex items-center justify-between">
              <CardLabel>Today</CardLabel>
              <Badge tone="success" label="All reversible" size="sm" />
            </div>
          </CardHeader>
          <div>
            {TODAY_RECEIPTS.map((item) => (
              <ListRow
                key={item.text}
                size="sm"
                truncateTitle={false}
                align="start"
                leading={
                  <ToolIcon name={item.text} icon={item.icon} size="sm" shape="circle" className="mt-0.5" />
                }
                title={<span className="text-xs font-normal leading-5">{item.text}</span>}
                meta={item.time}
              />
            ))}
          </div>
          <CardFooter divider className="!py-2">
            <Button variant="ghost" size="sm" className="w-full">
              Open activity log
            </Button>
          </CardFooter>
        </Card>

        <Card variant="flat" radius="sm" padding="none">
          <CardHeader className="!py-2.5">
            <CardLabel>Connections</CardLabel>
          </CardHeader>
          <div>
            {CONNECTIONS.map((conn) => (
              <ListRow
                key={conn.name}
                size="sm"
                leading={<ToolIcon name={conn.name} size="sm" />}
                title={<span className={railTitle}>{conn.name}</span>}
                trailing={<Badge tone={conn.status} label={conn.label} size="sm" />}
                onClick={() => {}}
              />
            ))}
          </div>
          <CardFooter divider className="!py-2">
            <Button variant="ghost" size="sm" className="w-full">
              <Icon name="plus" className="mr-1.5 h-3.5 w-3.5" />
              Connect a tool
            </Button>
          </CardFooter>
        </Card>

        <Card variant="flat" radius="sm" padding="sm">
          <div className="flex items-start gap-2.5">
            <Icon name="shield" className="mt-0.5 h-4 w-4 shrink-0 text-[var(--v2-text-muted)]" />
            <p className="text-xs leading-5 text-[var(--v2-text-muted)]">
              Credentials stay sealed in the vault. Outbound traffic is limited
              to allowlisted endpoints.
            </p>
          </div>
        </Card>
      </div>
    </Card>
  ),
};

/* ── 2 · Workspace — chat thread ──────────────────────────────────── */
/* The core surface on its own: agent narrates what it set up (a receipt
   card in the thread), the user steers. Composer at the bottom. */

export const ChatThread: Story = {
  name: "Workspace · Chat thread",
  render: () => (
    <Card padding="none" className="flex h-[34rem] w-[42rem] flex-col">
      <CardHeader divider className="!py-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2.5">
            <AgentAvatar />
            <div>
              <div className="text-sm font-medium text-[var(--v2-text-strong)]">IronClaw</div>
              <div className="text-xs text-[var(--v2-text-faint)]">
                Connected · Gmail, Calendar, GitHub
              </div>
            </div>
          </div>
          <Badge tone="success" label="Online" size="sm" />
        </div>
      </CardHeader>
      <div className="flex-1 space-y-3.5 overflow-y-auto px-4 py-3.5 md:px-6">
        <ThreadMessages />
      </div>
      <CardFooter divider className="!py-3">
        <Composer />
      </CardFooter>
    </Card>
  ),
};

/* ── 3 · Automations — full page with navigation ──────────────────── */
/* The same table as the fragment below, but inside the app shell:
   sidebar nav, page header, stat strip. What the screen actually is. */

const AUTOMATION_ROWS = [
  { name: "Morning digest", schedule: "Weekdays · 8:00am", status: "success", label: "Success" },
  { name: "Inbox triage", schedule: "Continuous", status: "info", label: "Running" },
  { name: "Invoice chaser", schedule: "Mondays · 9:00am", status: "muted", label: "Paused" },
  { name: "Standup notes", schedule: "Weekdays · 9:30am", status: "danger", label: "Failed" },
] as const;

function AutomationRows({ status = "all" }: { status?: string }) {
  const rows =
    status === "all"
      ? AUTOMATION_ROWS
      : AUTOMATION_ROWS.filter((row) => row.label.toLowerCase() === status);
  return (
    <>
      {rows.map((row) => (
        <ListRow
          key={row.name}
          leading={<Checkbox aria-label={`Select ${row.name}`} />}
          title={row.name}
          description={row.schedule}
          trailing={
            <>
              <Badge tone={row.status} label={row.label} />
              <Button variant="ghost" size="icon-sm" aria-label={`More actions for ${row.name}`}>
                <Icon name="more" className="h-4 w-4" />
              </Button>
            </>
          }
          onClick={() => {}}
        />
      ))}
    </>
  );
}

export const AutomationsPage: Story = {
  name: "Automations · Full page",
  render: function AutomationsPageStory() {
    const [status, setStatus] = useState("all");
    return (
      <Card padding="none" className="flex h-[36rem] w-[64rem] overflow-hidden">
        <WorkspaceSidebar active="automations" />

        {/* Main column */}
        <div className="flex min-w-0 flex-1 flex-col overflow-y-auto">
          <div className="flex items-center justify-between border-b border-[var(--v2-panel-border)] px-5 py-3.5">
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

          <div className="grid grid-cols-3 gap-3.5 px-5 pt-3.5">
            <StatCard label="Active" value="8" tone="success" badgeLabel="Healthy" />
            <StatCard label="Runs today" value="128" tone="info" badgeLabel="Running" />
            <StatCard label="Failures" value="1" tone="danger" badgeLabel="Attention" />
          </div>

          <div className="px-5 pb-5 pt-2">
            <Card variant="flat" padding="none">
              <AutomationRows status={status} />
            </Card>
          </div>
        </div>
      </Card>
    );
  },
};

/* ── 4 · Runs — run detail ────────────────────────────────────────── */
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
      <div>
        {RUN_STEPS.map((step) => (
          <ListRow
            key={step.title}
            size="sm"
            leading={
              <ToolIcon
                name={step.title}
                icon={step.icon}
                size="md"
                className={step.status === "danger" ? "text-[var(--v2-danger-text)]" : undefined}
              />
            }
            title={<span className="font-normal">{step.title}</span>}
            trailing={
              <>
                <span className={mono}>{step.duration}</span>
                <Badge tone={step.status} label={step.label} size="sm" />
              </>
            }
          />
        ))}
      </div>
      <div className="px-4 pb-4 pt-2 md:px-6">
        <Card variant="inset" radius="sm" padding="sm">
          <div className="font-mono text-xs leading-5 text-[var(--v2-text-muted)]">
            SlackDeliveryError: token for #personal has expired.
            <br />
            Generate a new token in Settings → Connections, then retry.
          </div>
        </Card>
      </div>
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

/* ── 5 · Activity — agent receipts feed ───────────────────────────── */
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
      <div>
        {ACTIVITY.map((item) => (
          <ListRow
            key={item.text}
            align="start"
            truncateTitle={false}
            leading={
              <ToolIcon name={item.text} icon={item.icon} size="md" shape="circle" className="mt-0.5" />
            }
            title={<span className="font-normal">{item.text}</span>}
            meta={item.time}
            trailing={
              <Button variant="ghost" size="sm" className="shrink-0">
                {item.action}
              </Button>
            }
          />
        ))}
      </div>
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

/* ── 6 · Agent workspace — settings panel ─────────────────────────── */

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

/* ── 7 · Automations — table card (fragment) ──────────────────────── */

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
            <AutomationRows />
          </div>
        </Card>
      </div>
    );
  },
};

/* ── 8 · Marketing — hero fragment ────────────────────────────────── */

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

/* ── 9 · Onboarding — routine card ────────────────────────────────── */
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
