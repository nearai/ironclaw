// @ts-nocheck
import { Modal, ModalBody } from "../../../design-system/modal";
import { useT } from "../../../lib/i18n";
import { DeliveryDefaultsContent } from "./automation-delivery-defaults-panel";

// Delivery defaults presented as a modal opened from the list header, rather
// than a flat always-present card on the page. The modal owns the title; the
// content component supplies the form.
export function AutomationDeliveryDefaultsModal({ deliveryState, open, onClose }) {
  const t = useT();
  return (
    <Modal open={open} onClose={onClose} size="md" title={t("automations.delivery.title")}>
      <ModalBody>
        <DeliveryDefaultsContent deliveryState={deliveryState} />
      </ModalBody>
    </Modal>
  );
}
