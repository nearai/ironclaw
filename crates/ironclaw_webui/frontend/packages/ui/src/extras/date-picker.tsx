/**
 * Calendar + DatePicker
 *
 * Lightweight month-grid date selection with plain Date math — no date
 * library. Two exports:
 *
 *   Calendar    — inline month grid (role="grid") with full keyboard support:
 *                 arrows move by day/week, PageUp/PageDown by month, Home/End
 *                 to week edges, Enter/Space selects. Month/weekday names come
 *                 from Intl so the locale prop localizes everything.
 *   DatePicker  — Input-styled trigger that opens a Calendar in a popover
 *                 panel (Escape / outside click / selection closes it).
 *
 * Scope: single-date selection with optional min/max bounds. Ranges and
 * multi-month views are intentionally out of scope for the extras kit.
 */
import React from "react";
import { cn } from "../primitives/cn";
import { Icon } from "../primitives/icon";
import { OVERLAY_SURFACE_CLASSES } from "./overlay";

/* ── Date math (all local-time, day precision) ─────────────────────── */

function startOfDay(date: Date): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

function isSameDay(a: Date | null | undefined, b: Date): boolean {
  return (
    !!a &&
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  );
}

function addDays(date: Date, days: number): Date {
  const next = new Date(date);
  next.setDate(next.getDate() + days);
  return next;
}

function addMonths(date: Date, months: number): Date {
  // Clamp to the last day of the target month (Jan 31 + 1mo -> Feb 28/29).
  const day = date.getDate();
  const next = new Date(date.getFullYear(), date.getMonth() + months, 1);
  const lastDay = new Date(next.getFullYear(), next.getMonth() + 1, 0).getDate();
  next.setDate(Math.min(day, lastDay));
  return next;
}

function isOutOfRange(date: Date, min?: Date, max?: Date): boolean {
  if (min && startOfDay(date) < startOfDay(min)) return true;
  if (max && startOfDay(date) > startOfDay(max)) return true;
  return false;
}

