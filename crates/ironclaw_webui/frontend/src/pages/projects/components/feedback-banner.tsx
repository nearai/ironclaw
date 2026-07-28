import { Callout } from "@ironclaw/ui";
import { useT } from "../../../lib/i18n";

const CALLOUT_TONES = {
  success: "success",
  error: "danger",
  info: "info",
};

export function FeedbackBanner({ result, onDismiss }) {
  const t = useT();
  if (!result) return null;

  return (
    <Callout
      tone={CALLOUT_TONES[result.type] || "info"}
      onDismiss={onDismiss}
      dismissLabel={t("projects.feedback.dismiss")}
    >
      {result.message}
    </Callout>
  );
}
