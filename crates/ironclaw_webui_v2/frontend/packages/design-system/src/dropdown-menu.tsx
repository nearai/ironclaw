/**
 * DropdownMenu — `@radix-ui/react-dropdown-menu` (shadcn action menu).
 * Use SelectMenu for value-picking selects; use this for command menus.
 */
import * as DropdownMenuPrimitive from "@radix-ui/react-dropdown-menu";
import type { ComponentPropsWithoutRef } from "react";
import { cn } from "./cn";

export const DropdownMenu = DropdownMenuPrimitive.Root;
export const DropdownMenuTrigger = DropdownMenuPrimitive.Trigger;
export const DropdownMenuGroup = DropdownMenuPrimitive.Group;
export const DropdownMenuPortal = DropdownMenuPrimitive.Portal;
export const DropdownMenuSub = DropdownMenuPrimitive.Sub;
export const DropdownMenuRadioGroup = DropdownMenuPrimitive.RadioGroup;

export function DropdownMenuContent({
  className = "",
  sideOffset = 6,
  ...props
}: ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.Content>) {
  return (
    <DropdownMenuPrimitive.Portal>
      <DropdownMenuPrimitive.Content
        sideOffset={sideOffset}
        className={cn(
          "z-30 min-w-[10rem] overflow-hidden rounded-[10px] p-1",
          "border border-[color-mix(in_srgb,var(--v2-text-strong)_16%,var(--v2-panel-border))]",
          "bg-[color-mix(in_srgb,var(--v2-canvas-strong)_92%,var(--v2-surface))]",
          "shadow-[var(--v2-shadow-menu)]",
          className
        )}
        {...props}
      />
    </DropdownMenuPrimitive.Portal>
  );
}

export function DropdownMenuItem({
  className = "",
  inset,
  ...props
}: ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.Item> & {
  inset?: boolean;
}) {
  return (
    <DropdownMenuPrimitive.Item
      className={cn(
        "relative flex cursor-default items-center gap-2 rounded-[7px] px-2.5 py-2",
        "text-[13px] text-[var(--v2-text)] outline-none select-none",
        "transition-colors duration-[var(--v2-duration-fast)]",
        "data-[highlighted]:bg-[var(--v2-surface-muted)] data-[highlighted]:text-[var(--v2-text-strong)]",
        "data-[disabled]:pointer-events-none data-[disabled]:opacity-50",
        inset && "pl-8",
        className
      )}
      {...props}
    />
  );
}

export function DropdownMenuLabel({
  className = "",
  inset,
  ...props
}: ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.Label> & {
  inset?: boolean;
}) {
  return (
    <DropdownMenuPrimitive.Label
      className={cn(
        "px-2.5 py-1.5 font-mono text-[10px] uppercase tracking-[0.08em] text-[var(--v2-text-faint)]",
        inset && "pl-8",
        className
      )}
      {...props}
    />
  );
}

export function DropdownMenuSeparator({
  className = "",
  ...props
}: ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.Separator>) {
  return (
    <DropdownMenuPrimitive.Separator
      className={cn("my-1 h-px bg-[var(--v2-panel-border)]", className)}
      {...props}
    />
  );
}

export function DropdownMenuCheckboxItem({
  className = "",
  children,
  checked,
  ...props
}: ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.CheckboxItem>) {
  return (
    <DropdownMenuPrimitive.CheckboxItem
      className={cn(
        "relative flex cursor-default items-center rounded-[7px] py-2 pr-2.5 pl-8",
        "text-[13px] text-[var(--v2-text)] outline-none select-none",
        "data-[highlighted]:bg-[var(--v2-surface-muted)] data-[highlighted]:text-[var(--v2-text-strong)]",
        "data-[disabled]:pointer-events-none data-[disabled]:opacity-50",
        className
      )}
      checked={checked}
      {...props}
    >
      <span className="absolute left-2.5 flex h-3.5 w-3.5 items-center justify-center">
        <DropdownMenuPrimitive.ItemIndicator>
          <span className="h-1.5 w-1.5 rounded-full bg-[var(--v2-accent)]" />
        </DropdownMenuPrimitive.ItemIndicator>
      </span>
      {children}
    </DropdownMenuPrimitive.CheckboxItem>
  );
}
