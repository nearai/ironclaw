import React from "react";
import { UiTextProvider } from "@ironclaw/ui";
import { useT } from "../lib/i18n";

/**
 * Bridges the app's i18n into @ironclaw/ui's built-in strings (modal close
 * aria-label, confirm-dialog cancel label). The design system stays
 * i18n-agnostic; this provider re-renders whenever the active language
 * pack changes, keeping the fallbacks localized exactly as before.
 */
export function UiTextBridge({ children }) {
  const t = useT();
  const text = React.useMemo(
    () => ({ close: t("common.close"), cancel: t("common.cancel") }),
    [t]
  );
  return (<UiTextProvider text={text}>{children}</UiTextProvider>);
}
