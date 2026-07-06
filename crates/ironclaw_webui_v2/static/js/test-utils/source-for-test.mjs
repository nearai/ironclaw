import { readFileSync } from "node:fs";

function stripExport(line) {
  if (/^\s*export\s+\{/.test(line)) return null;
  return line
    .replace(/^(\s*)export\s+default\s+async\s+function(\s+)([A-Za-z_$][\w$]*)/, "$1async function$2$3")
    .replace(/^(\s*)export\s+default\s+async\s+function\b/, "$1async function __defaultExport")
    .replace(/^(\s*)export\s+default\s+function(\s+)([A-Za-z_$][\w$]*)/, "$1function$2$3")
    .replace(/^(\s*)export\s+default\s+function\b/, "$1function __defaultExport")
    .replace(/^(\s*)export\s+default\s+class(\s+)([A-Za-z_$][\w$]*)/, "$1class$2$3")
    .replace(/^(\s*)export\s+default\s+class\b/, "$1class __defaultExport")
    .replace(/^(\s*)export\s+default\s+/, "$1const __defaultExport = ")
    .replace(/^(\s*)export\s+((?:async\s+)?function|class|const|let|var)\b/, "$1$2");
}

function exportBinding(name) {
  return name === "default" ? "default: __defaultExport" : `${name}: ${name}`;
}

export function sourceForTest(baseUrl, path, exportNames) {
  const source = readFileSync(new URL(path, baseUrl), "utf8");
  const lines = [];
  let skippingImport = false;
  for (const line of source.split("\n")) {
    if (!skippingImport && /^\s*import\b/.test(line)) {
      skippingImport = !line.trimEnd().endsWith(";");
      continue;
    }
    if (skippingImport) {
      skippingImport = !line.trimEnd().endsWith(";");
      continue;
    }
    const stripped = stripExport(line);
    if (stripped != null) lines.push(stripped);
  }
  return `${lines.join("\n")}\nglobalThis.__testExports = { ${exportNames.map(exportBinding).join(", ")} };`;
}
