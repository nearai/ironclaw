/**
 * Component galleries for /playground — live renders of every
 * design-system component in each of its variants/states, plus the
 * import line an agent should copy. These render the real
 * components from src/design-system/, so the gallery cannot
 * drift from what pages ship.
 */
import { useState } from "react";
import { Avatar, AvatarFallback } from "@ironclaw/design-system";
import { Badge } from "@ironclaw/design-system";
import { Button } from "@ironclaw/design-system";
import { Card, CardBody, CardFooter, CardHeader, CardLabel } from "@ironclaw/design-system";
import { Checkbox } from "@ironclaw/design-system";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@ironclaw/design-system";
import { Icon } from "@ironclaw/design-system";
import { FormField, Input, Select, Textarea } from "@ironclaw/design-system";
import { Modal, ModalBody, ModalFooter } from "@ironclaw/design-system";
import { Popover, PopoverContent, PopoverTrigger } from "@ironclaw/design-system";
import {
  EmptyPanel,
  FlowList,
  SectionHeader,
  StatCard,
  SubLabel,
} from "@ironclaw/design-system";
import { RadioGroup, RadioGroupItem } from "@ironclaw/design-system";
import { ScrollArea } from "@ironclaw/design-system";
import { SelectMenu } from "@ironclaw/design-system";
import { Separator } from "@ironclaw/design-system";
import { Skeleton } from "@ironclaw/design-system";
import { Slider } from "@ironclaw/design-system";
import { Switch } from "@ironclaw/design-system";
import { Tabs } from "@ironclaw/design-system";
import { Tooltip, TooltipProvider } from "@ironclaw/design-system";
import { STATUS_CANON } from "@ironclaw/design-system/tokens";
import { SectionTitle } from "./token-sections";

/* ── Shared bits ───────────────────────────────────────────────────── */

function ImportLine({ children }) {
  return (
    <pre
      className="mb-6 overflow-x-auto rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3 font-mono text-[0.75rem] leading-5 text-[var(--v2-text)]"
    >{children}</pre>
  );
}

function Row({ children, className = "" }) {
  return <div className={`flex flex-wrap items-center gap-3 ${className}`}>{children}</div>;
}

function Caption({ children }) {
  return (
    <span className="font-mono text-[0.625rem] text-[var(--v2-text-faint)]">{children}</span>
  );
}

/* ── Button ───────────────────────────────────────────────────────── */

const BUTTON_VARIANTS = ["primary", "outline", "secondary", "ghost", "danger"] as const;

export function ButtonSection() {
  return (
    <div>
      <ImportLine>import {"{ Button }"} from "@ironclaw/design-system";</ImportLine>

      <SectionTitle>Variants</SectionTitle>
      <Row>
        {BUTTON_VARIANTS.map((variant) => (
          <div key={variant} className="flex flex-col items-center gap-1.5">
            <Button variant={variant}>{variant}</Button>
            <Caption>variant="{variant}"</Caption>
          </div>
        ))}
      </Row>

      <SectionTitle>Sizes — compact control scale</SectionTitle>
      <Row className="items-end">
        {([
          ["sm", "sm · 28px", "New chat"],
          ["md", "md (default) · 32px", "New chat"],
          ["lg", "lg · 36px", "New chat"],
        ] as const).map(([size, caption, label]) => (
          <div key={size} className="flex flex-col items-center gap-1.5">
            <Button variant="secondary" size={size}>{label}</Button>
            <Caption>{caption}</Caption>
          </div>
        ))}
        <div className="flex flex-col items-center gap-1.5">
          <Button variant="secondary" size="icon" aria-label="Add">
            <Icon name="plus" className="h-4 w-4" />
          </Button>
          <Caption>icon · 32px sq</Caption>
        </div>
        <div className="flex flex-col items-center gap-1.5">
          <Button variant="secondary" size="icon-sm" aria-label="Add">
            <Icon name="plus" className="h-4 w-4" />
          </Button>
          <Caption>icon-sm · 28px sq</Caption>
        </div>
      </Row>
      <p className="mt-4 max-w-[62ch] text-sm leading-6 text-[var(--v2-text-muted)]">
        Heights and paddings come from the shared{" "}
        <code className="font-mono text-[0.75rem]">--v2-control-h-* / --v2-control-px-*</code>{" "}
        density tokens, so buttons and inputs align in mixed rows.
      </p>

      <SectionTitle>States</SectionTitle>
      <Row>
        <Button disabled>primary disabled</Button>
        <Button variant="secondary" disabled>secondary disabled</Button>
        <Button fullWidth className="max-w-xs">
          <Icon name="send" className="h-4 w-4" /> fullWidth with icon
        </Button>
      </Row>
    </div>
  );
}

