/**
 * Command
 *
 * cmdk-style command palette, hand-built on the package's listbox semantics
 * (no cmdk dep). Compositional API mirroring shadcn:
 *
 *   <Command>
 *     <CommandInput placeholder="Type a command…" />
 *     <CommandList>
 *       <CommandEmpty>No results</CommandEmpty>
 *       <CommandGroup heading="Actions">
 *         <CommandItem value="new-run" onSelect={…}>New run</CommandItem>
 *       </CommandGroup>
 *     </CommandList>
 *   </Command>
 *
 * How it works: CommandItems register themselves (value + keywords + onSelect)
 * with the root context in mount order. The root filters that registry against
 * the query; items outside the match set render nothing, groups with no
 * visible items hide themselves, and CommandEmpty shows when the match set is
 * empty. The input is the combobox (aria-activedescendant navigation); the
 * list is a listbox with option rows. ArrowUp/Down move, Enter selects.
 *
 * CommandDialog wraps a Command in a centered overlay for the classic ⌘K
 * experience (Escape or backdrop click to close).
 */
import React, { type ReactNode } from "react";
import { cn } from "../primitives/cn";
import { Icon } from "../primitives/icon";
import { MENU_SHORTCUT_CLASSES, OVERLAY_SURFACE_CLASSES } from "../primitives/overlay";

/* ── Registry plumbing ─────────────────────────────────────────────── */

type CommandItemRecord = {
  id: string;
  value: string;
  keywords: string;
  groupId: string | null;
  disabled: boolean;
  onSelect: (() => void) | null;
};

type CommandContextValue = {
  listId: string;
  query: string;
  setQuery: (query: string) => void;
  register: (record: CommandItemRecord) => () => void;
  visibleIds: string[];
  activeId: string | null;
  setActiveId: (id: string) => void;
  moveActive: (direction: 1 | -1) => void;
  selectActive: () => void;
  selectItem: (id: string) => void;
  isGroupVisible: (groupId: string) => boolean;
};

const CommandContext = React.createContext<CommandContextValue | null>(null);
const CommandGroupContext = React.createContext<string | null>(null);

function useCommandContext(component: string): CommandContextValue {
  const context = React.useContext(CommandContext);
  if (!context) {
    throw new Error(`${component} must be rendered inside <Command>`);
  }
  return context;
}

function matches(record: CommandItemRecord, query: string): boolean {
  if (!query) return true;
  const haystack = `${record.value} ${record.keywords}`.toLowerCase();
  return query
    .toLowerCase()
    .split(/\s+/)
    .every((word) => haystack.includes(word));
}

/* ── Root ──────────────────────────────────────────────────────────── */

type CommandProps = {
  children?: ReactNode;
  className?: string;
  /** Accessible name for the palette's listbox. */
  label?: string;
};

