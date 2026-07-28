import { Button, Callout } from "@ironclaw/design-system";
import { useT } from "../../../lib/i18n";

const CALLOUT_TONE = {
  success: "success",
  error: "danger",
  info: "accent",
};

export function FeedbackBanner({ result, onDismiss }) {
  const t = useT();
  if (!result) return null;

  return (
    <Callout tone={CALLOUT_TONE[result.type] || "accent"} icon={null}>
      <div className="flex items-center gap-3">
        <span className="min-w-0 flex-1">{result.message}</span>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="shrink-0"
          onClick={onDismiss}
        >
          {t("projects.feedback.dismiss")}
        </Button>
      </div>
    </Callout>
  );
}