/* ── Badge ────────────────────────────────────────────────────────── */

const BADGE_TONES = ["success", "warning", "danger", "info", "accent", "muted"];

export function BadgeSection() {
  return (
    <div>
      <ImportLine>import {"{ Badge }"} from "@ironclaw/design-system";</ImportLine>

      <SectionTitle>Tones</SectionTitle>
      <Row>
        {BADGE_TONES.map((tone) => (
          <Badge key={tone} tone={tone} label={tone} />
        ))}
      </Row>

      <SectionTitle>Status canon</SectionTitle>
      <p className="mb-3 max-w-[62ch] text-sm leading-6 text-[var(--v2-text-muted)]">
        The one mapping from product status words to tokens. Text, dots,
        and progress fills for a status must all come from the same pair
        (STATUS_CANON in design-system/tokens.ts) — never a second hue.
      </p>
      <div className="flex flex-col gap-2">
        {STATUS_CANON.map((entry) => (
          <div key={entry.tone} className="flex flex-wrap items-center gap-3">
            <span className="w-64 shrink-0 text-[0.8125rem] text-[var(--v2-text)]">
              {entry.status}
            </span>
            <Badge tone={entry.tone} label={entry.tone} />
            <span
              className="h-2 w-24 shrink-0 overflow-hidden rounded-full"
              style={{ background: `var(${entry.fill})` }}
            >
              <span
                className="block h-full w-2/3 rounded-full"
                style={{ background: `var(${entry.text})` }}
              />
            </span>
            <Caption>{entry.text} / {entry.fill}</Caption>
          </div>
        ))}
      </div>

      <SectionTitle>Sizes + no dot</SectionTitle>
      <Row>
        <Badge tone="info" size="sm" label="size sm" />
        <Badge tone="info" size="md" label="size md" />
        <Badge tone="muted" dot={false} label="dot=false" />
      </Row>
      <p className="mt-4 max-w-[62ch] text-sm leading-6 text-[var(--v2-text-muted)]">
        Success/positive/signal tones render a breathing dot — the sanctioned
        "live" indicator. Pass a translated{" "}
        <code className="font-mono text-[0.75rem]">label</code>; never rely on
        the tone keyword as user-facing copy.
      </p>
    </div>
  );
}

/* ── Card ─────────────────────────────────────────────────────────── */

const CARD_VARIANTS = ["default", "bordered", "flat", "subtle", "inset"];

