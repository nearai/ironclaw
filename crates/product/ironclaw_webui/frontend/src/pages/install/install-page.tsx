// @ts-nocheck
import React from "react";
import { useLocation } from "react-router";
import { useMutation } from "@tanstack/react-query";
import { useT } from "../../lib/i18n";
import { Button } from "../../design-system/button";
import { Card } from "../../design-system/card";
import {
  deliverIronhubInstall,
  installErrorKey,
  readIronhubInstallRequest,
} from "../../lib/ironhub-install-api";

function DetailRow({ label, value, mono = false }) {
  return (
    <div className="flex flex-col gap-1">
      <span className="text-xs font-medium text-[var(--v2-text-muted)]">{label}</span>
      {mono
        ? (<code className="block break-all rounded-lg bg-[var(--v2-surface-muted)] px-3 py-2 text-xs">
            {value}
          </code>)
        : (<span className="text-sm">{value}</span>)}
    </div>
  );
}

export function InstallPage() {
  const t = useT();
  const location = useLocation();
  const { request, missing } = React.useMemo(
    () => readIronhubInstallRequest(location.search),
    [location.search],
  );

  const install = useMutation({
    mutationFn: () => deliverIronhubInstall(request),
  });

  if (missing.length > 0) {
    return (
      <Card padding="md">
        <h2 className="text-sm font-semibold">{t("ironhub.install.title")}</h2>
        <p className="mt-2 text-xs text-destructive">{t("ironhub.install.linkInvalid")}</p>
      </Card>
    );
  }

  return (
    <Card padding="md">
      <div className="space-y-4">
        <div>
          <h2 className="text-sm font-semibold">{t("ironhub.install.title")}</h2>
          <p className="mt-1 text-xs text-[var(--v2-text-muted)]">
            {t("ironhub.install.description")}
          </p>
        </div>

        <div className="space-y-3">
          <DetailRow label={t("ironhub.install.name")} value={request.slug} />
          <DetailRow label={t("ironhub.install.version")} value={request.version} />
          <DetailRow label={t("ironhub.install.digest")} value={request.artifact_digest} mono />
          {request.private_manifest_url
            ? (<DetailRow
                label={t("ironhub.install.privateSource")}
                value={request.private_manifest_url}
                mono
              />)
            : null}
        </div>

        <div className="flex items-center gap-3">
          <Button onClick={() => install.mutate()} disabled={install.isPending}>
            {install.isPending ? t("ironhub.install.installing") : t("ironhub.install.confirm")}
          </Button>
        </div>

        {install.data
          ? (<div className="space-y-1">
              <p className="text-xs">
                {install.data.installed
                  ? t("ironhub.install.installed")
                  : t("ironhub.install.notInstalled")}
              </p>
              {install.data.message
                ? (<p className="text-xs text-[var(--v2-text-muted)]">{install.data.message}</p>)
                : null}
            </div>)
          : null}

        {install.error
          ? (<p className="text-xs text-destructive">{t(installErrorKey(install.error))}</p>)
          : null}
      </div>
    </Card>
  );
}
