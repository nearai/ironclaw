import { readdirSync, readFileSync } from "node:fs";
import { dirname, extname, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

import ts from "typescript";

const JAVASCRIPT_FAMILY_EXTENSIONS = new Set([
  ".cjs",
  ".cts",
  ".js",
  ".jsx",
  ".mjs",
  ".mts",
  ".ts",
  ".tsx",
]);
const TYPESCRIPT_EXTENSIONS = new Set([".ts", ".tsx"]);
const EXPLICIT_RELATIVE_EXTENSION = /\.(?:[cm]?[jt]sx?)(?:[?#].*)?$/i;

export type ConventionViolationKind =
  | "explicit-relative-extension"
  | "html-tagged-template"
  | "invalid-module-extension"
  | "prohibited-ts-ignore"
  | "stale-ts-nocheck-baseline"
  | "unbaselined-ts-nocheck";

export type ConventionViolation = {
  file: string;
  kind: ConventionViolationKind;
  line: number;
};

function isRelativeModuleSpecifier(value: string): boolean {
  return value.startsWith("./") || value.startsWith("../");
}

function scriptKindForExtension(extension: string): ts.ScriptKind {
  if (extension === ".tsx") return ts.ScriptKind.TSX;
  if (extension === ".jsx") return ts.ScriptKind.JSX;
  if ([".js", ".mjs", ".cjs"].includes(extension)) return ts.ScriptKind.JS;
  return ts.ScriptKind.TS;
}

function lineForNode(sourceFile: ts.SourceFile, node: ts.Node): number {
  return sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile)).line + 1;
}

function moduleSpecifierForNode(node: ts.Node): ts.StringLiteralLike | undefined {
  if (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) {
    return node.moduleSpecifier && ts.isStringLiteralLike(node.moduleSpecifier)
      ? node.moduleSpecifier
      : undefined;
  }
  if (
    ts.isCallExpression(node) &&
    node.expression.kind === ts.SyntaxKind.ImportKeyword &&
    node.arguments.length >= 1 &&
    ts.isStringLiteralLike(node.arguments[0])
  ) {
    return node.arguments[0];
  }
  if (
    ts.isImportEqualsDeclaration(node) &&
    ts.isExternalModuleReference(node.moduleReference) &&
    node.moduleReference.expression &&
    ts.isStringLiteralLike(node.moduleReference.expression)
  ) {
    return node.moduleReference.expression;
  }
  if (
    ts.isImportTypeNode(node) &&
    ts.isLiteralTypeNode(node.argument) &&
    ts.isStringLiteralLike(node.argument.literal)
  ) {
    return node.argument.literal;
  }
  return undefined;
}

function compareViolations(left: ConventionViolation, right: ConventionViolation): number {
  return (
    left.file.localeCompare(right.file) ||
    left.line - right.line ||
    left.kind.localeCompare(right.kind)
  );
}

function checkTypeScriptSuppressions(
  filePath: string,
  sourceFile: ts.SourceFile,
  sourceText: string,
  legacyTsNocheckFiles: ReadonlySet<string>,
  seenLegacyTsNocheckFiles?: Set<string>,
): ConventionViolation[] {
  const violations: ConventionViolation[] = [];
  const scanner = ts.createScanner(
    ts.ScriptTarget.Latest,
    false,
    sourceFile.languageVariant,
    sourceText,
  );
  let remainingLegacyNocheck = legacyTsNocheckFiles.has(filePath) ? 1 : 0;

  for (let token = scanner.scan(); token !== ts.SyntaxKind.EndOfFileToken; token = scanner.scan()) {
    if (
      token !== ts.SyntaxKind.SingleLineCommentTrivia &&
      token !== ts.SyntaxKind.MultiLineCommentTrivia
    ) {
      continue;
    }

    const comment = scanner.getTokenText();
    const directivePattern = /@ts-(nocheck|ignore)\b/g;
    for (const match of comment.matchAll(directivePattern)) {
      const position = scanner.getTokenPos() + (match.index ?? 0);
      const line = sourceFile.getLineAndCharacterOfPosition(position).line + 1;
      if (match[1] === "ignore") {
        violations.push({ file: filePath, kind: "prohibited-ts-ignore", line });
      } else if (remainingLegacyNocheck > 0) {
        remainingLegacyNocheck -= 1;
        seenLegacyTsNocheckFiles?.add(filePath);
      } else {
        violations.push({ file: filePath, kind: "unbaselined-ts-nocheck", line });
      }
    }
  }

  return violations;
}

function checkSourceFileWithSeenBaseline(
  filePath: string,
  sourceText: string,
  legacyTsNocheckFiles: ReadonlySet<string>,
  seenLegacyTsNocheckFiles?: Set<string>,
): ConventionViolation[] {
  const extension = extname(filePath).toLowerCase();
  if (!JAVASCRIPT_FAMILY_EXTENSIONS.has(extension)) return [];

  const violations: ConventionViolation[] = [];
  if (!TYPESCRIPT_EXTENSIONS.has(extension)) {
    violations.push({ file: filePath, kind: "invalid-module-extension", line: 1 });
  }

  const sourceFile = ts.createSourceFile(
    filePath,
    sourceText,
    ts.ScriptTarget.Latest,
    true,
    scriptKindForExtension(extension),
  );
  violations.push(
    ...checkTypeScriptSuppressions(
      filePath,
      sourceFile,
      sourceText,
      legacyTsNocheckFiles,
      seenLegacyTsNocheckFiles,
    ),
  );

  function visit(node: ts.Node): void {
    const moduleSpecifier = moduleSpecifierForNode(node);
    if (
      moduleSpecifier &&
      isRelativeModuleSpecifier(moduleSpecifier.text) &&
      EXPLICIT_RELATIVE_EXTENSION.test(moduleSpecifier.text)
    ) {
      violations.push({
        file: filePath,
        kind: "explicit-relative-extension",
        line: lineForNode(sourceFile, moduleSpecifier),
      });
    }
    if (
      ts.isTaggedTemplateExpression(node) &&
      ts.isIdentifier(node.tag) &&
      node.tag.text === "html"
    ) {
      violations.push({
        file: filePath,
        kind: "html-tagged-template",
        line: lineForNode(sourceFile, node),
      });
    }
    ts.forEachChild(node, visit);
  }

  visit(sourceFile);
  return violations.sort(compareViolations);
}

export function checkSourceFile(
  filePath: string,
  sourceText: string,
  legacyTsNocheckFiles: ReadonlySet<string> = new Set(),
): ConventionViolation[] {
  return checkSourceFileWithSeenBaseline(
    filePath,
    sourceText,
    legacyTsNocheckFiles,
  );
}

function sourceFilesUnder(root: string): string[] {
  const files: string[] = [];
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const path = resolve(root, entry.name);
    if (entry.isDirectory()) {
      files.push(...sourceFilesUnder(path));
    } else if (entry.isFile()) {
      files.push(path);
    }
  }
  return files.sort();
}