export function CardSection() {
  return (
    <div>
      <ImportLine>import {"{ Card, CardHeader, CardBody, CardFooter, CardLabel }"} from "@ironclaw/design-system";</ImportLine>

      <SectionTitle>Variants</SectionTitle>
      <div className="grid gap-4 sm:grid-cols-2">
        {CARD_VARIANTS.map((variant) => (
          <Card key={variant} variant={variant} padding="md">
            <CardLabel>variant="{variant}"</CardLabel>
            <p className="mt-2 text-sm text-[var(--v2-text-muted)]">
              Card surface backed by themed tokens.
            </p>
          </Card>
        ))}
      </div>
      <p className="mt-4 max-w-[62ch] text-sm leading-6 text-[var(--v2-text-muted)]">
        <code className="font-mono text-[0.75rem]">flat</code> is
        border-defined with no shadow at all — use it for in-page cards
        (tables, stat grids) that should sit flush on the canvas instead
        of floating above it.
      </p>

      <SectionTitle>Composed: header / body / footer</SectionTitle>
      <Card className="max-w-lg">
        <CardHeader divider>
          <CardLabel>Channels</CardLabel>
          <h3 className="mt-1 text-[1.2rem] font-semibold tracking-[-0.02em] text-[var(--v2-text-strong)]">
            Slack workspace
          </h3>
        </CardHeader>
        <CardBody>
          <p className="text-sm leading-6 text-[var(--v2-text-muted)]">
            Route thread replies into a shared channel. Composition:
            CardHeader with divider, CardBody, CardFooter.
          </p>
        </CardBody>
        <CardFooter>
          <Button variant="ghost" size="sm">Cancel</Button>
          <Button size="sm">Connect</Button>
        </CardFooter>
      </Card>
    </div>
  );
}

/* ── Inputs ───────────────────────────────────────────────────────── */

export function InputSection() {
  return (
    <div>
      <ImportLine>import {"{ Input, Textarea, Select, Label, FormField }"} from "@ironclaw/design-system";</ImportLine>

      <SectionTitle>Input sizes + states</SectionTitle>
      <div className="flex max-w-md flex-col gap-4">
        <Input size="sm" placeholder="size sm" />
        <Input placeholder="size md (default)" />
        <Input placeholder="disabled" disabled />
        <Input placeholder="error state" error />
      </div>

      <SectionTitle>FormField composition</SectionTitle>
      <div className="flex max-w-md flex-col gap-5">
        <FormField label="API token" hint="Stored locally, never echoed." required>
          <Input placeholder="icw_..." />
        </FormField>
        <FormField label="Provider" error="Select a provider to continue.">
          <Select error>
            <option value="">Choose…</option>
            <option value="one">Provider One</option>
          </Select>
        </FormField>
        <FormField label="Notes">
          <Textarea placeholder="Optional deployment notes" rows={3} />
        </FormField>
      </div>
    </div>
  );
}

/* ── Tabs ─────────────────────────────────────────────────────────── */

const DEMO_TABS = [
  { value: "all", label: "All", count: 12 },
  { value: "active", label: "Active", count: 8 },
  { value: "paused", label: "Paused", count: 3 },
  { value: "failing", label: "Failing", count: 1 },
];

export function TabsSection() {
  const [tab, setTab] = useState("all");
  const [toolbarTab, setToolbarTab] = useState("active");
  return (
    <div>
      <ImportLine>import {"{ Tabs }"} from "@ironclaw/design-system";</ImportLine>

      <SectionTitle>Underline tabs</SectionTitle>
      <Tabs
        tabs={DEMO_TABS}
        value={tab}
        onChange={setTab}
        ariaLabel="Filter automations"
        className="max-w-xl"
      />
      <p className="mt-4 max-w-[62ch] text-sm leading-6 text-[var(--v2-text-muted)]">
        Single-select filters over one collection. Row height derives from
        the shared control tokens (
        <code className="font-mono text-[0.75rem]">--v2-control-h-md + --v2-control-px-sm</code>
        ) so a tab row lines up with buttons and selects in adjacent
        toolbars. Below the <code className="font-mono text-[0.75rem]">sm</code>{" "}
        breakpoint, swap to a SelectMenu instead of shrinking the row.
      </p>

      <SectionTitle>In a toolbar (bordered=false)</SectionTitle>
      <div className="flex max-w-xl items-stretch justify-between gap-3 border-b border-[var(--v2-panel-border)]">
        <Tabs
          tabs={DEMO_TABS.slice(0, 3)}
          value={toolbarTab}
          onChange={setToolbarTab}
          ariaLabel="Toolbar tabs demo"
          bordered={false}
        />
        <div className="flex items-center gap-2 pb-2">
          <Button variant="secondary" size="sm">Set defaults</Button>
        </div>
      </div>
      <p className="mt-4 max-w-[62ch] text-sm leading-6 text-[var(--v2-text-muted)]">
        Pass <code className="font-mono text-[0.75rem]">bordered=false</code>{" "}
        when a parent toolbar owns the baseline hairline, so right-side
        controls share the same rule and the tabs stretch to center
        against them.
      </p>
    </div>
  );
}

