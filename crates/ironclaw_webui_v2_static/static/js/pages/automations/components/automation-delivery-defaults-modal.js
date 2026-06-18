import { Modal, ModalBody } from "../../../design-system/modal.js";
import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { DeliveryDefaultsContent } from "./automation-delivery-defaults-panel.js";

// Delivery defaults presented as a modal opened from the list header, rather
// than a flat always-present card on the page. The modal owns the title; the
// content component supplies the form.
export function AutomationDeliveryDefaultsModal({ deliveryState, open, onClose }) {
  const t = useT();
  return html`
    <${Modal}
      open=${open}
      onClose=${onClose}
      size="lg"
      title=${t("automations.delivery.title")}
    >
      <${ModalBody}>
        <${DeliveryDefaultsContent} deliveryState=${deliveryState} />
      <//>
    <//>
  `;
}
