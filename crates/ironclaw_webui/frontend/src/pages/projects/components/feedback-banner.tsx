import { Callout } from "@ironclaw/ui";
import { useT } from "../../../lib/i18n";

// Thin wrapper over the design-system <Callout> so call sites keep passing
// the `{ type, message }` action-result shape; the legacy mint/signal/red
// banner colors folded into Callout's token tones.
const tone = {
  success: "success",
  error: "danger",
  info: "info",
};

export function FeedbackBanner({ result, onDismiss }) {
  const t = useT();
  if (!result) return null;

  return (
    <Callout
      tone={tone[result.type] || tone.info}
      onDismiss={onDismiss}
      dismissLabel={t("projects.feedback.dismiss")}
    >
      {result.message}
    </Callout>
  );
}
