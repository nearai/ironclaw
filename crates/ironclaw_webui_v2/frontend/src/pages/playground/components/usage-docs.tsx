/**
 * Per-section usage docs for /playground: a copyable example snippet,
 * a props/variants summary, and the deep link into the hosted
 * Storybook (the canonical spec — full variant coverage, a11y checks,
 * and source exploration live there; the playground is the in-product
 * harness).
 */
import { useCallback, useState } from "react";
import { Icon, cn } from "@ironclaw/design-system";

export const STORYBOOK_URL = "https://ironclaw-storybook.vercel.app";

type UsageDoc = {
  /** Copyable usage example (import + a representative render). */
  snippet: string;
  /** prop → summary pairs shown as a compact reference. */
  props?: [string, string][];
  /** Storybook path (joined onto STORYBOOK_URL). */
  storybook: string;
};

export const USAGE_DOCS: Record<string, UsageDoc> = {
  /* ── Tokens ── */
  colors: {
    snippet: `import { COLOR_TOKENS, STATUS_CANON, readToken } from "@ironclaw/design-system/tokens";

// In styles: always the custom property, never a raw value
<div className="bg-[var(--v2-surface)] text-[var(--v2-text-muted)]" />

// At runtime (theme-aware)
const accent = readToken("--v2-accent");`,
    props: [
      ["COLOR_TOKENS", "catalog of every color token, grouped by role"],
      ["STATUS_CANON", "the one status-word → token mapping"],
      ["readToken(name)", "computed value under the active theme"],
    ],
    storybook: "/?path=/story/tokens-reference--colors",
  },
  typography: {
    snippet: `<h1 className="text-[length:var(--v2-font-size-display)] tracking-[var(--v2-tracking-display)]">
  Automations
</h1>
<span className="v2-tag-face text-[length:var(--v2-font-size-label)]">TRACE COMMONS</span>`,
    props: [
      ["--v2-font-size-*", "label 11 · caption 12 · body-sm 13 · body 14 · body-lg 16 · title 20 · heading 24 · display-sm 28 · display 36"],
      ["--v2-tracking-*", "tag 0.08em · caps 0.14em · wide 0.22em · tight −0.02em · display −0.04em"],
      [".v2-tag-face", "pixel-face uppercase tag language"],
    ],
    storybook: "/?path=/story/tokens-reference--typography",
  },
  spacing: {
    snippet: `<div className="grid gap-[var(--v2-space-4)] p-[var(--v2-space-7)]">
  {/* form-field stack inside desktop card padding */}
</div>`,
    props: [
      ["--v2-space-1…10", "4px grid: hairline 4 → hero breathing room 40"],
      ["--v2-control-h-*", "control heights 28/32/36 — never invent one"],
    ],
    storybook: "/?path=/story/tokens-reference--radius-and-space",
  },
  radii: {
    snippet: `<div className="rounded-[var(--v2-radius-lg)] shadow-[var(--v2-card-shadow)]
  border border-[var(--v2-card-border)] bg-[var(--v2-card-bg)]" />`,
    props: [
      ["--v2-radius-*", "xs 6 · sm 8 · md 10 · lg 16 · xl 20 · 2xl 24 · full"],
      ["--v2-card-shadow", "cards: minimal 1px lift, borders separate"],
      ["--v2-shadow-menu / -modal", "menus/popovers · dialogs"],
    ],
    storybook: "/?path=/story/tokens-reference--elevation",
  },
  motion: {
    snippet: `// CSS: pick a duration + easing pair, never raw values
"transition-colors duration-[var(--v2-duration-fast)] ease-[var(--v2-ease-standard)]"

// motion/react: mirrored constants
import { MOTION_DURATION, MOTION_EASE_OUT } from "@ironclaw/design-system/motion";`,
    props: [
      ["--v2-duration-*", "instant 100 · exit 120 · fast 150 · menu 180 · base 250 · slow 400"],
      ["--v2-ease-*", "standard · in-out · out-expo (entrances) · spring (pops)"],
      ["useReducedMotion()", "all motion must respect it"],
    ],
    storybook: "/?path=/story/tokens-reference--motion-and-z",
  },
  "z-index": {
    snippet: `<div className="z-[var(--v2-z-modal)]">{/* dialogs, palettes */}</div>`,
    props: [["--v2-z-*", "raised 10 · sticky 20 · overlay 40 · modal 50 · toast 60"]],
    storybook: "/?path=/story/tokens-reference--motion-and-z",
  },

  /* ── Components ── */
  button: {
    snippet: `import { Button } from "@ironclaw/design-system";

<Button onClick={save}>Save changes</Button>
<Button variant="secondary" size="sm" loading={isSaving}>Refresh</Button>
<Button variant="danger" onClick={confirmDelete}>Delete</Button>`,
    props: [
      ["variant", "primary · outline · secondary · ghost · danger"],
      ["size", "sm · md · lg · icon · icon-sm"],
      ["loading", "spinner + disabled, label keeps its width"],
      ["as / href / to", "renders as <a> or a Link-like component"],
      ["fullWidth · disabled", "layout / state flags"],
    ],
    storybook: "/?path=/docs/components-button--docs",
  },
  badge: {
    snippet: `import { Badge } from "@ironclaw/design-system";

<Badge tone="success" label={t("status.running")} />`,
    props: [
      ["tone", "success · info · warning · danger · accent · muted (STATUS_CANON)"],
      ["label", "translated copy — tone never carries the text"],
      ["dot", "status dot (default true); success tones breathe"],
      ["size", "sm · md"],
    ],
    storybook: "/?path=/docs/components-badge--docs",
  },
  card: {
    snippet: `import { Card, CardBody, CardFooter, CardHeader, CardLabel } from "@ironclaw/design-system";

<Card padding="none">
  <CardHeader divider><CardLabel>Automation</CardLabel></CardHeader>
  <CardBody>…</CardBody>
  <CardFooter>…</CardFooter>
</Card>`,
    props: [
      ["variant", "default · bordered · flat (in-page) · subtle · inset"],
      ["radius", "sm · md · lg"],
      ["padding", "none · sm · md · lg"],
      ["sub-components", "CardHeader/Footer (divider) · CardBody · CardLabel"],
    ],
    storybook: "/?path=/docs/components-card--docs",
  },
  input: {
    snippet: `import { FormField, Input } from "@ironclaw/design-system";

<FormField label="Workspace name" hint="Shown in the sidebar." error={errors.name}>
  <Input value={name} onChange={(e) => setName(e.target.value)} />
</FormField>`,
    props: [
      ["Input / Select size", "sm · md · lg (control-density scale)"],
      ["error", "boolean on controls; message string on FormField"],
      ["FormField", "label + control + hint/error slots"],
      ["Label required", "appends the required marker"],
      ["Textarea rows", "initial height"],
    ],
    storybook: "/?path=/docs/components-input--docs",
  },
  "select-menu": {
    snippet: `import { SelectMenu } from "@ironclaw/design-system";

<SelectMenu
  prefix="Status"
  value={status}
  onChange={setStatus}
  options={[
    { value: "all", label: "All" },
    { value: "failed", label: "Failed", tone: "danger" },
  ]}
/>`,
    props: [
      ["options", "{ value, label?, tone?, disabled? }[]"],
      ["tone", "neutral · positive · warning · danger · info · accent dots"],
      ["prefix", "inline label for dense toolbars"],
      ["align", "left · right menu alignment"],
    ],
    storybook: "/?path=/docs/components-selectmenu--docs",
  },
  tabs: {
    snippet: `import { Tabs } from "@ironclaw/design-system";

<Tabs
  ariaLabel="Automation filters"
  value={tab}
  onChange={setTab}
  tabs={[{ value: "all", label: "All", count: 12 }, { value: "failed", label: "Failed" }]}
/>`,
    props: [
      ["tabs", "{ value, label, count? }[]"],
      ["value / onChange", "controlled selection"],
      ["bordered", "full-width baseline rule"],
    ],
    storybook: "/?path=/docs/components-tabs--docs",
  },
  modal: {
    snippet: `import { Modal, ModalBody, ModalFooter, ModalHeader } from "@ironclaw/design-system";

<Modal open={open} onClose={close} size="md">
  <ModalHeader onClose={close}>Configure extension</ModalHeader>
  <ModalBody>…</ModalBody>
  <ModalFooter>…</ModalFooter>
</Modal>`,
    props: [
      ["open / onClose", "controlled; Escape + scrim close when onClose set"],
      ["size", "sm · md · lg · xl · full"],
      ["ConfirmDialog", "packaged destructive-action pattern"],
    ],
    storybook: "/?path=/docs/components-modal--docs",
  },
  tooltip: {
    snippet: `import { Tooltip, TooltipProvider } from "@ironclaw/design-system";

<TooltipProvider>
  <Tooltip content="Run now">
    <Button size="icon" aria-label="Run now"><Icon name="play" /></Button>
  </Tooltip>
</TooltipProvider>`,
    props: [
      ["content", "the hint (opens on hover and focus)"],
      ["side / align", "placement"],
      ["TooltipProvider", "mount once to share delayDuration"],
    ],
    storybook: "/?path=/docs/components-tooltip--docs",
  },
  controls: {
    snippet: `import { Checkbox, RadioGroup, RadioGroupItem, Slider, Switch } from "@ironclaw/design-system";

<Switch checked={on} onCheckedChange={setOn} />
<Checkbox defaultChecked />
<RadioGroup value={mode} onValueChange={setMode}>…</RadioGroup>
<Slider defaultValue={[40]} max={100} step={5} />`,
    props: [
      ["Radix pass-through", "checked/onCheckedChange, value/onValueChange, disabled…"],
      ["keyboard", "Space toggles; arrows move radio/slider"],
    ],
    storybook: "/?path=/docs/components-selection-controls--docs",
  },
  menus: {
    snippet: `import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@ironclaw/design-system";

<DropdownMenu>
  <DropdownMenuTrigger asChild><Button variant="ghost" size="icon" /></DropdownMenuTrigger>
  <DropdownMenuContent align="end">
    <DropdownMenuItem onSelect={runNow}>Run now</DropdownMenuItem>
  </DropdownMenuContent>
</DropdownMenu>`,
    props: [
      ["DropdownMenu", "actions/commands (roving focus, typeahead)"],
      ["Popover", "anchored content: inline forms, detail panes"],
      ["align / sideOffset", "placement relative to the trigger"],
    ],
    storybook: "/?path=/docs/components-dropdownmenu--docs",
  },
  display: {
    snippet: `import { Avatar, AvatarFallback, ScrollArea, Separator, Skeleton } from "@ironclaw/design-system";

<Avatar><AvatarFallback>IC</AvatarFallback></Avatar>
<Skeleton className="h-4 w-40" />
<ScrollArea className="h-48">…</ScrollArea>`,
    props: [
      ["Avatar", "AvatarImage with graceful AvatarFallback"],
      ["Separator", "orientation horizontal · vertical"],
      ["Skeleton", "size via className; shimmer respects reduced motion"],
      ["ScrollArea", "themed scrollbars over native scrolling"],
    ],
    storybook: "/?path=/docs/components-display--docs",
  },
  icons: {
    snippet: `import { Icon } from "@ironclaw/design-system";

<Icon name="spark" className="h-4 w-4 text-[var(--v2-accent-text)]" />`,
    props: [
      ["name", "glyph key (see the gallery below)"],
      ["strokeWidth", "default 1.7"],
      ["a11y", "aria-hidden; label the interactive parent"],
    ],
    storybook: "/?path=/docs/components-icon--docs",
  },
  primitives: {
    snippet: `import { EmptyPanel, SectionHeader, StatCard } from "@ironclaw/design-system";

<StatCard label="Runs today" value="128" tone="success" badgeLabel={t("status.healthy")} />
<EmptyPanel title="No automations yet" description="Connect a tool to get suggestions." />`,
    props: [
      ["StatCard", "label · value · tone · badgeLabel · detail"],
      ["SectionHeader", "title · subtitle"],
      ["EmptyPanel", "title · description · children (CTA) · boxed"],
      ["FlowList", "{ title, description }[] numbered steps"],
      ["SubLabel", "mono-caps eyebrow"],
    ],
    storybook: "/?path=/docs/components-primitives--docs",
  },
};

