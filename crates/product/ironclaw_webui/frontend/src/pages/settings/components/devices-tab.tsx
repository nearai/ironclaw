import { useState } from "react";

import { Card } from "../../../design-system/card";
import { InlineNotice } from "../../../design-system/inline-notice";
import { LinkPayloadPanel } from "../../../components/link-payload-panel";
import { useT } from "../../../lib/i18n";
import { ApiError } from "../../../lib/api";
import { mintSessionToken } from "../../../lib/session-tokens-api";
import { matchesSearch } from "../lib/settings-search";
import { SettingsSearchEmpty } from "./settings-search-empty";

// DEMO SCOPE: self-serve connect-link card. The link embeds a bearer minted
// for the current caller — anyone who has it can act as that caller until it
// is rotated. Superseded by device-code pairing; delete this tab (and its
// route/lib) when that lands.
export function DevicesTab({ searchQuery = "" }) {
  const t = useT();
  const [payload, setPayload] = useState("");
  const [isMinting, setIsMinting] = useState(false);
  const [error, setError] = useState(null);

  const mint = async () => {
    setIsMinting(true);
    setError(null);
    try {
      const response = await mintSessionToken();
      const token = response?.token;
      if (!token) {
        throw new Error("empty token");
      }
      const url = new URL("ironclaw://connect");
      url.searchParams.set("url", window.location.origin);
      url.searchParams.set("token", token);
      setPayload(url.toString());
    } catch (err) {
      setPayload("");
      setError(err instanceof ApiError ? err.message : t("devices.mintFailed"));
    } finally {
      setIsMinting(false);
    }
  };

  if (!matchesSearch(searchQuery, ["devices", t("settings.devices"), t("devices.title")])) {
    return (<SettingsSearchEmpty query={searchQuery} />);
  }

  return (
    <Card padding="md">
      <h3
        className="mb-2 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
      >
        {t("devices.title")}
      </h3>
      <p className="text-sm leading-6 text-[var(--v2-text-muted)]">
        {t("devices.description")}
      </p>

      <div className="mt-4">
        {payload
          ? (
            <LinkPayloadPanel
              payload={payload}
              showQr
              idPrefix="device-link"
              labels={{
                qrAlt: t("devices.qrAlt"),
                copy: t("devices.copyLink"),
                copied: t("devices.copied"),
                open: "",
                expiresIn: () => "",
                expired: "",
                renew: t("devices.newLink"),
              }}
              onRenew={mint}
              isRenewing={isMinting}
            />
          )
          : (
            <button
              type="button"
              onClick={mint}
              disabled={isMinting}
              data-testid="devices-generate-link"
              className="rounded-lg border border-[var(--v2-accent-soft)] px-3 py-1.5 text-xs font-medium text-[var(--v2-accent-text)] transition-colors hover:bg-[var(--v2-accent-soft)] disabled:cursor-not-allowed disabled:opacity-50"
            >
              {isMinting ? t("devices.generating") : t("devices.generateLink")}
            </button>
          )}
      </div>

      {error &&
      (
        <InlineNotice className="mt-3" tone="danger" role="alert">
          {error}
        </InlineNotice>
      )}

      <p className="mt-5 text-xs leading-5 text-[var(--v2-text-faint)]">
        {t("devices.note")}
      </p>
    </Card>
  );
}