export function Command({ children, className, label = "Command menu" }: CommandProps) {
  const listId = React.useId();
  const [query, setQueryState] = React.useState("");
  const [activeId, setActiveId] = React.useState<string | null>(null);
  const registryRef = React.useRef<CommandItemRecord[]>([]);
  const [registryVersion, setRegistryVersion] = React.useState(0);

  const register = React.useCallback((record: CommandItemRecord) => {
    registryRef.current = [...registryRef.current, record];
    setRegistryVersion((version) => version + 1);
    return () => {
      registryRef.current = registryRef.current.filter((entry) => entry.id !== record.id);
      setRegistryVersion((version) => version + 1);
    };
  }, []);

  const visible = React.useMemo(
    () => registryRef.current.filter((record) => !record.disabled && matches(record, query)),
    // registryVersion invalidates the memo when items mount/unmount.
    [query, registryVersion]
  );
  const visibleIds = React.useMemo(() => visible.map((record) => record.id), [visible]);

  // Keep the active row inside the visible set as the query changes.
  React.useEffect(() => {
    if (activeId === null || !visibleIds.includes(activeId)) {
      setActiveId(visibleIds[0] ?? null);
    }
  }, [visibleIds, activeId]);

  const setQuery = React.useCallback((next: string) => {
    setQueryState(next);
  }, []);

  const moveActive = React.useCallback(
    (direction: 1 | -1) => {
      if (visibleIds.length === 0) return;
      const currentIndex = activeId ? visibleIds.indexOf(activeId) : -1;
      const nextIndex =
        (currentIndex + direction + visibleIds.length) % visibleIds.length;
      setActiveId(visibleIds[nextIndex]);
    },
    [visibleIds, activeId]
  );

  const selectItem = React.useCallback((id: string) => {
    const record = registryRef.current.find((entry) => entry.id === id);
    if (record && !record.disabled) record.onSelect?.();
  }, []);

  const selectActive = React.useCallback(() => {
    if (activeId) selectItem(activeId);
  }, [activeId, selectItem]);

  const isGroupVisible = React.useCallback(
    (groupId: string) => visible.some((record) => record.groupId === groupId),
    [visible]
  );

  const contextValue = React.useMemo<CommandContextValue>(
    () => ({
      listId,
      query,
      setQuery,
      register,
      visibleIds,
      activeId,
      setActiveId,
      moveActive,
      selectActive,
      selectItem,
      isGroupVisible,
    }),
    [
      listId,
      query,
      setQuery,
      register,
      visibleIds,
      activeId,
      moveActive,
      selectActive,
      selectItem,
      isGroupVisible,
    ]
  );

  return (
    <CommandContext.Provider value={contextValue}>
      <div
        aria-label={label}
        className={cn(
          "flex w-full flex-col overflow-hidden rounded-[12px]",
          "border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)]",
          className
        )}
      >
        {children}
      </div>
    </CommandContext.Provider>
  );
}

/* ── Input ─────────────────────────────────────────────────────────── */

type CommandInputProps = {
  placeholder?: string;
  className?: string;
  "aria-label"?: string;
};

export function CommandInput({
  placeholder = "Type to search…",
  className,
  "aria-label": ariaLabel = "Search commands",
}: CommandInputProps) {
  const context = useCommandContext("CommandInput");
  return (
    <div className="flex items-center gap-2 border-b border-[var(--v2-panel-border)] px-3.5 py-3">
      <Icon name="search" className="h-4 w-4 shrink-0 text-[var(--v2-text-faint)]" />
      <input
        role="combobox"
        aria-expanded="true"
        aria-controls={context.listId}
        aria-activedescendant={context.activeId ?? undefined}
        aria-autocomplete="list"
        aria-label={ariaLabel}
        autoFocus
        value={context.query}
        placeholder={placeholder}
        onChange={(event) => context.setQuery(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            context.moveActive(event.key === "ArrowDown" ? 1 : -1);
            return;
          }
          if (event.key === "Enter") {
            event.preventDefault();
            context.selectActive();
          }
        }}
        className={cn(
          "w-full rounded-[4px] bg-transparent text-ui text-[var(--v2-text-strong)] outline-none",
          "focus-visible:ring-2 focus-visible:ring-[var(--v2-focus-ring)]",
          "placeholder:text-[var(--v2-text-faint)]",
          className
        )}
      />
    </div>
  );
}

/* ── List / group / item ───────────────────────────────────────────── */

type CommandSectionProps = {
  children?: ReactNode;
  className?: string;
};

export function CommandList({ children, className }: CommandSectionProps) {
  const context = useCommandContext("CommandList");
  return (
    <div
      id={context.listId}
      role="listbox"
      aria-label="Commands"
      className={cn("max-h-72 overflow-y-auto p-1.5", className)}
    >
      {children}
    </div>
  );
}

export function CommandEmpty({ children, className }: CommandSectionProps) {
  const context = useCommandContext("CommandEmpty");
  if (context.visibleIds.length > 0) return null;
  return (
    <div className={cn("px-3 py-8 text-center text-ui-sm text-[var(--v2-text-faint)]", className)}>
      {children}
    </div>
  );
}