function isoDate(date: Date): string {
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${date.getFullYear()}-${month}-${day}`;
}

/** All grid cells for a month view: leading/trailing days pad full weeks. */
function monthGrid(month: Date, weekStartsOn: 0 | 1): Date[] {
  const first = new Date(month.getFullYear(), month.getMonth(), 1);
  const lead = (first.getDay() - weekStartsOn + 7) % 7;
  const start = addDays(first, -lead);
  const daysInMonth = new Date(month.getFullYear(), month.getMonth() + 1, 0).getDate();
  const weeks = Math.ceil((lead + daysInMonth) / 7);
  return Array.from({ length: weeks * 7 }, (_cell, index) => addDays(start, index));
}

/* ── Calendar ──────────────────────────────────────────────────────── */

type CalendarProps = {
  /** Selected day (day precision; time-of-day is ignored). */
  value?: Date | null;
  onChange?: (date: Date) => void;
  /** Month shown when first rendered (defaults to value ?? today). */
  defaultMonth?: Date;
  minDate?: Date;
  maxDate?: Date;
  /** 0 = Sunday (default), 1 = Monday. */
  weekStartsOn?: 0 | 1;
  /** BCP-47 locale for month/weekday names; defaults to the browser locale. */
  locale?: string;
  className?: string;
};

export function Calendar({
  value = null,
  onChange,
  defaultMonth,
  minDate,
  maxDate,
  weekStartsOn = 0,
  locale,
  className,
}: CalendarProps) {
  const initial = startOfDay(value ?? defaultMonth ?? new Date());
  const [month, setMonth] = React.useState(
    () => new Date(initial.getFullYear(), initial.getMonth(), 1)
  );
  const [focusedDate, setFocusedDate] = React.useState(initial);
  const gridRef = React.useRef<HTMLTableElement | null>(null);
  const pendingFocusRef = React.useRef(false);

  const monthFormatter = React.useMemo(
    () => new Intl.DateTimeFormat(locale, { month: "long", year: "numeric" }),
    [locale]
  );
  const weekdayFormatter = React.useMemo(
    () => new Intl.DateTimeFormat(locale, { weekday: "short" }),
    [locale]
  );
  const dayLabelFormatter = React.useMemo(
    () => new Intl.DateTimeFormat(locale, { dateStyle: "full" }),
    [locale]
  );

  // After a keyboard move, put focus on the (possibly re-rendered) day button.
  React.useEffect(() => {
    if (!pendingFocusRef.current) return;
    pendingFocusRef.current = false;
    const target = gridRef.current?.querySelector<HTMLButtonElement>(
      `button[data-date="${isoDate(focusedDate)}"]`
    );
    target?.focus();
  }, [focusedDate, month]);

  const today = startOfDay(new Date());
  const cells = monthGrid(month, weekStartsOn);
  const weekdays = Array.from({ length: 7 }, (_day, index) =>
    // Jan 4 1970 was a Sunday; offset gives a stable week of sample dates.
    weekdayFormatter.format(new Date(1970, 0, 4 + weekStartsOn + index))
  );

  const moveFocus = (next: Date) => {
    if (isOutOfRange(next, minDate, maxDate)) return;
    pendingFocusRef.current = true;
    setFocusedDate(startOfDay(next));
    if (next.getMonth() !== month.getMonth() || next.getFullYear() !== month.getFullYear()) {
      setMonth(new Date(next.getFullYear(), next.getMonth(), 1));
    }
  };

  const handleDayKeyDown = (event: React.KeyboardEvent<HTMLButtonElement>, day: Date) => {
    const moves: Record<string, () => void> = {
      ArrowLeft: () => moveFocus(addDays(day, -1)),
      ArrowRight: () => moveFocus(addDays(day, 1)),
      ArrowUp: () => moveFocus(addDays(day, -7)),
      ArrowDown: () => moveFocus(addDays(day, 7)),
      PageUp: () => moveFocus(addMonths(day, -1)),
      PageDown: () => moveFocus(addMonths(day, 1)),
      Home: () => moveFocus(addDays(day, -((day.getDay() - weekStartsOn + 7) % 7))),
      End: () => moveFocus(addDays(day, 6 - ((day.getDay() - weekStartsOn + 7) % 7))),
    };
    const move = moves[event.key];
    if (move) {
      event.preventDefault();
      move();
    }
  };

  const navButtonClasses = cn(
    "grid h-7 w-7 place-items-center rounded-[8px] border border-[var(--v2-panel-border)]",
    "bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)] transition-colors",
    "hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]",
    "hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",
    "active:bg-[color-mix(in_srgb,var(--v2-text-strong)_10%,var(--v2-surface-muted))]",
    "focus-visible:outline-none focus-visible:ring-2",
    "focus-visible:ring-[var(--v2-focus-ring)]"
  );

  return (
    <div className={cn("inline-block select-none font-sans", className)}>
      <div className="mb-2 flex items-center justify-between gap-2">
        <button
          type="button"
          aria-label="Previous month"
          onClick={() => setMonth(addMonths(month, -1))}
          className={navButtonClasses}
        >
          <Icon name="chevron" className="h-3.5 w-3.5 rotate-90" />
        </button>
        <div aria-live="polite" className="text-ui font-semibold text-[var(--v2-text-strong)]">
          {monthFormatter.format(month)}
        </div>
        <button
          type="button"
          aria-label="Next month"
          onClick={() => setMonth(addMonths(month, 1))}
          className={navButtonClasses}
        >
          <Icon name="chevron" className="h-3.5 w-3.5 -rotate-90" />
        </button>
      </div>

      <table ref={gridRef} role="grid" className="border-collapse">
        <thead>
          <tr>
            {weekdays.map((weekday) => (
              <th
                key={weekday}
                scope="col"
                className="h-8 w-9 text-center text-ui-sm font-medium text-[var(--v2-text-faint)]"
              >
                {weekday}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {Array.from({ length: cells.length / 7 }, (_week, weekIndex) => (
            <tr key={weekIndex}>
              {cells.slice(weekIndex * 7, weekIndex * 7 + 7).map((day) => {
                const inMonth = day.getMonth() === month.getMonth();
                const selected = isSameDay(value, day);
                const isToday = isSameDay(today, day);
                const disabled = isOutOfRange(day, minDate, maxDate);
                const focusable = isSameDay(focusedDate, day);
                return (
                  <td key={isoDate(day)} role="gridcell" aria-selected={selected || undefined}>
                    <button
                      type="button"
                      data-date={isoDate(day)}
                      tabIndex={focusable ? 0 : -1}
                      disabled={disabled}
                      aria-label={dayLabelFormatter.format(day)}
                      aria-current={isToday ? "date" : undefined}
                      onFocus={() => setFocusedDate(startOfDay(day))}
                      onKeyDown={(event) => handleDayKeyDown(event, day)}
                      onClick={() => onChange?.(startOfDay(day))}
                      className={cn(
                        "grid h-8 w-9 place-items-center rounded-[8px] text-ui transition-colors",
                        inMonth ? "text-[var(--v2-text)]" : "text-[var(--v2-text-faint)]",
                        "hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)]",
                        !selected &&
                          "active:bg-[color-mix(in_srgb,var(--v2-text-strong)_10%,var(--v2-surface-muted))]",
                        "focus-visible:outline-none focus-visible:ring-2",
                        "focus-visible:ring-[var(--v2-focus-ring)]",
                        "disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-transparent",
                        isToday && !selected &&
                          "border border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))]",
                        selected &&
                          "bg-[var(--v2-accent)] font-semibold text-[var(--v2-inverse)] hover:bg-[var(--v2-accent-strong)] hover:text-[var(--v2-inverse)] active:bg-[var(--v2-accent-strong)]"
                      )}
                    >
                      {day.getDate()}
                    </button>
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/* ── DatePicker ────────────────────────────────────────────────────── */

type DatePickerProps = {
  value?: Date | null;
  onChange?: (date: Date) => void;
  placeholder?: string;
  minDate?: Date;
  maxDate?: Date;
  weekStartsOn?: 0 | 1;
  locale?: string;
  disabled?: boolean;
  "aria-label"?: string;
  className?: string;
};

export function DatePicker({
  value = null,
  onChange,
  placeholder = "Pick a date",
  minDate,
  maxDate,
  weekStartsOn = 0,
  locale,
  disabled = false,
  "aria-label": ariaLabel,
  className,
}: DatePickerProps) {
  const [open, setOpen] = React.useState(false);
  const rootRef = React.useRef<HTMLDivElement | null>(null);
  const triggerRef = React.useRef<HTMLButtonElement | null>(null);

  const formatter = React.useMemo(
    () => new Intl.DateTimeFormat(locale, { dateStyle: "medium" }),
    [locale]
  );

  React.useEffect(() => {
    if (!open) return;
    const handleMouseDown = (event: MouseEvent) => {
      if (!rootRef.current?.contains(event.target as Node | null)) setOpen(false);
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setOpen(false);
        triggerRef.current?.focus();
      }
    };
    document.addEventListener("mousedown", handleMouseDown);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("mousedown", handleMouseDown);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [open]);

  return (
    <div ref={rootRef} className={cn("relative inline-block", className)}>
      <button
        ref={triggerRef}
        type="button"
        disabled={disabled}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={ariaLabel}
        onClick={() => setOpen((current) => !current)}
        className={cn(
          "inline-flex h-9 min-w-[12rem] items-center gap-2 rounded-[10px] border px-2.5 text-ui",
          "border-[var(--v2-panel-border)] bg-[var(--v2-input-bg)] text-left transition-colors",
          value ? "text-[var(--v2-text-strong)]" : "text-[var(--v2-text-faint)]",
          "hover:bg-[var(--v2-surface-soft)]",
          "hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",
          "active:bg-[var(--v2-surface-muted)]",
          "focus-visible:outline-none focus-visible:ring-2",
          "focus-visible:ring-[var(--v2-focus-ring)]",
          "disabled:cursor-not-allowed disabled:opacity-50",
          "disabled:hover:border-[var(--v2-panel-border)] disabled:hover:bg-[var(--v2-input-bg)]"
        )}
      >
        <Icon name="calendar" className="h-4 w-4 shrink-0 text-[var(--v2-text-faint)]" />
        <span className="truncate">{value ? formatter.format(value) : placeholder}</span>
      </button>

      {open && (
        <div
          role="dialog"
          aria-label={ariaLabel ?? placeholder}
          className={cn(OVERLAY_SURFACE_CLASSES, "absolute left-0 top-[calc(100%+0.35rem)] p-3")}
        >
          <Calendar
            value={value}
            minDate={minDate}
            maxDate={maxDate}
            weekStartsOn={weekStartsOn}
            locale={locale}
            onChange={(date) => {
              onChange?.(date);
              setOpen(false);
              triggerRef.current?.focus();
            }}
          />
        </div>
      )}
    </div>
  );
}
