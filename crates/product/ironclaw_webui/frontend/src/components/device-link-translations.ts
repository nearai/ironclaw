import { registerPack } from "../lib/i18n";

// Consent copy for the lazy connection and device-link setup surfaces.
// The fallback English pack loads eagerly on every route, including /chat.
// Non-English copy remains in each locale pack and parity is enforced by
// `src/lib/i18n.test.ts`.
registerPack("en", {
  "extensions.connectionChoice.title": "How do you want to connect {name}?",
  "extensions.connectionChoice.workspaceBot": "Connect a workspace bot",
  "extensions.connectionChoice.workspaceBotDisclosure": "Connect this identity to the workspace bot. The bot can receive your messages and reply as the bot. This does not link your personal account or enable personal-account tools.",
  "extensions.connectionChoice.personalAccount": "Link my personal account",
  "deviceLink.personalDisclosure": "Link your personal {name} account as a third-party device. When you ask IronClaw, it can read your {name} chats and send messages that recipients see as coming from you. Revoke access at any time in {name} Settings → Devices.",
  "deviceLink.title": "Link your {name} account",
  "deviceLink.pillLink": "Link account",
  "deviceLink.startFailed": "Could not start linking {name}.",
  "deviceLink.pollFailed": "Could not refresh the link.",
  "deviceLink.submitFailed": "That value was not accepted. Try again.",
  "deviceLink.qrAlt": "{name} device-link QR",
  "deviceLink.copyCode": "Copy code",
  "deviceLink.openIn": "Open in {name}",
  "deviceLink.expiresIn": "Expires in {time}",
  "deviceLink.expired": "This code expired.",
  "deviceLink.refresh": "Get a new code",
  // Fallbacks ONLY. The mode switch is labelled from the recipe's own
  // `default_mode_label` / `alternate_mode_label`; this copy is what a card
  // says when the extension supplied none. It is shared by every device-link
  // extension, so it must never name one vendor's ceremony.
  "deviceLink.useAlternate": "Use another way to link",
  "deviceLink.useDefault": "Use the first way to link instead",
  "deviceLink.awaiting": "Waiting for {name} to confirm the link…",
  "deviceLink.identifierLabel": "Phone number",
  "deviceLink.codeLabel": "Login code",
  "deviceLink.passwordLabel": "Account password",
  "deviceLink.submit": "Continue",
  "deviceLink.linked": "{name} account linked",
  "deviceLink.confirmDeviceAccount": "Linked as {account}",
  // Vendor-neutral on purpose: the check is "one new device, just now", and
  // every service names the screen that shows it differently. Naming one
  // vendor's menu path here sends every other vendor's users looking for a
  // menu that does not exist.
  "deviceLink.confirmDevice": "Now open your linked-device settings in {name} and check that exactly one new IronClaw device appeared, just now. If you see more than one, or one you did not expect, revoke it there and unlink here.",
  "deviceLink.revokeHint": "IronClaw now shows up as a device in {name}. If you ever see a device you do not recognize, revoke it there.",
  "deviceLink.startAgain": "Start again",
  "deviceLink.cannotRetry": "This {name} account cannot be linked.",
  "deviceLink.error.expired": "The code expired before it was used.",
  "deviceLink.error.unknown_flow": "This link is no longer open.",
  "deviceLink.error.declined": "The device was refused.",
  "deviceLink.error.invalid_input": "That value was not accepted.",
  "deviceLink.error.rate_limited": "Too many attempts. Wait a moment before trying again.",
  "deviceLink.error.account_unavailable": "This account cannot be linked.",
  "deviceLink.error.identity_conflict": "This account is already linked. Unlink it from the IronClaw account where it is connected, then try again.",
  "deviceLink.error.vendor_unavailable": "The service is temporarily unavailable.",
  "deviceLink.error.custody_failed": "The link could not be saved securely.",
  "deviceLink.error.internal": "Something went wrong while linking.",
});
