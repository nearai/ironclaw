/**
 * ModifyTaskDialog — the "Modify" affordance for both suggested and automated
 * tasks. It edits the fields that change what the task *does*, then hands back a
 * kind-shaped patch; it never mutates the task itself (the caller runs the
 * command through the API seam and applies the confirmed result).
 *
 *   calendar_reschedule → choose a different destination slot + an optional note
 *   email_triage        → toggle which drafted replies send, and edit their text
 *   other               → a free-form note
 */
import React from "react";
import { Modal, ModalBody, ModalFooter } from "../../../design-system/modal";
import { Button } from "../../../design-system/button";
import { Icon } from "../../../design-system/icons";
import { useT } from "../../../lib/i18n";
import type {
  AutomationTask,
  AutomationTaskPatch,
  CalendarSlot,
  TriagedEmail,
} from "../lib/automation-tasks";

const FIELD_CLASS =
  "w-full rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 py-2 text-sm text-[var(--v2-text-strong)] outline-none focus:border-[var(--v2-accent)] focus:ring-2 focus:ring-[color-mix(in_srgb,var(--v2-accent)_26%,transparent)]";

function slotKey(slot: CalendarSlot): string {
  return `${slot.day} · ${slot.time}`;
}

export function ModifyTaskDialog({
  task,
  open,
  onClose,
  onSave,
}: {
  task: AutomationTask | null;
  open: boolean;
  onClose: () => void;
  onSave: (patch: AutomationTaskPatch) => void;
}) {
  const t = useT();

  const reschedule = task?.reschedule;
  const [selectedSlot, setSelectedSlot] = React.useState<string>("");
  const [note, setNote] = React.useState<string>("");
  const [emails, setEmails] = React.useState<TriagedEmail[]>([]);

  // Re-seed local edit state whenever a new task opens the dialog.
  React.useEffect(() => {
    if (!open || !task) return;
    setSelectedSlot(reschedule ? slotKey(reschedule.to) : "");
    setNote(reschedule?.note ?? "");
    setEmails(task.emails ? task.emails.map((email) => ({ ...email })) : []);
  }, [open, task, reschedule]);

  if (!task) return null;

  const handleSave = () => {
    const patch: AutomationTaskPatch = {};
    if (reschedule) {
      const chosen =
        [reschedule.to, ...reschedule.alternativeSlots].find(
          (slot) => slotKey(slot) === selectedSlot,
        ) ?? reschedule.to;
      patch.reschedule = { to: chosen, note };
    }
    if (task.emails) {
      patch.emails = emails;
    }
    if (!reschedule && !task.emails) {
      patch.note = note;
    }
    onSave(patch);
    onClose();
  };

  const slotOptions: CalendarSlot[] = reschedule
    ? [
        reschedule.to,
        ...reschedule.alternativeSlots.filter(
          (slot) => slotKey(slot) !== slotKey(reschedule.to),
        ),
      ]
    : [];

  return (
    <Modal
      open={open}
      onClose={onClose}
      title={t("automation.modify.title")}
      size="lg"
      closeLabel={t("common.close")}
    >
      <ModalBody className="space-y-5">
        <p className="text-sm text-[var(--v2-text-muted)]">{task.title}</p>

        {reschedule && (
          <div className="space-y-3">
            <div className="font-mono text-[0.6875rem] font-semibold uppercase tracking-[0.16em] text-[var(--v2-text-faint)]">
              {t("automation.modify.chooseSlot")}
            </div>
            <div className="grid gap-2">
              {slotOptions.map((slot) => {
                const key = slotKey(slot);
                const active = key === selectedSlot;
                return (
                  <button
                    key={key}
                    type="button"
                    onClick={() => setSelectedSlot(key)}
                    className={[
                      "flex items-center justify-between gap-3 rounded-[12px] border px-4 py-3 text-left transition-colors",
                      active
                        ? "border-[var(--v2-accent)] bg-[var(--v2-accent-soft)]"
                        : "border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] hover:border-[color-mix(in_srgb,var(--v2-accent)_36%,var(--v2-panel-border))]",
                    ].join(" ")}
                  >
                    <span className="flex items-center gap-3">
                      <Icon
                        name="calendar"
                        className="h-4 w-4 text-[var(--v2-text-muted)]"
                      />
                      <span className="text-sm text-[var(--v2-text-strong)]">
                        <span className="font-medium">{slot.day}</span>
                        <span className="mx-1.5 text-[var(--v2-text-faint)]">·</span>
                        {slot.time}
                      </span>
                    </span>
                    {active && (
                      <Icon
                        name="check"
                        className="h-4 w-4 text-[var(--v2-accent-text)]"
                      />
                    )}
                  </button>
                );
              })}
            </div>
          </div>
        )}

        {emails.length > 0 && (
          <div className="space-y-3">
            <div className="font-mono text-[0.6875rem] font-semibold uppercase tracking-[0.16em] text-[var(--v2-text-faint)]">
              {t("automation.modify.chooseReplies")}
            </div>
            <div className="grid gap-3">
              {emails.map((email, index) => (
                <div
                  key={email.id}
                  className="rounded-[12px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-3"
                >
                  <label className="flex items-start gap-2.5">
                    <input
                      type="checkbox"
                      checked={email.include}
                      onChange={(event) =>
                        setEmails((current) =>
                          current.map((item, i) =>
                            i === index
                              ? { ...item, include: event.currentTarget.checked }
                              : item,
                          ),
                        )
                      }
                      className="mt-1 h-3.5 w-3.5 accent-[var(--v2-accent)]"
                    />
                    <span className="min-w-0">
                      <span className="block text-sm font-medium text-[var(--v2-text-strong)]">
                        {email.subject}
                      </span>
                      <span className="block text-xs text-[var(--v2-text-muted)]">
                        {email.from}
                      </span>
                    </span>
                  </label>
                  <textarea
                    value={email.draft}
                    disabled={!email.include}
                    onChange={(event) =>
                      setEmails((current) =>
                        current.map((item, i) =>
                          i === index
                            ? { ...item, draft: event.currentTarget.value }
                            : item,
                        ),
                      )
                    }
                    rows={2}
                    className={`mt-2.5 resize-none ${FIELD_CLASS} disabled:opacity-50`}
                  />
                </div>
              ))}
            </div>
          </div>
        )}

        <div className="space-y-2">
          <div className="font-mono text-[0.6875rem] font-semibold uppercase tracking-[0.16em] text-[var(--v2-text-faint)]">
            {t("automation.modify.note")}
          </div>
          <textarea
            value={note}
            onChange={(event) => setNote(event.currentTarget.value)}
            rows={2}
            placeholder={t("automation.modify.notePlaceholder")}
            className={`resize-none ${FIELD_CLASS}`}
          />
        </div>
      </ModalBody>
      <ModalFooter>
        <Button variant="ghost" onClick={onClose}>
          {t("common.cancel")}
        </Button>
        <Button variant="primary" onClick={handleSave}>
          {t("automation.modify.save")}
        </Button>
      </ModalFooter>
    </Modal>
  );
}