export function checkSourceTree(
  sourceRoot: string,
  legacyTsNocheckFiles: ReadonlySet<string> = new Set(),
): ConventionViolation[] {
  const root = resolve(sourceRoot);
  const seenLegacyTsNocheckFiles = new Set<string>();
  const violations = sourceFilesUnder(root).flatMap((absolutePath) => {
    const file = relative(root, absolutePath).split(sep).join("/");
    return checkSourceFileWithSeenBaseline(
      file,
      readFileSync(absolutePath, "utf8"),
      legacyTsNocheckFiles,
      seenLegacyTsNocheckFiles,
    );
  });
  for (const file of legacyTsNocheckFiles) {
    if (!seenLegacyTsNocheckFiles.has(file)) {
      violations.push({ file, kind: "stale-ts-nocheck-baseline", line: 1 });
    }
  }
  return violations.sort(compareViolations);
}

const VIOLATION_MESSAGES: Record<ConventionViolationKind, string> = {
  "explicit-relative-extension": "relative module imports must be extensionless",
  "html-tagged-template": "React markup must use JSX instead of html tagged templates",
  "invalid-module-extension": "authored modules must use .ts or .tsx",
  "prohibited-ts-ignore":
    "@ts-ignore is prohibited; fix the type error or use a justified @ts-expect-error",
  "stale-ts-nocheck-baseline":
    "legacy suppression baseline entry has no matching @ts-nocheck directive",
  "unbaselined-ts-nocheck": "@ts-nocheck is not in the legacy suppression baseline",
};

export function formatViolation(violation: ConventionViolation): string {
  return `${violation.file}:${violation.line}: ${VIOLATION_MESSAGES[violation.kind]}`;
}

function runCli(): void {
  const scriptDirectory = dirname(fileURLToPath(import.meta.url));
  const sourceRoot = resolve(scriptDirectory, "../src");
  const baselinePath = resolve(scriptDirectory, "ts-nocheck-baseline.txt");
  const legacyTsNocheckFiles = new Set(
    readFileSync(baselinePath, "utf8")
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter(Boolean),
  );
  const violations = checkSourceTree(sourceRoot, legacyTsNocheckFiles);
  if (violations.length === 0) return;

  for (const violation of violations) {
    console.error(formatViolation(violation));
  }
  console.error(`Found ${violations.length} frontend source convention violation(s).`);
  process.exitCode = 1;
}

const invokedPath = process.argv[1] ? resolve(process.argv[1]) : undefined;
if (invokedPath === fileURLToPath(import.meta.url)) runCli();
