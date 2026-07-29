/**
 * UiText
 *
 * The design system is i18n-agnostic: components never import the app's
 * translation layer. The handful of built-in strings (the Modal close
 * button's aria-label, ConfirmDialog's default cancel label) resolve through
 * this context instead. The app mounts <UiTextProvider> once, bridging its
 * own i18n `t()` into these slots; without a provider the English defaults
 * apply. Explicit props (closeLabel, cancelLabel) always win over context.
 */
import React from "react";

export type UiText = {
  /** aria-label / title for dismiss affordances (Modal close button). */
  close: string;
  /** Default cancel-action label (ConfirmDialog). */
  cancel: string;
};

export const DEFAULT_UI_TEXT: UiText = {
  close: "Close",
  cancel: "Cancel",
};

const UiTextContext = React.createContext<UiText>(DEFAULT_UI_TEXT);

export function UiTextProvider({
  text,
  children,
}: {
  text: Partial<UiText>;
  children?: React.ReactNode;
}) {
  const value = React.useMemo<UiText>(
    () => ({ ...DEFAULT_UI_TEXT, ...text }),
    [text]
  );
  return (<UiTextContext.Provider value={value}>{children}</UiTextContext.Provider>);
}

export function useUiText(): UiText {
  return React.useContext(UiTextContext);
}
