import { useState } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
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
 * Applied compositions — one fragment per product touchpoint, built purely
 * from design-system components ("approach it from an atomic standpoint …
 * pick a few from each of the touchpoints and apply that"). These prove the
 * primitives compose into real surfaces without one-off styling.
 */
const meta = {
  title: "Compositions/Touchpoints",
  parameters: {
    layout: "padded",
    docs: {
      description: {
        component:
          "Real product fragments assembled exclusively from design-system " +
          "components: a workspace settings panel, the automations table card, a " +
          "marketing hero fragment, and the onboarding proposal card. If a " +
          "composition needs a class the system doesn't provide, that's a gap in " +
          "the system — not a license for one-off styling.",
      },
    },
  },
} satisfies Meta;

export default meta;
type Story = StoryObj<typeof meta>;

/* ── 1 · Agent workspace — settings panel ─────────────────────────── */

export const WorkspaceSettings: Story = {
  name: "Workspace · Settings panel",
  render: function WorkspaceSettingsStory() {
    const [tab, setTab] = useState("agent");
    const [autoApprove, setAutoApprove] = useState(false);
    return (
      <Card padding="none" className="w-[36rem]">
        <CardHeader divider>
          <CardLabel>Settings</CardLabel>
          <h3 className="mt-1 text-lg font-semibold text-[var(--v2-text-strong)]">
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
                <Label htmlFor="auto-approve" className="!mb-0">Auto-approve routine runs</Label>
                <p className="mt-0.5 text-xs text-[var(--v2-text-faint)]">
                  Suggest-only stays the default for new automations.
                </p>
              </div>
              <Switch
                id="auto-approve"
                checked={autoApprove}
                onCheckedChange={setAutoApprove}
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

/* ── 2 · Automations — table card ─────────────────────────────────── */

const AUTOMATION_ROWS = [
  { name: "Morning digest", schedule: "Weekdays · 8:00am", status: "success", label: "Success" },
  { name: "Inbox triage", schedule: "Continuous", status: "info", label: "Running" },
  { name: "Invoice chaser", schedule: "Mondays · 9:00am", status: "muted", label: "Paused" },
  { name: "Standup notes", schedule: "Weekdays · 9:30am", status: "danger", label: "Failed" },
] as const;

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
              <h3 className="font-semibold text-[var(--v2-text-strong)]">Automations</h3>
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

/* ── 3 · Marketing — hero fragment ────────────────────────────────── */

export const MarketingHero: Story = {
  name: "Marketing · Hero fragment",
  render: () => (
    <div className="flex w-[40rem] flex-col items-center py-10 text-center">
      <Badge tone="accent" label="Private by design" />
      <h1 className="mt-5 text-[length:var(--v2-font-size-display)] font-semibold leading-tight tracking-[var(--v2-tracking-display)] text-[var(--v2-text-strong)]">
        An agent that runs
        <br />
        your routine work.
      </h1>
      <p className="mt-4 max-w-md text-[length:var(--v2-font-size-body-lg)] text-[var(--v2-text-muted)]">
        Connect your tools once. IronClaw reads your world, suggests
        automations, and never acts without your approval.
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

/* ── 4 · Onboarding — proposal card ───────────────────────────────── */

export const OnboardingProposal: Story = {
  name: "Onboarding · Proposal card",
  render: () => (
    <Card className="w-[26rem]" padding="none">
      <CardHeader>
        <div className="flex items-start justify-between gap-3">
          <div className="flex items-center gap-2.5">
            <span className="grid h-9 w-9 place-items-center rounded-[var(--v2-radius-md)] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]">
              <Icon name="spark" className="h-4.5 w-4.5 text-[var(--v2-accent-text)]" />
            </span>
            <div>
              <CardLabel>Suggested automation</CardLabel>
              <h3 className="mt-0.5 font-semibold text-[var(--v2-text-strong)]">
                Morning digest
              </h3>
            </div>
          </div>
          <Badge tone="muted" label="Suggest-only" size="sm" />
        </div>
      </CardHeader>
      <CardBody className="!pt-0 text-sm leading-6 text-[var(--v2-text-muted)]">
        I found 34 newsletter threads and 12 recurring status emails in your
        inbox. I can summarize them into one morning digest — you approve
        every send until you say otherwise.
      </CardBody>
      <CardFooter divider>
        <div className="flex items-center justify-between">
          <Button variant="ghost" size="sm">View details</Button>
          <div className="flex gap-2">
            <Button variant="secondary" size="sm">Dismiss</Button>
            <Button size="sm">
              <Icon name="check" className="mr-1.5 h-3.5 w-3.5" />
              Approve
            </Button>
          </div>
        </div>
      </CardFooter>
    </Card>
  ),
};
