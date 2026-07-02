/**
 * Component galleries for /playground — live renders of every
 * design-system component in each of its variants/states, plus the
 * import line an agent should copy. These render the real
 * components from static/js/design-system/, so the gallery cannot
 * drift from what pages ship.
 */
import { React, html } from "../../../lib/html.js";
import { Badge } from "../../../design-system/badge.js";
import { Button } from "../../../design-system/button.js";
import { Card, CardBody, CardFooter, CardHeader, CardLabel } from "../../../design-system/card.js";
import { Icon } from "../../../design-system/icons.js";
import { FormField, Input, Label, Select, Textarea } from "../../../design-system/input.js";
import { Modal, ModalBody, ModalFooter } from "../../../design-system/modal.js";
import {
  EmptyPanel,
  FlowList,
  SectionHeader,
  StatCard,
  SubLabel,
} from "../../../design-system/primitives.js";
import { SectionTitle } from "./token-sections.js";

/* ── Shared bits ───────────────────────────────────────────────────── */

function ImportLine({ children }) {
  return html`
    <pre
      className="mb-6 overflow-x-auto rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3 font-mono text-[0.75rem] leading-5 text-[var(--v2-text)]"
    >${children}</pre>
  `;
}

function Row({ children, className = "" }) {
  return html`<div className=${`flex flex-wrap items-center gap-3 ${className}`}>${children}</div>`;
}

function Caption({ children }) {
  return html`
    <span className="font-mono text-[0.625rem] text-[var(--v2-text-faint)]">${children}</span>
  `;
}

/* ── Button ───────────────────────────────────────────────────────── */

const BUTTON_VARIANTS = ["primary", "outline", "secondary", "ghost", "danger"];

export function ButtonSection() {
  return html`
    <div>
      <${ImportLine}>import { Button } from "../../design-system/button.js";<//>

      <${SectionTitle}>Variants<//>
      <${Row}>
        ${BUTTON_VARIANTS.map(
          (variant) => html`
            <div key=${variant} className="flex flex-col items-center gap-1.5">
              <${Button} variant=${variant}>${variant}<//>
              <${Caption}>variant="${variant}"<//>
            </div>
          `
        )}
      <//>

      <${SectionTitle}>Sizes<//>
      <${Row}>
        <${Button} variant="secondary" size="sm">sm<//>
        <${Button} variant="secondary" size="md">md (default)<//>
        <${Button} variant="secondary" size="lg">lg<//>
        <${Button} variant="secondary" size="icon" aria-label="Icon button">
          <${Icon} name="plus" className="h-4 w-4" />
        <//>
        <${Button} variant="secondary" size="icon-sm" aria-label="Small icon button">
          <${Icon} name="plus" className="h-4 w-4" />
        <//>
      <//>

      <${SectionTitle}>States<//>
      <${Row}>
        <${Button} disabled>primary disabled<//>
        <${Button} variant="secondary" disabled>secondary disabled<//>
        <${Button} fullWidth className="max-w-xs">
          <${Icon} name="send" className="h-4 w-4" /> fullWidth with icon
        <//>
      <//>
    </div>
  `;
}

/* ── Badge ────────────────────────────────────────────────────────── */

const BADGE_TONES = ["success", "warning", "danger", "info", "accent", "muted"];

export function BadgeSection() {
  return html`
    <div>
      <${ImportLine}>import { Badge } from "../../design-system/badge.js";<//>

      <${SectionTitle}>Tones<//>
      <${Row}>
        ${BADGE_TONES.map(
          (tone) => html`<${Badge} key=${tone} tone=${tone} label=${tone} />`
        )}
      <//>

      <${SectionTitle}>Sizes + no dot<//>
      <${Row}>
        <${Badge} tone="info" size="sm" label="size sm" />
        <${Badge} tone="info" size="md" label="size md" />
        <${Badge} tone="muted" dot=${false} label="dot=false" />
      <//>
      <p className="mt-4 max-w-[62ch] text-sm leading-6 text-[var(--v2-text-muted)]">
        Success/positive/signal tones render a breathing dot — the sanctioned
        "live" indicator. Pass a translated ${" "}
        <code className="font-mono text-[0.75rem]">label</code>; never rely on
        the tone keyword as user-facing copy.
      </p>
    </div>
  `;
}

/* ── Card ─────────────────────────────────────────────────────────── */

const CARD_VARIANTS = ["default", "bordered", "subtle", "inset"];

export function CardSection() {
  return html`
    <div>
      <${ImportLine}>import { Card, CardHeader, CardBody, CardFooter, CardLabel } from "../../design-system/card.js";<//>

      <${SectionTitle}>Variants<//>
      <div className="grid gap-4 sm:grid-cols-2">
        ${CARD_VARIANTS.map(
          (variant) => html`
            <${Card} key=${variant} variant=${variant} padding="md">
              <${CardLabel}>variant="${variant}"<//>
              <p className="mt-2 text-sm text-[var(--v2-text-muted)]">
                Card surface backed by themed tokens.
              </p>
            <//>
          `
        )}
      </div>

      <${SectionTitle}>Composed: header / body / footer<//>
      <${Card} className="max-w-lg">
        <${CardHeader} divider>
          <${CardLabel}>Channels<//>
          <h3 className="mt-1 text-[1.2rem] font-semibold tracking-[-0.02em] text-[var(--v2-text-strong)]">
            Slack workspace
          </h3>
        <//>
        <${CardBody}>
          <p className="text-sm leading-6 text-[var(--v2-text-muted)]">
            Route thread replies into a shared channel. Composition:
            CardHeader with divider, CardBody, CardFooter.
          </p>
        <//>
        <${CardFooter}>
          <${Button} variant="ghost" size="sm">Cancel<//>
          <${Button} size="sm">Connect<//>
        <//>
      <//>
    </div>
  `;
}

