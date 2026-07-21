/**
 * Design-system i18n bridge.
 *
 * The package must not depend on the host app's i18n stack, but a few
 * components render built-in strings (modal close, confirm-dialog
 * cancel). Hosts that localize wrap their tree in
 * `DesignSystemI18nProvider` and pass their own `t`; without a
 * provider the components fall back to the bundled English strings.
 */
import { createContext, useContext, type ReactNode } from "react";

const DEFAULT_STRINGS: Record<string, string> = {
  "common.close": "Close",
  "common.cancel": "Cancel",
};

export type DesignSystemTranslate = (key: string) => string;

const fallbackT: DesignSystemTranslate = (key) => DEFAULT_STRINGS[key] ?? key;

const DesignSystemI18nContext = createContext<DesignSystemTranslate>(fallbackT);

export function DesignSystemI18nProvider({
  t,
  children,
}: {
  t: DesignSystemTranslate;
  children: ReactNode;
}) {
  return (
    <DesignSystemI18nContext.Provider value={t}>{children}</DesignSystemI18nContext.Provider>
  );
}

/** Translate hook used internally by design-system components. */
export function useDesignSystemT(): DesignSystemTranslate {
  return useContext(DesignSystemI18nContext);
}
