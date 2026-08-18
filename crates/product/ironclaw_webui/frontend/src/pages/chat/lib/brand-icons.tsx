/**
 * Brand icons for OOBE suggestion cards.
 *
 * The backend suggestion card (PR #7694, now on `main`) carries a **required**
 * `icon` enum whose values are exactly `BRAND_ICON_IDS` below, plus `sources`
 * (concise human-readable tool names like "Gmail", for display — NOT extension
 * ids, and NOT the icon source). This module is the single icon → glyph map.
 * See docs/internal/design/oobe/SUGGESTION-ICONS.md for the enum + schema.
 *
 * `resolveIconId` trusts the authoritative `icon` field when it is a known enum
 * value, and otherwise falls back to `generic` — the enum's guaranteed-valid
 * value (a model could still emit an out-of-enum string, and the field is typed
 * optional here so the card renders even if it is ever absent).
 *
 * The colored brand marks (gmail/calendar/docs/drive/slack/notion/github/
 * telegram) are the same license-clean inline SVGs already committed in the
 * OOBE mockup; sheets/slides/web/memory/generic are neutral in-house glyphs.
 * Brand trademarks belong to their owners; these are nominative-use marks for
 * "this suggestion touches <tool>".
 */
import type { ReactNode } from "react";

export const BRAND_ICON_IDS = [
  "gmail",
  "google_calendar",
  "google_docs",
  "google_drive",
  "google_sheets",
  "google_slides",
  "github",
  "slack",
  "notion",
  "telegram",
  "web",
  "memory",
  "generic",
] as const;

export type BrandIconId = (typeof BRAND_ICON_IDS)[number];

const ICON_ID_SET = new Set<string>(BRAND_ICON_IDS);

/** The icon a suggestion should render: the backend's required `icon` enum when
 *  it is a known value, else `generic` (the guaranteed-valid fallback). */
export function resolveIconId(
  suggestion: { icon?: string | null } | null | undefined,
): BrandIconId {
  const icon = suggestion?.icon;
  return icon && ICON_ID_SET.has(icon) ? (icon as BrandIconId) : "generic";
}