/* ── SelectMenu ───────────────────────────────────────────────────── */

const SORT_OPTIONS = [
  { value: "next-run", label: "Next run" },
  { value: "name", label: "Name" },
  { value: "recent", label: "Recently created" },
];

const STATUS_OPTIONS = [
  { value: "ok", label: "Healthy", tone: "positive" },
  { value: "degraded", label: "Degraded", tone: "warning" },
  { value: "down", label: "Failing", tone: "danger" },
];

export function SelectMenuSection() {
  const [sort, setSort] = useState("next-run");
  const [status, setStatus] = useState("ok");
  return (
    <div>
      <ImportLine>import {"{ SelectMenu }"} from "@ironclaw/design-system";</ImportLine>

      <SectionTitle>With a prefix label</SectionTitle>
      <Row>
        <SelectMenu
          prefix="Sort"
          ariaLabel="Sort automations"
          value={sort}
          options={SORT_OPTIONS}
          onChange={setSort}
          className="w-48"
          align="left"
        />
      </Row>
      <p className="mt-4 max-w-[62ch] text-sm leading-6 text-[var(--v2-text-muted)]">
        <code className="font-mono text-[0.75rem]">prefix</code> renders a
        muted inline label inside the trigger, before the selected value —
        use it instead of an external label so the control stays
        self-describing in dense toolbars.
      </p>

      <SectionTitle>Option tones</SectionTitle>
      <Row>
        <SelectMenu
          ariaLabel="Status filter"
          value={status}
          options={STATUS_OPTIONS}
          onChange={setStatus}
          className="w-44"
          align="left"
        />
      </Row>
      <p className="mt-4 max-w-[62ch] text-sm leading-6 text-[var(--v2-text-muted)]">
        The menu floats on{" "}
        <code className="font-mono text-[0.75rem]">--v2-shadow-menu</code>:
        the hairline border does the separation work and the shadow only
        lifts the surface (see Radii &amp; Shadows).
      </p>
    </div>
  );
}

/* ── Modal ────────────────────────────────────────────────────────── */

export function ModalSection() {
  const [open, setOpen] = useState(false);
  return (
    <div>
      <ImportLine>import {"{ Modal, ModalBody, ModalFooter }"} from "@ironclaw/design-system";</ImportLine>

      <SectionTitle>Dialog</SectionTitle>
      <Button variant="secondary" onClick={() => setOpen(true)}>Open modal</Button>
      <p className="mt-4 max-w-[62ch] text-sm leading-6 text-[var(--v2-text-muted)]">
        Backdrop click and Escape both close. Body scroll locks while open.
        Sits on the modal layer (var(--v2-z-modal)).
      </p>

      <Modal open={open} onClose={() => setOpen(false)} title="Remove extension" closeLabel="Close">
        <ModalBody>
          <p className="text-sm leading-6 text-[var(--v2-text-muted)]">
            This removes the extension and its stored configuration. Threads
            that used it keep their history.
          </p>
        </ModalBody>
        <ModalFooter>
          <Button variant="ghost" size="sm" onClick={() => setOpen(false)}>Cancel</Button>
          <Button variant="danger" size="sm" onClick={() => setOpen(false)}>Remove</Button>
        </ModalFooter>
      </Modal>
    </div>
  );
}