/* ── Inputs ───────────────────────────────────────────────────────── */

export function InputSection() {
  return html`
    <div>
      <${ImportLine}>import { Input, Textarea, Select, Label, FormField } from "../../design-system/input.js";<//>

      <${SectionTitle}>Input sizes + states<//>
      <div className="flex max-w-md flex-col gap-4">
        <${Input} size="sm" placeholder="size sm" />
        <${Input} placeholder="size md (default)" />
        <${Input} placeholder="disabled" disabled />
        <${Input} placeholder="error state" error />
      </div>

      <${SectionTitle}>FormField composition<//>
      <div className="flex max-w-md flex-col gap-5">
        <${FormField} label="API token" hint="Stored locally, never echoed." required>
          <${Input} placeholder="icw_..." />
        <//>
        <${FormField} label="Provider" error="Select a provider to continue.">
          <${Select} error>
            <option value="">Choose…</option>
            <option value="one">Provider One</option>
          <//>
        <//>
        <${FormField} label="Notes">
          <${Textarea} placeholder="Optional deployment notes" rows="3" />
        <//>
      </div>
    </div>
  `;
}

/* ── Modal ────────────────────────────────────────────────────────── */

export function ModalSection() {
  const [open, setOpen] = React.useState(false);
  return html`
    <div>
      <${ImportLine}>import { Modal, ModalBody, ModalFooter } from "../../design-system/modal.js";<//>

      <${SectionTitle}>Dialog<//>
      <${Button} variant="secondary" onClick=${() => setOpen(true)}>Open modal<//>
      <p className="mt-4 max-w-[62ch] text-sm leading-6 text-[var(--v2-text-muted)]">
        Backdrop click and Escape both close. Body scroll locks while open.
        Sits on the modal layer (var(--v2-z-modal)).
      </p>

      <${Modal} open=${open} onClose=${() => setOpen(false)} title="Remove extension">
        <${ModalBody}>
          <p className="text-sm leading-6 text-[var(--v2-text-muted)]">
            This removes the extension and its stored configuration. Threads
            that used it keep their history.
          </p>
        <//>
        <${ModalFooter}>
          <${Button} variant="ghost" size="sm" onClick=${() => setOpen(false)}>Cancel<//>
          <${Button} variant="danger" size="sm" onClick=${() => setOpen(false)}>Remove<//>
        <//>
      <//>
    </div>
  `;
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
  return html`
    <div>
      <${ImportLine}>import { Icon } from "../../design-system/icons.js";<//>

      <${SectionTitle}>Icon set — 24px grid, stroke 1.7<//>
      <div className="grid grid-cols-4 gap-3 sm:grid-cols-6 lg:grid-cols-8">
        ${ICON_NAMES.map(
          (name) => html`
            <div
              key=${name}
              className="flex flex-col items-center gap-2 rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] px-2 py-3"
            >
              <${Icon} name=${name} className="h-5 w-5 text-[var(--v2-text-strong)]" />
              <${Caption}>${name}<//>
            </div>
          `
        )}
      </div>
    </div>
  `;
}

/* ── Primitives ───────────────────────────────────────────────────── */

export function PrimitivesSection() {
  return html`
    <div>
      <${ImportLine}>import { StatCard, FlowList, EmptyPanel, SectionHeader, SubLabel } from "../../design-system/primitives.js";<//>

      <${SectionTitle}>SectionHeader (md+ only)<//>
      <${SectionHeader} title="Automations" subtitle="Recurring work the agent runs for you." />

      <${SectionTitle}>StatCard<//>
      <div className="grid max-w-2xl gap-0 sm:grid-cols-2">
        <${StatCard} label="Active runs" value="12" tone="success" badgeLabel="live" detail="3 waiting on approval" />
        <${StatCard} label="Failures (7d)" value="2" tone="danger" badgeLabel="attention" />
      </div>

      <${SectionTitle}>SubLabel + FlowList<//>
      <${SubLabel}>How pairing works<//>
      <div className="max-w-2xl">
        <${FlowList}
          items=${[
            { title: "Generate a code", description: "The agent issues a one-time pairing code." },
            { title: "Paste it in Slack", description: "Run /ironclaw pair in the target channel." },
            { title: "Confirm", description: "The channel appears under Extensions → Channels." },
          ]}
        />
      </div>

      <${SectionTitle}>EmptyPanel<//>
      <div className="max-w-2xl">
        <${EmptyPanel}
          title="No automations yet"
          description="Create one from any chat thread — ask the agent to do something on a schedule."
        >
          <${Button} size="sm">New automation<//>
        <//>
      </div>
    </div>
  `;
}
