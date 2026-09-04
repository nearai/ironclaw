import { registerPack } from "../../lib/i18n";

// Consent copy for the lazy IronHub install route.
// The fallback English pack loads eagerly on every route, including /chat.
// Non-English copy remains in each locale pack and parity is enforced by
// `src/lib/i18n.test.ts`.
registerPack("en", {
  "ironhub.install.title": "Install from IronHub",
  "ironhub.install.description": "Review what this link installs before you approve it.",
  "ironhub.install.name": "Name",
  "ironhub.install.version": "Version",
  "ironhub.install.digest": "Artifact digest",
  "ironhub.install.privateSource": "Private manifest source",
  "ironhub.install.confirm": "Install",
  "ironhub.install.installing": "Installing...",
  "ironhub.install.installed": "Installed.",
  "ironhub.install.notInstalled": "The hub reported that nothing was installed.",
  "ironhub.install.linkInvalid": "This install link is incomplete or malformed.",
  "ironhub.install.rejected": "This install link was not signed for this agent.",
  "ironhub.install.expired": "This install link has expired. Start the install again from the hub.",
  "ironhub.install.alreadyUsed": "This install link has already been used. Start the install again from the hub.",
  "ironhub.install.failed": "The install could not be completed.",
});
