import { useT } from "../../../lib/i18n";
import { Icon, Modal, ModalBody } from "@ironclaw/design-system";

const SHORTCUTS = [
  { keys: ["Enter"], descKey: "shortcuts.send" },
  { keys: ["Shift", "Enter"], descKey: "shortcuts.newline" },
  { keys: ["?"], descKey: "shortcuts.help" },
  { keys: ["Esc"], descKey: "shortcuts.close" },
];

export function KeyboardShortcuts({ open, onClose }) {
  const t = useT();

  return (
    <Modal
      open={open}
      onClose={onClose}
      size="sm"
      closeLabel={t("shortcuts.close")}
      title={
        <span className="flex items-center gap-2">
          <span className="grid h-8 w-8 place-items-center rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)]">
            <Icon name="bolt" className="h-4 w-4" />
          </span>
          {t("shortcuts.title")}
        </span>
      }
    >
      <ModalBody>
        <ul className="flex flex-col gap-2">
          {SHORTCUTS.map(
            (shortcut, index) => (
              <li
                key={index}
                className="flex items-center justify-between gap-3 text-sm text-[var(--v2-text)]"
              >
                <span>{t(shortcut.descKey)}</span>
                <span className="flex items-center gap-1">
                  {shortcut.keys.map(
                    (key, keyIndex) => (<kbd
                      key={keyIndex}
                      className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-2 py-0.5 font-mono text-[11px] text-[var(--v2-text-muted)]"
                    >{key}</kbd>)
                  )}
                </span>
              </li>
            )
          )}
        </ul>
      </ModalBody>
    </Modal>
  );
}