/* ── Icons ────────────────────────────────────────────────────────── */

const ICON_NAMES = [
  "attach", "bolt", "bell", "bookOpen", "calendar", "check", "chat", "close",
  "clock", "download", "file", "flag", "pin", "pause", "play", "folder",
  "layers", "list", "logs", "lock", "logout", "moon", "plug", "plus",
  "pulse", "send", "search", "settings", "spark", "sun", "shield", "tool",
  "terminal", "trash", "upload", "chevron", "more", "copy", "arrowDown", "retry",
];

export function IconSection() {
  return (
    <div>
      <ImportLine>import {"{ Icon }"} from "@ironclaw/design-system";</ImportLine>

      <SectionTitle>Icon set — 24px grid, stroke 1.7</SectionTitle>
      <div className="grid grid-cols-4 gap-3 sm:grid-cols-6 lg:grid-cols-8">
        {ICON_NAMES.map((name) => (
          <div
            key={name}
            className="flex flex-col items-center gap-2 rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] px-2 py-3"
          >
            <Icon name={name} className="h-5 w-5 text-[var(--v2-text-strong)]" />
            <Caption>{name}</Caption>
          </div>
        ))}
      </div>
    </div>
  );
}

/* ── Primitives ───────────────────────────────────────────────────── */

export function PrimitivesSection() {
  return (
    <div>
      <ImportLine>import {"{ StatCard, FlowList, EmptyPanel, SectionHeader, SubLabel }"} from "@ironclaw/design-system";</ImportLine>

      <SectionTitle>SectionHeader (md+ only)</SectionTitle>
      <SectionHeader title="Automations" subtitle="Recurring work the agent runs for you." />

      <SectionTitle>StatCard</SectionTitle>
      <div className="grid max-w-2xl gap-0 sm:grid-cols-2">
        <StatCard label="Active runs" value="12" tone="success" badgeLabel="live" detail="3 waiting on approval" />
        <StatCard label="Failures (7d)" value="2" tone="danger" badgeLabel="attention" />
      </div>

      <SectionTitle>SubLabel + FlowList</SectionTitle>
      <SubLabel>How pairing works</SubLabel>
      <div className="max-w-2xl">
        <FlowList
          items={[
            { title: "Generate a code", description: "The agent issues a one-time pairing code." },
            { title: "Paste it in Slack", description: "Run /ironclaw pair in the target channel." },
            { title: "Confirm", description: "The channel appears under Extensions → Channels." },
          ]}
        />
      </div>

      <SectionTitle>EmptyPanel</SectionTitle>
      <div className="max-w-2xl">
        <EmptyPanel
          title="No automations yet"
          description="Create one from any chat thread — ask the agent to do something on a schedule."
        >
          <Button size="sm">New automation</Button>
        </EmptyPanel>
      </div>
    </div>
  );
}

/* ── Overlay / form controls (Radix) ──────────────────────────────── */

export function TooltipSection() {
  return (
    <div>
      <ImportLine>import {"{ Tooltip, TooltipProvider }"} from "@ironclaw/design-system";</ImportLine>
      <SectionTitle>Hover / focus tip</SectionTitle>
      <TooltipProvider>
        <Tooltip content="Open the agent settings">
          <Button variant="secondary" size="sm">Hover me</Button>
        </Tooltip>
      </TooltipProvider>
    </div>
  );
}

