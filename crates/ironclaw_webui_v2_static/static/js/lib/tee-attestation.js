export function getTeeEndpoint(location) {
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

export function buildTeeReportCopyPayload({ report, teeInfo }) {
  return JSON.stringify({ ...report, instance_attestation: teeInfo }, null, 2);
}

function isIpAddress(hostname) {
  return hostname.includes(":") || /^(\d{1,3}\.){3}\d{1,3}$/.test(hostname);
}
