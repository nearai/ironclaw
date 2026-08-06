import { readFileSync } from "node:fs";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const STATIC_IMPORT_RE =
  /(^|\n)[ \t]*import(?:\s+[\s\S]*?\s+from\s*)?\s*["'][^"'\n]+["'](?:\s+(?:assert|with)\s*\{[\s\S]*?\})?\s*;?[ \t]*(?=\n|$)/g;
const NAMED_EXPORT_DECLARATION_RE =
  /^(\s*)export\s+(?=(?:async\s+)?function\b|class\b|(?:const|let|var)\b)/gm;
const NAMED_EXPORT_LIST_RE = /^(\s*)export\s*\{([\s\S]*?)\}\s*;?[ \t]*$/gm;

export function sourceForVmTest(path, exportNames, metaUrl) {
  return sourceTextForVmTest(
    readFileSync(new URL(path, metaUrl), "utf8"),
    exportNames
  );
}

export function sourceTextForVmTest(source, exportNames) {
  const exportAliases = new Map();
  const transformed = source
    .replace(STATIC_IMPORT_RE, "$1")
    .replace(NAMED_EXPORT_LIST_RE, (_match, _indent, specifiers) => {
      for (const specifier of specifiers.split(",")) {
        const trimmed = specifier.trim();
        if (!trimmed) continue;
        const [localName, exportedName = localName] = trimmed.split(/\s+as\s+/);
        exportAliases.set(exportedName.trim(), localName.trim());
      }
      return "";
    })
    .replace(NAMED_EXPORT_DECLARATION_RE, "$1");
  const testExports = exportNames
    .map((name) => {
      const localName = exportAliases.get(name) || name;
      return localName === name ? name : `${JSON.stringify(name)}: ${localName}`;
    })
    .join(", ");
  return `${transformed.trimEnd()}\nglobalThis.__testExports = { ${testExports} };\n`;
}

export function runVmModuleForTest(path, exportNames, context, metaUrl) {
  const moduleUrl = new URL(path, metaUrl);
  // Capture exports on a detached object; the sandbox globals stay explicit.
  context.globalThis = {};
  vm.runInNewContext(sourceForVmTest(path, exportNames, metaUrl), context, {
    filename: fileURLToPath(moduleUrl),
  });
  return context.globalThis.__testExports;
}