export function CheckboxSwitchSection() {
  const [checked, setChecked] = useState(true);
  const [enabled, setEnabled] = useState(false);
  const [plan, setPlan] = useState("pro");
  const [volume, setVolume] = useState([40]);
  return (
    <div>
      <ImportLine>import {"{ Checkbox }"} from "@ironclaw/design-system";
import {"{ Switch }"} from "@ironclaw/design-system";
import {"{ RadioGroup, RadioGroupItem }"} from "@ironclaw/design-system";
import {"{ Slider }"} from "@ironclaw/design-system";</ImportLine>

      <SectionTitle>Checkbox + Switch</SectionTitle>
      <Row>
        <label className="inline-flex items-center gap-2 text-sm text-[var(--v2-text)]">
          <Checkbox checked={checked} onCheckedChange={(v) => setChecked(v === true)} />
          Email digests
        </label>
        <label className="inline-flex items-center gap-2 text-sm text-[var(--v2-text)]">
          <Switch checked={enabled} onCheckedChange={setEnabled} />
          Auto-run
        </label>
      </Row>

      <SectionTitle>RadioGroup</SectionTitle>
      <RadioGroup value={plan} onValueChange={setPlan} className="max-w-sm">
        {["free", "pro", "team"].map((value) => (
          <label key={value} className="flex items-center gap-2 text-sm text-[var(--v2-text)]">
            <RadioGroupItem value={value} id={`plan-${value}`} />
            {value}
          </label>
        ))}
      </RadioGroup>

      <SectionTitle>Slider</SectionTitle>
      <div className="max-w-sm">
        <Slider value={volume} onValueChange={setVolume} max={100} step={1} />
        <Caption>{volume[0]}%</Caption>
      </div>
    </div>
  );
}

export function DropdownPopoverSection() {
  return (
    <div>
      <ImportLine>import {"{ DropdownMenu, DropdownMenuTrigger, DropdownMenuContent, DropdownMenuItem }"} from "@ironclaw/design-system";
import {"{ Popover, PopoverTrigger, PopoverContent }"} from "@ironclaw/design-system";</ImportLine>

      <SectionTitle>DropdownMenu</SectionTitle>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="secondary" size="sm">Actions</Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start">
          <DropdownMenuLabel>Thread</DropdownMenuLabel>
          <DropdownMenuItem>Rename</DropdownMenuItem>
          <DropdownMenuItem>Pin</DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem>Delete</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      <SectionTitle>Popover</SectionTitle>
      <Popover>
        <PopoverTrigger asChild>
          <Button variant="outline" size="sm">Open popover</Button>
        </PopoverTrigger>
        <PopoverContent align="start">
          <p className="text-sm leading-6 text-[var(--v2-text-muted)]">
            Anchored content for denser chrome — menus stay in DropdownMenu / SelectMenu.
          </p>
        </PopoverContent>
      </Popover>
    </div>
  );
}

export function AvatarSkeletonSection() {
  return (
    <div>
      <ImportLine>import {"{ Avatar, AvatarFallback }"} from "@ironclaw/design-system";
import {"{ Skeleton }"} from "@ironclaw/design-system";
import {"{ Separator }"} from "@ironclaw/design-system";
import {"{ ScrollArea }"} from "@ironclaw/design-system";</ImportLine>

      <SectionTitle>Avatar</SectionTitle>
      <Row>
        <Avatar>
          <AvatarFallback>IC</AvatarFallback>
        </Avatar>
        <Avatar className="h-10 w-10">
          <AvatarFallback>AG</AvatarFallback>
        </Avatar>
      </Row>

      <SectionTitle>Skeleton + Separator</SectionTitle>
      <div className="max-w-sm space-y-3">
        <Skeleton className="h-4 w-2/3" />
        <Skeleton className="h-4 w-full" />
        <Separator />
        <Skeleton className="h-20 w-full" />
      </div>

      <SectionTitle>ScrollArea</SectionTitle>
      <ScrollArea className="h-28 max-w-sm rounded-[12px] border border-[var(--v2-panel-border)] p-3">
        <div className="space-y-2 text-sm text-[var(--v2-text-muted)]">
          {Array.from({ length: 12 }, (_, i) => (
            <div key={i}>Scrollable row {i + 1}</div>
          ))}
        </div>
      </ScrollArea>
    </div>
  );
}
