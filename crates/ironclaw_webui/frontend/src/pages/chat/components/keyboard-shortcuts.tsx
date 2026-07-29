import { useT } from "../../../lib/i18n";
import { Icon, Modal, ModalBody, Text } from "@ironclaw/design-system";

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
          {SHORTCUTS.map((shortcut, index) => (
            <Text
              as="li"
              key={index}
              variant="body"
              className="flex items-center justify-between gap-3"
            >
              <span>{t(shortcut.descKey)}</span>
              <span className="flex items-center gap-1">
                {shortcut.keys.map((key, keyIndex) => (
                  <Text
                    as="kbd"
                    key={keyIndex}
                    variant="mono"
                    tone="muted"
                    className="rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-2 py-0.5 text-[length:var(--v2-font-size-label)]"
                  >
                    {key}
                  </Text>
                ))}
              </span>
            </Text>
          ))}
        </ul>
      </ModalBody>
    </Modal>
  );
}
