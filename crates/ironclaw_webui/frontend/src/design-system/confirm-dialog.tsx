import type { ReactNode } from "react";
import { useT } from "../lib/i18n";
import { Button } from "./button";
import { Modal, ModalBody, ModalFooter } from "./modal";

type ConfirmDialogProps = {
  open: boolean;
  title: string;
  description?: ReactNode;
  confirmLabel: string;
  cancelLabel?: string;
  isConfirming?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
};

export function ConfirmDialog({
  open,
  title,
  description,
  confirmLabel,
  cancelLabel,
  isConfirming = false,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  const t = useT();
  const resolvedCancelLabel = cancelLabel || t("common.cancel");
  const handleCancel = () => {
    if (!isConfirming) onCancel();
  };

  return (
    <Modal
      open={open}
      onClose={isConfirming ? undefined : handleCancel}
      title={title}
      closeLabel={resolvedCancelLabel}
      size="sm"
    >
      {description ? (
        <ModalBody>
          <p className="text-sm leading-6 text-[var(--v2-text-muted)]">{description}</p>
        </ModalBody>
      ) : null}
      <ModalFooter>
        <Button
          type="button"
          variant="secondary"
          size="sm"
          autoFocus
          disabled={isConfirming}
          onClick={handleCancel}
          data-testid="confirm-dialog-cancel"
        >
          {resolvedCancelLabel}
        </Button>
        <Button
          type="button"
          variant="danger"
          size="sm"
          loading={isConfirming}
          onClick={onConfirm}
          data-testid="confirm-dialog-confirm"
        >
          {confirmLabel}
        </Button>
      </ModalFooter>
    </Modal>
  );
}
