// @ts-nocheck
import React from "react";
import { Button } from "../../../design-system/button";
import { Icon } from "../../../design-system/icons";
import { Input } from "../../../design-system/input";
import { useT } from "../../../lib/i18n";
import { cn } from "../../../utils/cn";

// Matches the backend limit for automation names.
const AUTOMATION_NAME_MAX_BYTES = 256;

function byteLength(value) {
  if (typeof TextEncoder !== "undefined") {
    return new TextEncoder().encode(value).length;
  }
  return value.length;
}

/**
 * The automation display name with inline rename editing (pencil → input +
 * save/cancel). Shared by the detail modal and the full-screen detail page so
 * rename works identically on both surfaces.
 *
 * Props
 *   automation          normalized automation (display_name, automation_id)
 *   isMutating          disables the form while a mutation is in flight
 *   onRenameAutomation  ({ automationId, name }) => void — omit to hide rename
 *   headingClassName    typography for the read view heading
 */
export function EditableAutomationName({
  automation,
  isMutating = false,
  onRenameAutomation,
  headingClassName = "truncate text-lg font-semibold tracking-[-0.02em] text-[var(--v2-text-strong)] md:text-2xl",
}) {
  const t = useT();
  const [isEditing, setIsEditing] = React.useState(false);
  const [draftName, setDraftName] = React.useState("");
  const [nameError, setNameError] = React.useState("");

  React.useEffect(() => {
    setIsEditing(false);
    setDraftName(automation?.display_name || "");
    setNameError("");
  }, [automation?.automation_id]);

  const canRename = Boolean(onRenameAutomation);
  const renameTitle = `${t("automations.rename.action")}: ${automation.display_name}`;

  const handleStart = () => {
    setDraftName(automation.display_name);
    setNameError("");
    setIsEditing(true);
  };
  const handleCancel = () => {
    setDraftName(automation.display_name);
    setNameError("");
    setIsEditing(false);
  };
  const handleSubmit = (event) => {
    event.preventDefault();
    const name = draftName.trim();
    if (!name) {
      setNameError(t("automations.rename.nameRequired"));
      return;
    }
    if (byteLength(name) > AUTOMATION_NAME_MAX_BYTES) {
      setNameError(t("automations.rename.nameTooLong"));
      return;
    }
    setNameError("");
    if (name !== automation.display_name) {
      onRenameAutomation?.({ automationId: automation.automation_id, name });
    }
    setIsEditing(false);
  };

  if (isEditing) {
    return (
      <form className="flex min-w-0 flex-col gap-2" onSubmit={handleSubmit}>
        <div className="flex min-w-0 items-center gap-2">
          <Input
            size="sm"
            value={draftName}
            data-testid="automation-rename-input"
            aria-label={t("automations.rename.nameLabel")}
            disabled={isMutating}
            error={Boolean(nameError)}
            className="min-w-0"
            onInput={(event) => {
              setDraftName(event.currentTarget.value);
              if (nameError) setNameError("");
            }}
          />
          <Button
            type="submit"
            variant="primary"
            size="icon-sm"
            data-testid="automation-rename-save"
            aria-label={t("common.save")}
            title={t("common.save")}
            disabled={isMutating}
          >
            <Icon name="check" className="h-4 w-4" />
          </Button>
          <Button
            type="button"
            variant="secondary"
            size="icon-sm"
            aria-label={t("common.cancel")}
            title={t("common.cancel")}
            disabled={isMutating}
            onClick={handleCancel}
          >
            <Icon name="close" className="h-4 w-4" />
          </Button>
        </div>
        {nameError && (
          <div className="text-xs text-[var(--v2-danger-text)]" role="alert">
            {nameError}
          </div>
        )}
      </form>
    );
  }

  return (
    <div className="flex min-w-0 items-center gap-2">
      <h2
        data-testid="automation-detail-title"
        className={cn("min-w-0", headingClassName)}
        title={automation.display_name}
      >
        {automation.display_name}
      </h2>
      {canRename && (
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          className="shrink-0"
          data-testid="automation-rename-button"
          aria-label={renameTitle}
          title={renameTitle}
          disabled={isMutating}
          onClick={handleStart}
        >
          <Icon name="edit" className="h-4 w-4" />
        </Button>
      )}
    </div>
  );
}