/* ── Copyable snippet + props summary block ─────────────────────────── */

export function UsageBlock({ doc }: { doc: UsageDoc | undefined }) {
  const [copied, setCopied] = useState(false);
  const copy = useCallback(() => {
    if (!doc) return;
    navigator.clipboard?.writeText(doc.snippet).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    });
  }, [doc]);

  if (!doc) return null;

  return (
    <div className="mb-8 grid gap-4 lg:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
      <div className="relative min-w-0">
        <pre className="h-full overflow-x-auto rounded-[var(--v2-radius-md)] border border-[var(--v2-panel-border)] bg-[var(--v2-code-bg)] px-4 py-3.5 pr-12 font-mono text-[0.75rem] leading-5 text-[var(--v2-text)]">
          {doc.snippet}
        </pre>
        <button
          type="button"
          onClick={copy}
          title="Copy snippet"
          className={cn(
            "absolute right-2.5 top-2.5 grid h-7 w-7 place-items-center rounded-[var(--v2-radius-sm)] border",
            copied
              ? "border-[var(--v2-positive-text)] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]"
              : "border-[var(--v2-panel-border)] bg-[var(--v2-surface)] text-[var(--v2-text-muted)] hover:text-[var(--v2-text-strong)]"
          )}
        >
          <Icon name={copied ? "check" : "copy"} className="h-3.5 w-3.5" />
        </button>
      </div>
      {doc.props && doc.props.length > 0 && (
        <div className="min-w-0 rounded-[var(--v2-radius-md)] border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] px-4 py-3">
          <p className="v2-tag-face mb-2 text-[0.625rem] text-[var(--v2-text-faint)]">
            Props & variants
          </p>
          <dl className="grid gap-1.5">
            {doc.props.map(([name, summary]) => (
              <div key={name} className="grid grid-cols-[minmax(6rem,auto)_minmax(0,1fr)] items-baseline gap-3">
                <dt className="font-mono text-[0.7rem] text-[var(--v2-accent-text)]">{name}</dt>
                <dd className="text-[0.75rem] leading-5 text-[var(--v2-text-muted)]">{summary}</dd>
              </div>
            ))}
          </dl>
        </div>
      )}
    </div>
  );
}
