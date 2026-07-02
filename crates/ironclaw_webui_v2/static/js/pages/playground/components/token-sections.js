/**
 * Token pages for /playground — colors, typography, spacing, radii,
 * shadows, motion, and z-index. Values are read live from the CSS
 * custom properties so the swatches always show what the active
 * theme actually resolves, not a stale copy.
 */
import { React, html } from "../../../lib/html.js";
import {
  COLOR_TOKENS,
  MOTION_TOKENS,
  RADIUS_TOKENS,
  SHADOW_TOKENS,
  SPACE_TOKENS,
  TYPE_TOKENS,
  Z_TOKENS,
  readToken,
} from "../../../design-system/tokens.js";

/* ── Shared bits ───────────────────────────────────────────────────── */

export function SectionTitle({ children }) {
  return html`
    <h3
      className="mb-3 mt-8 font-mono text-[0.6875rem] font-semibold uppercase tracking-[0.14em] text-[var(--v2-text-muted)] first:mt-0"
    >
      ${children}
    </h3>
  `;
}

function TokenName({ children }) {
  return html`
    <span className="font-mono text-[0.6875rem] text-[var(--v2-text-strong)]">${children}</span>
  `;
}

function TokenValue({ children }) {
  return html`
    <span className="truncate font-mono text-[0.625rem] text-[var(--v2-text-faint)]">${children}</span>
  `;
}

function useCopy() {
  const [copied, setCopied] = React.useState(null);
  const copy = React.useCallback((value) => {
    try {
      navigator.clipboard?.writeText(value);
    } catch (_) {}
    setCopied(value);
    window.setTimeout(() => setCopied(null), 1200);
  }, []);
  return { copied, copy };
}

/* ── Colors ───────────────────────────────────────────────────────── */

function Swatch({ name, value, copied, onCopy }) {
  return html`
    <button
      type="button"
      onClick=${onCopy}
      title=${`Copy ${name}`}
      className="group flex flex-col gap-1.5 text-left"
    >
      <span
        className="block h-14 w-full rounded-[10px] border border-[var(--v2-panel-border)]"
        style=${{ background: value || "transparent" }}
      />
      <${TokenName}>${name}<//>
      <${TokenValue}>${copied ? "copied" : value || "—"}<//>
    </button>
  `;
}

export function ColorsSection({ theme }) {
  const { copied, copy } = useCopy();
  return html`
    <div>
      ${COLOR_TOKENS.map(
        (group) => html`
          <div key=${group.group}>
            <${SectionTitle}>${group.group}<//>
            <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
              ${group.tokens.map((token) => {
                const value = readToken(token.var);
                return html`
                  <div key=${token.var + theme} className="flex flex-col gap-1">
                    <${Swatch}
                      name=${token.var}
                      value=${value}
                      copied=${copied === token.var}
                      onCopy=${() => copy(`var(${token.var})`)}
                    />
                    <span className="text-[0.6875rem] leading-4 text-[var(--v2-text-muted)]">
                      ${token.note}
                    </span>
                  </div>
                `;
              })}
            </div>
          </div>
        `
      )}
    </div>
  `;
}

/* ── Typography ───────────────────────────────────────────────────── */

export function TypographySection({ theme }) {
  return html`
    <div>
      <${SectionTitle}>Type scale — Geist (sans) / Geist Mono<//>
      <div className="flex flex-col gap-6">
        ${TYPE_TOKENS.map((token) => {
          const value = readToken(token.var);
          return html`
            <div key=${token.var + theme} className="flex flex-col gap-1">
              <div className="flex items-baseline gap-3">
                <${TokenName}>${token.var}<//>
                <${TokenValue}>${value} · ${token.note}<//>
              </div>
              <span
                className="text-[var(--v2-text-strong)]"
                style=${{ fontSize: `var(${token.var})` }}
              >
                ${token.sample}
              </span>
            </div>
          `;
        })}
      </div>

      <${SectionTitle}>Letter spacing<//>
      <div className="flex flex-col gap-3">
        <span
          className="font-mono text-[0.6875rem] uppercase text-[var(--v2-text-muted)]"
          style=${{ letterSpacing: "var(--v2-tracking-caps)" }}
        >
          --v2-tracking-caps — mono-caps labels
        </span>
        <span
          className="font-mono text-[0.6875rem] uppercase text-[var(--v2-text-faint)]"
          style=${{ letterSpacing: "var(--v2-tracking-wide)" }}
        >
          --v2-tracking-wide — card eyebrows
        </span>
        <span
          className="text-[1.35rem] font-medium text-[var(--v2-text-strong)]"
          style=${{ letterSpacing: "var(--v2-tracking-tight)" }}
        >
          --v2-tracking-tight — titles
        </span>
        <span
          className="text-[2.2rem] font-medium text-[var(--v2-text-strong)]"
          style=${{ letterSpacing: "var(--v2-tracking-display)" }}
        >
          --v2-tracking-display — display headings
        </span>
      </div>

      <${SectionTitle}>Weights<//>
      <div className="flex flex-wrap gap-x-8 gap-y-2 text-xl text-[var(--v2-text-strong)]">
        <span className="font-normal">Regular 400</span>
        <span className="font-medium">Medium 500</span>
        <span className="font-semibold">Semibold 600</span>
      </div>
    </div>
  `;
}

/* ── Spacing ──────────────────────────────────────────────────────── */

