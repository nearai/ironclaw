import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  buildTeeReportCopyPayload,
  getTeeEndpoint,
} from "./tee-attestation.js";

function locationFor(hostname, protocol = "https:") {
  return { hostname, protocol };
}

function source(relativePath) {
  return readFileSync(new URL(relativePath, import.meta.url), "utf8");
}

function assertIncludes(haystack, needles, label) {
  for (const needle of needles) {
    assert.ok(
      haystack.includes(needle),
      `${label} should include ${JSON.stringify(needle)}`
    );
  }
}

test("getTeeEndpoint derives the deployment-owned attestation API for public hosts", () => {
  assert.deepEqual(getTeeEndpoint(locationFor("app.example.com")), {
    base: "https://api.example.com",
    instance: "app",
  });
  assert.deepEqual(getTeeEndpoint(locationFor("tee.eu.example.com")), {
    base: "https://api.eu.example.com",
    instance: "tee",
  });
  assert.deepEqual(getTeeEndpoint(locationFor("preview.example.test", "http:")), {
    base: "http://api.example.test",
    instance: "preview",
  });
});

test("getTeeEndpoint suppresses local, IP, IPv6, empty, and single-label hosts", () => {
  for (const hostname of [
    "",
    "localhost",
    "127.0.0.1",
    "192.168.1.25",
    "::1",
    "2001:db8::1",
    "devbox",
  ]) {
    assert.equal(getTeeEndpoint(locationFor(hostname)), null, hostname);
  }
});

test("buildTeeReportCopyPayload formats report data with instance attestation", () => {
  const report = {
    report_data: "report-data",
    vm_config: { cpu: 4 },
  };
  const teeInfo = {
    image_digest: "sha256:abc",
    tls_certificate_fingerprint: "fp",
  };

  assert.equal(
    buildTeeReportCopyPayload({ report, teeInfo }),
    JSON.stringify({ ...report, instance_attestation: teeInfo }, null, 2)
  );
});

test("useTeeAttestation fetches attestation/report endpoints and handles copy gates", () => {
  const hook = source("../hooks/useTeeAttestation.js");

  assertIncludes(
    hook,
    [
      "buildTeeReportCopyPayload",
      "getTeeEndpoint",
      'from "../lib/tee-attestation.js";',
      "getTeeEndpoint(window.location)",
      "fetch(`${endpoint.base}/instances/${encodeURIComponent(endpoint.instance)}/attestation`,",
      "if (!response.ok) throw new Error(String(response.status));",
      "if (!controller.signal.aborted) setTeeInfo(null);",
      "if (!endpoint || report || reportLoading) return report;",
      "fetch(`${endpoint.base}/attestation/report`)",
      'setReportError(err.message || "Could not load attestation report.");',
      "const data = report || (await loadReport());",
      "if (!data || !navigator.clipboard) return false;",
      "navigator.clipboard.writeText(",
      "buildTeeReportCopyPayload({ report: data, teeInfo })",
      "window.setTimeout(() => setCopied(false), 1800);",
      "available: Boolean(teeInfo)",
    ],
    "useTeeAttestation"
  );
});

test("TeeShield and PageHeader retain report popover and header integration contracts", () => {
  const shield = source("../components/tee-shield.js");
  const header = source("../components/page-header.js");

  assertIncludes(
    shield,
    [
      "useTeeAttestation()",
      "if (nextOpen) tee.loadReport();",
      "tee.copyReport().catch(() => {});",
      "if (!tee.available) return null;",
      "const rows = buildRows({ teeInfo: tee.teeInfo, report: tee.report, t });",
      "tee.reportLoading",
      "tee.reportError",
      'disabled=${tee.reportLoading}',
      'tee.copied ? t("tee.copied") : t("tee.copyReport")',
      "text.length > 72",
    ],
    "TeeShield"
  );

  assertIncludes(
    header,
    [
      'import { TeeShield } from "./tee-shield.js";',
      "<${TeeShield} />",
      'to="/logs"',
      "href=${DOCS_URL}",
    ],
    "PageHeader"
  );
});