type CommandGroupProps = CommandSectionProps & {
  heading?: ReactNode;
};

export function CommandGroup({ heading, children, className }: CommandGroupProps) {
  const context = useCommandContext("CommandGroup");
  const groupId = React.useId();
  const headingId = `${groupId}-heading`;
  const visible = context.isGroupVisible(groupId);
  return (
    <CommandGroupContext.Provider value={groupId}>
      <div
        role="group"
        aria-labelledby={heading ? headingId : undefined}
        className={cn(!visible && "hidden", className)}
      >
        {heading && (
          <div id={headingId} className="px-2.5 pb-1 pt-2 text-ui-sm font-medium text-[var(--v2-text-faint)]">
            {heading}
          </div>
        )}
        {children}
      </div>
    </CommandGroupContext.Provider>
  );
}

export function CommandSeparator({ className }: { className?: string }) {
  return <div aria-hidden="true" className={cn("-mx-1.5 my-1 h-px bg-[var(--v2-panel-border)]", className)} />;
}

type CommandItemProps = {
  /** Text the filter matches against; also passed to onSelect. */
  value: string;
  /** Extra search terms that should match this item. */
  keywords?: string;
  disabled?: boolean;
  onSelect?: (value: string) => void;
  children?: ReactNode;
  className?: string;
};

export function CommandItem({
  value,
  keywords = "",
  disabled = false,
  onSelect,
  children,
  className,
}: CommandItemProps) {
  const context = useCommandContext("CommandItem");
  const groupId = React.useContext(CommandGroupContext);
  const id = React.useId();
  const onSelectRef = React.useRef(onSelect);
  onSelectRef.current = onSelect;

  React.useEffect(() => {
    return context.register({
      id,
      value,
      keywords,
      groupId,
      disabled,
      onSelect: () => onSelectRef.current?.(value),
    });
  }, [context.register, id, value, keywords, groupId, disabled]);

  const isVisible = context.visibleIds.includes(id);
  if (!isVisible) return null;

  const isActive = context.activeId === id;
  return (
    <div
      id={id}
      role="option"
      aria-selected={isActive}
      aria-disabled={disabled || undefined}
      onMouseEnter={() => !disabled && context.setActiveId(id)}
      onMouseDown={(event) => event.preventDefault()}
      onClick={() => !disabled && context.selectItem(id)}
      className={cn(
        "flex cursor-default select-none items-center gap-2 rounded-[7px] px-2.5 py-2",
        "text-ui text-[var(--v2-text)] transition-colors",
        isActive && "bg-[var(--v2-surface-muted)] text-[var(--v2-text-strong)]",
        disabled && "pointer-events-none opacity-50",
        className
      )}
    >
      {children}
    </div>
  );
}

export function CommandShortcut({ children, className }: CommandSectionProps) {
  return <span className={cn(MENU_SHORTCUT_CLASSES, className)}>{children}</span>;
}

/* ── Dialog host ───────────────────────────────────────────────────── */

type CommandDialogProps = {
  open: boolean;
  onClose: () => void;
  children?: ReactNode;
  /** Dialog aria-label; defaults to "Command menu". */
  label?: string;
  className?: string;
};

export function CommandDialog({
  open,
  onClose,
  children,
  label = "Command menu",
  className,
}: CommandDialogProps) {
  React.useEffect(() => {
    if (!open) return;
    const handler = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center p-4 pt-[15dvh]">
      <div
        className="absolute inset-0 bg-black/55 backdrop-blur-sm"
        onClick={onClose}
        aria-hidden="true"
      />
      <div
        role="dialog"
        aria-modal="true"
        aria-label={label}
        className={cn(
          OVERLAY_SURFACE_CLASSES,
          "relative z-10 w-full max-w-lg p-0",
          className
        )}
      >
        {children}
      </div>
    </div>
  );
}
