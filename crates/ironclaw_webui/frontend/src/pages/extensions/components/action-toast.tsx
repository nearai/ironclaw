import React from "react";
import { Callout, Icon } from "@ironclaw/ui";
import { useT } from "../../../lib/i18n";

const TONES = {
  success: "success",
  error: "danger",
  info: "info",
};

export function ActionToast({ result, onDismiss }) {
  const t = useT();
  React.useEffect(() => {
    if (!result) return;
    const timer = setTimeout(onDismiss, 4000);
    return () => clearTimeout(timer);
  }, [result, onDismiss]);

  if (!result) return null;

  return (
    <Callout
      tone={TONES[result.type] || "info"}
      onDismiss={onDismiss}
      dismissLabel={
        <>
          <span className="sr-only">{t("common.dismiss")}</span>
          <Icon name="close" className="h-3.5 w-3.5" aria-hidden="true" />
        </>
      }
    >
      <span className="flex items-center gap-3">
        <Icon
          name={result.type === "success" ? "check" : result.type === "error" ? "close" : "bolt"}
          className="h-4 w-4 shrink-0"
        />
        <span className="min-w-0 flex-1">{result.message}</span>
      </span>
    </Callout>
  );
}