// Colored brand marks (native viewBoxes) + neutral currentColor glyphs for the
// non-brand ids. Each entry is a full <svg> so multi-fill brand logos render
// faithfully; `className` controls sizing.
const GLYPHS: Record<BrandIconId, ReactNode> = {
  gmail: (
    <svg viewBox="0 0 48 48" fill="none" aria-hidden="true">
      <rect x="4" y="9" width="40" height="30" rx="4" fill="#fff" />
      <path fill="#4caf50" d="M45 16.2l-5 2.75-5 4.75L35 40h7c1.657 0 3-1.343 3-3V16.2z" />
      <path fill="#1e88e5" d="M3 16.2l3.614 1.71L13 23.7V40H6c-1.657 0-3-1.343-3-3V16.2z" />
      <polygon fill="#e53935" points="35,11.2 24,19.45 13,11.2 12,17 13,23.7 24,31.95 35,23.7 36,17" />
      <path fill="#c62828" d="M3 12.298V16.2l10 7.5V11.2L9.876 8.859C9.132 8.301 8.228 8 7.298 8 4.924 8 3 9.924 3 12.298z" />
      <path fill="#fbc02d" d="M45 12.298V16.2l-10 7.5V11.2l3.124-2.341C38.868 8.301 39.772 8 40.702 8 43.076 8 45 9.924 45 12.298z" />
    </svg>
  ),
  google_calendar: (
    <svg viewBox="0 0 48 48" fill="none" aria-hidden="true">
      <rect x="6" y="6" width="36" height="36" rx="3" fill="#fff" />
      <polygon fill="#fbc02d" points="34,42 14,42 13,38 14,34 34,34 35,38" />
      <polygon fill="#4caf50" points="38,34 38,14 42,13 46,14 46,34 42,35" />
      <path fill="#1e88e5" d="M34 14l1-4-1-4H9C7.343 6 6 7.343 6 9v25l4 1 4-1V14h20z" />
      <polygon fill="#e53935" points="34,6 34,14 42,14 42,10 38,6" />
      <path fill="#1565c0" d="M42 14h-8V6h5c1.657 0 3 1.343 3 3v5z" />
      <path fill="#2e7d32" d="M9 42h5v-8H6v5c0 1.657 1.343 3 3 3z" />
      <path fill="#1e88e5" d="M22.94 23.75c.63-.58 1.02-1.37 1.02-2.25 0-1.75-1.53-3.17-3.42-3.17-1.6 0-2.97 1.01-3.33 2.45l1.66.42c.16-.66.87-1.15 1.67-1.15.94 0 1.71.65 1.71 1.44 0 .8-.77 1.44-1.71 1.44h-1v1.71h1c1.08 0 1.97.75 1.97 1.68 0 .95-.92 1.68-2.1 1.68-1.08 0-2.01-.61-2.17-1.45l-1.66.42c.36 1.44 1.73 2.5 3.33 2.5 1.88 0 3.42-1.42 3.42-3.17 0-.88-.39-1.67-2.01-2.25zM30 18.62h-1.44l-1.87 1.3 1.01 1.44 1.58-1.15v9.35H30z" />
    </svg>
  ),
  google_docs: (
    <svg viewBox="0 0 48 48" fill="none" aria-hidden="true">
      <path fill="#2196f3" d="M37 45H11c-1.657 0-3-1.343-3-3V6c0-1.657 1.343-3 3-3h19l10 10v29c0 1.657-1.343 3-3 3z" />
      <path fill="#bbdefb" d="M40 13H30V3z" />
      <path fill="#1565c0" d="M30 13l10 10V13z" />
      <path fill="#e3f2fd" d="M15 23h18v2H15zm0 4h18v2H15zm0 4h18v2H15zm0 4h10v2H15z" />
    </svg>
  ),
  google_drive: (
    <svg viewBox="0 0 48 48" fill="none" aria-hidden="true">
      <path fill="#ffc107" d="M17 6h14l14 24H31z" />
      <path fill="#1976d2" d="M9.875 42L16.938 30H45l-7 12z" />
      <path fill="#4caf50" d="M3 30.125L9.875 42 24 18 17 6z" />
    </svg>
  ),
  google_sheets: (
    <svg viewBox="0 0 48 48" fill="none" aria-hidden="true">
      <path fill="#43a047" d="M37 45H11c-1.657 0-3-1.343-3-3V6c0-1.657 1.343-3 3-3h19l10 10v29c0 1.657-1.343 3-3 3z" />
      <path fill="#c8e6c9" d="M40 13H30V3z" />
      <path fill="#2e7d32" d="M30 13l10 10V13z" />
      <path fill="#e8f5e9" d="M15 22h18v13H15V22zm2 2v2.6h5.5V24H17zm7.5 0v2.6H31V24h-6.5zM17 28.4V31h5.5v-2.6H17zm7.5 0V31H31v-2.6h-6.5zM17 32.8V35h5.5v-2.2H17zm7.5 0V35H31v-2.2h-6.5z" />
    </svg>
  ),
  google_slides: (
    <svg viewBox="0 0 48 48" fill="none" aria-hidden="true">
      <path fill="#f4b400" d="M37 45H11c-1.657 0-3-1.343-3-3V6c0-1.657 1.343-3 3-3h19l10 10v29c0 1.657-1.343 3-3 3z" />
      <path fill="#fce8b2" d="M40 13H30V3z" />
      <path fill="#e37400" d="M30 13l10 10V13z" />
      <path fill="#fff" d="M31 22H17c-.55 0-1 .45-1 1v10c0 .55.45 1 1 1h14c.55 0 1-.45 1-1V23c0-.55-.45-1-1-1zm-1 10H18v-8h12v8z" />
    </svg>
  ),
  github: (
    <svg viewBox="0 0 98 96" fill="none" aria-hidden="true">
      <path
        fill="currentColor"
        fillRule="evenodd"
        clipRule="evenodd"
        d="M41.4395 69.3848C28.8066 67.8535 19.9062 58.7617 19.9062 46.9902C19.9062 42.2051 21.6289 37.0371 24.5 33.5918C23.2559 30.4336 23.4473 23.7344 24.8828 20.959C28.7109 20.4805 33.8789 22.4902 36.9414 25.2656C40.5781 24.1172 44.4062 23.543 49.0957 23.543C53.7852 23.543 57.6133 24.1172 61.0586 25.1699C64.0254 22.4902 69.2891 20.4805 73.1172 20.959C74.457 23.543 74.6484 30.2422 73.4043 33.4961C76.4668 37.1328 78.0937 42.0137 78.0937 46.9902C78.0937 58.7617 69.1934 67.6621 56.3691 69.2891C59.623 71.3945 61.8242 75.9883 61.8242 81.252V91.2051C61.8242 94.0762 64.2168 95.7031 67.0879 94.5547C84.4102 87.9512 98 70.6289 98 49.1914C98 22.1074 75.9883 0 48.9043 0C21.8203 0 0 22.1074 0 49.1914C0 70.4375 13.4941 88.0469 31.6777 94.6504C34.2617 95.6074 36.75 93.8848 36.75 91.3008V83.6445C35.4102 84.2188 33.6875 84.6016 32.1562 84.6016C25.8398 84.6016 22.1074 81.1563 19.4277 74.7441C18.375 72.1602 17.2266 70.6289 15.0254 70.3418C13.877 70.2461 13.4941 69.7676 13.4941 69.1934C13.4941 68.0449 15.4082 67.1836 17.3223 67.1836C20.0977 67.1836 22.4902 68.9063 24.9785 72.4473C26.8926 75.2227 28.9023 76.4668 31.2949 76.4668C33.6875 76.4668 35.2187 75.6055 37.4199 73.4043C39.0469 71.7773 40.291 70.3418 41.4395 69.3848Z"
      />
    </svg>
  ),
  slack: (
    <svg viewBox="0 0 48 48" fill="none" aria-hidden="true">
      <path fill="#33d375" d="M33 8c0-2.209-1.791-4-4-4s-4 1.791-4 4v11c0 2.209 1.791 4 4 4s4-1.791 4-4z" />
      <path fill="#33d375" d="M43 19c0 2.209-1.791 4-4 4h-4v-4c0-2.209 1.791-4 4-4s4 1.791 4 4z" />
      <path fill="#40c4ff" d="M8 14c-2.209 0-4 1.791-4 4s1.791 4 4 4h11c2.209 0 4-1.791 4-4s-1.791-4-4-4z" />
      <path fill="#40c4ff" d="M19 4c2.209 0 4 1.791 4 4v4h-4c-2.209 0-4-1.791-4-4s1.791-4 4-4z" />
      <path fill="#e91e63" d="M14 40c0 2.209 1.791 4 4 4s4-1.791 4-4V29c0-2.209-1.791-4-4-4s-4 1.791-4 4z" />
      <path fill="#e91e63" d="M4 29c0-2.209 1.791-4 4-4h4v4c0 2.209-1.791 4-4 4s-4-1.791-4-4z" />
      <path fill="#ffc107" d="M40 34c2.209 0 4-1.791 4-4s-1.791-4-4-4H29c-2.209 0-4 1.791-4 4s1.791 4 4 4z" />
      <path fill="#ffc107" d="M29 44c-2.209 0-4-1.791-4-4v-4h4c2.209 0 4 1.791 4 4s-1.791 4-4 4z" />
    </svg>
  ),
  notion: (
    <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
      <path d="M4.459 4.208c.746.606 1.026.56 2.428.466l13.215-.793c.28 0 .047-.28-.046-.326L17.86 1.968c-.42-.326-.981-.7-2.055-.607L3.01 2.295c-.466.046-.56.28-.374.466zm.793 3.08v13.904c0 .747.373 1.027 1.214.98l14.523-.84c.841-.046.935-.56.935-1.167V6.354c0-.606-.233-.933-.748-.887l-15.177.887c-.56.047-.747.327-.747.933zm14.337.745c.093.42 0 .84-.42.888l-.7.14v10.264c-.608.327-1.168.514-1.635.514-.748 0-.935-.234-1.495-.933l-4.577-7.186v6.952l1.448.327s0 .84-1.168.84l-3.222.186c-.093-.186 0-.653.327-.746l.84-.233V9.854L7.822 9.76c-.094-.42.14-1.026.793-1.073l3.456-.233 4.764 7.279v-6.44l-1.215-.139c-.093-.514.28-.887.747-.933zM3.01 1.336l13.31-.98c1.634-.14 2.055-.047 3.082.7l4.249 2.986c.7.513.934.653.934 1.213v16.378c0 1.026-.373 1.634-1.68 1.726l-15.458.933c-.98.047-1.448-.093-1.962-.746l-3.129-4.06c-.56-.747-.793-1.306-.793-1.96V3.354c0-.839.374-1.54 1.447-1.632z" />
    </svg>
  ),
  telegram: (
    <svg viewBox="0 0 48 48" fill="none" aria-hidden="true">
      <circle cx="24" cy="24" r="22" fill="#29a9eb" />
      <path fill="#fff" d="M35.24 14.28c.42-.1.9.12.6 1.3l-4.02 18.92c-.3 1.32-1.1 1.63-2.1 1.02L23 31.5l-3.28 3.16c-.36.36-.67.66-1.37.66l.5-6.9 12.43-11.22c.54-.48-.12-.6-.83-.16L16.62 24l-6.42-2c-1.4-.44-1.42-1.4.3-2.07z" />
    </svg>
  ),
  web: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" aria-hidden="true">
      <circle cx="12" cy="12" r="9" />
      <path d="M3 12h18M12 3a14 14 0 0 1 0 18M12 3a14 14 0 0 0 0 18" />
    </svg>
  ),
  memory: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" aria-hidden="true">
      <ellipse cx="12" cy="6" rx="7" ry="3" />
      <path d="M5 6v12c0 1.657 3.134 3 7 3s7-1.343 7-3V6M5 12c0 1.657 3.134 3 7 3s7-1.343 7-3" />
    </svg>
  ),
  generic: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M12 4.5l1.8 4.2 4.7.4-3.6 3 1.1 4.6L12 14.9l-4 2.4 1.1-4.6-3.6-3 4.7-.4z" />
    </svg>
  ),
};

/** Render one brand/tool glyph by icon id (falls back to `generic`). */
export function BrandIcon({ id, className }: { id: BrandIconId; className?: string }) {
  const glyph = GLYPHS[id] ?? GLYPHS.generic;
  return <span className={className}>{glyph}</span>;
}