export function SpacingSection({ theme }) {
  return html`
    <div>
      <${SectionTitle}>Spacing scale — 4px base grid<//>
      <div className="flex flex-col gap-3">
        ${SPACE_TOKENS.map((token) => {
          const value = readToken(token.var);
          return html`
            <div key=${token.var + theme} className="flex items-center gap-4">
              <span className="w-40 shrink-0 font-mono text-[0.6875rem] text-[var(--v2-text-strong)]">
                ${token.var}
              </span>
              <span
                className="h-4 shrink-0 rounded-[3px] bg-[var(--v2-accent)]"
                style=${{ width: `var(${token.var})` }}
              />
              <${TokenValue}>${value} — ${token.note}<//>
            </div>
          `;
        })}
      </div>
    </div>
  `;
}

/* ── Radii & shadows ──────────────────────────────────────────────── */

export function RadiiSection({ theme }) {
  return html`
    <div>
      <${SectionTitle}>Radius scale<//>
      <div className="flex flex-wrap items-end gap-5">
        ${RADIUS_TOKENS.map((token) => {
          const value = readToken(token.var);
          return html`
            <div key=${token.var + theme} className="flex w-28 flex-col items-center gap-2 text-center">
              <div
                className="h-20 w-20 border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)]"
                style=${{ borderRadius: `var(${token.var})` }}
              />
              <${TokenName}>${token.var.replace("--v2-radius-", "")}<//>
              <${TokenValue}>${value}<//>
              <span className="text-[0.625rem] leading-4 text-[var(--v2-text-muted)]">${token.note}</span>
            </div>
          `;
        })}
      </div>

      <${SectionTitle}>Shadows<//>
      <div className="flex flex-wrap gap-6">
        ${SHADOW_TOKENS.map((token) => {
          const value = readToken(token.var);
          return html`
            <div key=${token.var + theme} className="flex w-52 flex-col gap-2">
              <div
                className="h-24 rounded-[1.25rem] border border-[var(--v2-card-border)] bg-[var(--v2-card-bg)]"
                style=${{ boxShadow: `var(${token.var})` }}
              />
              <${TokenName}>${token.var}<//>
              <${TokenValue}>${value || "none"}<//>
              <span className="text-[0.625rem] leading-4 text-[var(--v2-text-muted)]">${token.note}</span>
            </div>
          `;
        })}
      </div>
    </div>
  `;
}

/* ── Motion ───────────────────────────────────────────────────────── */

export function MotionSection({ theme }) {
  return html`
    <div>
      <${SectionTitle}>Static-motion policy<//>
      <p className="max-w-[62ch] text-sm leading-6 text-[var(--v2-text-muted)]">
        The v2 UI is intentionally static: app.css disables all CSS
        transitions and animations globally. The three loops below are the
        only sanctioned exceptions — they signal live work, and each is
        suppressed under ${" "}
        <code className="font-mono text-[0.75rem]">prefers-reduced-motion</code>.
        Do not add new animation without updating the policy in app.css and
        DESIGN_SYSTEM.md.
      </p>

      <${SectionTitle}>Sanctioned exceptions<//>
      <div className="flex flex-col gap-5">
        <div className="flex items-center gap-4">
          <span className="flex w-44 items-center gap-1 rounded-full border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 py-2">
            <span className="v2-typing-dot h-1.5 w-1.5 rounded-full bg-[var(--v2-text-muted)]" />
            <span className="v2-typing-dot h-1.5 w-1.5 rounded-full bg-[var(--v2-text-muted)]" />
            <span className="v2-typing-dot h-1.5 w-1.5 rounded-full bg-[var(--v2-text-muted)]" />
          </span>
          <${TokenValue}>.v2-typing-dot — var(--v2-duration-typing) = ${readToken("--v2-duration-typing")}<//>
        </div>
        <div className="flex items-center gap-4">
          <span className="grid w-44 place-items-center py-1">
            <span
              className="v2-spin h-5 w-5 rounded-full border-2 border-[var(--v2-panel-border)] border-t-[var(--v2-accent)]"
            />
          </span>
          <${TokenValue}>.v2-spin — var(--v2-duration-spin) = ${readToken("--v2-duration-spin")}<//>
        </div>
        <div className="flex items-center gap-4">
          <span className="flex w-44 items-center justify-center gap-2 py-1">
            <span className="h-2 w-2 animate-[v2-breathe_2s_ease-in-out_infinite] rounded-full bg-[var(--v2-positive-text)]" />
            <span className="font-mono text-[0.625rem] uppercase text-[var(--v2-positive-text)]">live</span>
          </span>
          <${TokenValue}>v2-breathe — var(--v2-duration-breathe) = ${readToken("--v2-duration-breathe")}<//>
        </div>
      </div>
      <span className="hidden">${theme}</span>
    </div>
  `;
}

/* ── Z-index ──────────────────────────────────────────────────────── */

export function ZIndexSection({ theme }) {
  return html`
    <div>
      <${SectionTitle}>Layer ladder<//>
      <div className="flex flex-col gap-2">
        ${Z_TOKENS.slice()
          .reverse()
          .map((token) => {
            const value = readToken(token.var);
            return html`
              <div
                key=${token.var + theme}
                className="flex items-center gap-4 rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] px-4 py-3"
              >
                <span className="w-10 text-right font-mono text-[0.9rem] font-semibold text-[var(--v2-accent-text)]">
                  ${value}
                </span>
                <${TokenName}>${token.var}<//>
                <span className="text-[0.75rem] text-[var(--v2-text-muted)]">${token.note}</span>
              </div>
            `;
          })}
      </div>
      <p className="mt-5 max-w-[62ch] text-sm leading-6 text-[var(--v2-text-muted)]">
        Use Tailwind classes ${" "}
        <code className="font-mono text-[0.75rem]">z-10 / z-20 / z-40 / z-50 / z-[60]</code>${" "}
        matching this ladder. Never introduce a new raw z-index value.
      </p>
    </div>
  `;
}
