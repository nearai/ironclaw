import { listAutomations } from "../../../lib/api.js";

export function fetchAutomations() {
  return listAutomations({ limit: 50 });
}
