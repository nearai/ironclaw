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
const SINGLE_LINE_CHECK_PRAGMA =
  /^\/\/\/?\s*@(ts-check|ts-nocheck)(?:(?:[^\S\r\n]|:).*)?$/i;

type ParsedCommentDirective = {
  range: ts.TextRange;
  type: number;
};

type SourceFileWithCommentDirectives = ts.SourceFile & {
  commentDirectives?: readonly ParsedCommentDirective[];
};

// TypeScript's public declarations do not expose CommentDirectiveType, but the
// pinned compiler represents ExpectError as 0 and Ignore as 1. Reading the
// parser-populated directives keeps recognition identical to the compiler.
const TYPESCRIPT_IGNORE_DIRECTIVE = 1;

export type ConventionViolationKind =
  | "explicit-relative-extension"
  | "html-tagged-template"
  | "invalid-module-extension"
  | "prohibited-ts-ignore"
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

function effectiveNocheckPosition(sourceText: string): number | undefined {
  let position: number | undefined;
  for (const range of ts.getLeadingCommentRanges(sourceText, 0) ?? []) {
    if (range.kind !== ts.SyntaxKind.SingleLineCommentTrivia) continue;
    const comment = sourceText.slice(range.pos, range.end);
    const match = SINGLE_LINE_CHECK_PRAGMA.exec(comment);
    if (!match) continue;
    position =
      match[1].toLowerCase() === "ts-nocheck"
        ? range.pos + comment.indexOf("@")
        : undefined;
  }
  return position;
}

function checkTypeScriptSuppressions(
  filePath: string,
  sourceFile: ts.SourceFile,
  sourceText: string,
  legacyTsNocheckFiles: ReadonlySet<string>,
): ConventionViolation[] {
  const violations: ConventionViolation[] = [];
  const nocheckPosition = effectiveNocheckPosition(sourceText);
  if (nocheckPosition !== undefined) {
    const line =
      sourceFile.getLineAndCharacterOfPosition(nocheckPosition).line + 1;
    if (!legacyTsNocheckFiles.has(filePath)) {
      violations.push({ file: filePath, kind: "unbaselined-ts-nocheck", line });
    }
  }

  const commentDirectives =
    (sourceFile as SourceFileWithCommentDirectives).commentDirectives ?? [];
  for (const directive of commentDirectives) {
    if (directive.type !== TYPESCRIPT_IGNORE_DIRECTIVE) continue;
    const line =
      sourceFile.getLineAndCharacterOfPosition(directive.range.pos).line + 1;
    violations.push({ file: filePath, kind: "prohibited-ts-ignore", line });
  }

  return violations;
}

function checkSourceFileContent(
  filePath: string,
  sourceText: string,
  legacyTsNocheckFiles: ReadonlySet<string>,
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
    ...checkTypeScriptSuppressions(filePath, sourceFile, sourceText, legacyTsNocheckFiles),
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
  return checkSourceFileContent(filePath, sourceText, legacyTsNocheckFiles);
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
  const violations = sourceFilesUnder(root).flatMap((absolutePath) => {
    const file = relative(root, absolutePath).split(sep).join("/");
    return checkSourceFileContent(
      file,
      readFileSync(absolutePath, "utf8"),
      legacyTsNocheckFiles,
    );
  });
  return violations.sort(compareViolations);
}

function typeScriptProjectFiles(configPath: string): string[] {
  const configResult = ts.readConfigFile(configPath, ts.sys.readFile);
  if (configResult.error) {
    throw new Error(
      ts.formatDiagnostic(configResult.error, {
        getCanonicalFileName: (fileName) => fileName,
        getCurrentDirectory: ts.sys.getCurrentDirectory,
        getNewLine: () => ts.sys.newLine,
      }),
    );
  }

  const parsed = ts.parseJsonConfigFileContent(
    configResult.config,
    ts.sys,
    dirname(configPath),
    undefined,
    configPath,
  );
  if (parsed.errors.length > 0) {
    throw new Error(
      ts.formatDiagnostics(parsed.errors, {
        getCanonicalFileName: (fileName) => fileName,
        getCurrentDirectory: ts.sys.getCurrentDirectory,
        getNewLine: () => ts.sys.newLine,
      }),
    );
  }
  return parsed.fileNames;
}

export function checkTypeScriptProject(
  projectRoot: string,
  legacyTsNocheckFiles: ReadonlySet<string> = new Set(),
): ConventionViolation[] {
  const root = resolve(projectRoot);
  const sourceRoot = resolve(root, "src");
  const configPath = resolve(root, "tsconfig.json");
  const authoredFiles = new Set([
    ...sourceFilesUnder(sourceRoot),
    ...typeScriptProjectFiles(configPath),
  ]);
  const violations = [...authoredFiles].flatMap((absolutePath) => {
    const file = relative(root, absolutePath).split(sep).join("/");
    return checkSourceFileContent(
      file,
      readFileSync(absolutePath, "utf8"),
      legacyTsNocheckFiles,
    );
  });
  return violations.sort(compareViolations);
}

const VIOLATION_MESSAGES: Record<ConventionViolationKind, string> = {
  "explicit-relative-extension": "relative module imports must be extensionless",
  "html-tagged-template": "React markup must use JSX instead of html tagged templates",
  "invalid-module-extension": "authored modules must use .ts or .tsx",
  "prohibited-ts-ignore":
    "@ts-ignore is prohibited; fix the type error or use a justified @ts-expect-error",
  "unbaselined-ts-nocheck": "@ts-nocheck is not in the legacy suppression baseline",
};

export function formatViolation(violation: ConventionViolation): string {
  return `${violation.file}:${violation.line}: ${VIOLATION_MESSAGES[violation.kind]}`;
}

function runCli(): void {
  const scriptDirectory = dirname(fileURLToPath(import.meta.url));
  const projectRoot = resolve(scriptDirectory, "..");
  const baselinePath = resolve(scriptDirectory, "ts-nocheck-baseline.txt");
  const legacyTsNocheckFiles = new Set(
    readFileSync(baselinePath, "utf8")
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter(Boolean)
      .map((file) => `src/${file}`),
  );
  const violations = checkTypeScriptProject(projectRoot, legacyTsNocheckFiles);
  if (violations.length === 0) return;

  for (const violation of violations) {
    console.error(formatViolation(violation));
  }
  console.error(`Found ${violations.length} frontend source convention violation(s).`);
  process.exitCode = 1;
}

const invokedPath = process.argv[1] ? resolve(process.argv[1]) : undefined;
if (invokedPath === fileURLToPath(import.meta.url)) runCli();
