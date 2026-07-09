import React from "react";

export function useTeeAttestation() {
  const endpoint = React.useMemo(() => getTeeEndpoint(window.location), []);
  const [teeInfo, setTeeInfo] = React.useState(null);
  const [report, setReport] = React.useState(null);
  const [reportLoading, setReportLoading] = React.useState(false);
  const [reportError, setReportError] = React.useState("");
  const [copied, setCopied] = React.useState(false);

  React.useEffect(() => {
    if (!endpoint) return undefined;
    const controller = new AbortController();

    fetch(`${endpoint.base}/instances/${encodeURIComponent(endpoint.instance)}/attestation`, {
      signal: controller.signal,
    })
      .then((response) => {
        if (!response.ok) throw new Error(String(response.status));
        return response.json();
      })
      .then(setTeeInfo)
      .catch(() => {
        if (!controller.signal.aborted) setTeeInfo(null);
      });

    return () => controller.abort();
  }, [endpoint]);

  const loadReport = React.useCallback(async () => {
    if (!endpoint || report || reportLoading) return report;
    setReportLoading(true);
    setReportError("");
    try {
      const response = await fetch(`${endpoint.base}/attestation/report`);
      if (!response.ok) throw new Error(String(response.status));
      const data = await response.json();
      setReport(data);
      return data;
    } catch (err) {
      setReportError(err.message || "Could not load attestation report.");
      return null;
    } finally {
      setReportLoading(false);
    }
  }, [endpoint, report, reportLoading]);

  const copyReport = React.useCallback(async () => {
    const data = report || (await loadReport());
    if (!data || !navigator.clipboard) return false;
    await navigator.clipboard.writeText(
      JSON.stringify({ ...data, instance_attestation: teeInfo }, null, 2)
    );
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1800);
    return true;
  }, [loadReport, report, teeInfo]);

  return {
    available: Boolean(teeInfo),
    teeInfo,
    report,
    reportError,
    reportLoading,
    copied,
    loadReport,
    copyReport,
  };
}

function getTeeEndpoint(location) {
  const hostname = location.hostname;
  if (!hostname || hostname === "localhost" || isIpAddress(hostname)) {
    return null;
  }

  const parts = hostname.split(".");
  if (parts.length < 2) return null;

  return {
    base: `${location.protocol}//api.${parts.slice(1).join(".")}`,
    instance: parts[0],
  };
}

function isIpAddress(hostname) {
  return hostname.includes(":") || /^(\d{1,3}\.){3}\d{1,3}$/.test(hostname);
}
