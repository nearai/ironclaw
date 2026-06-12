import { useStore } from "@tanstack/react-form";
import { useQuery } from "@tanstack/react-query";
import {
  Bold,
  CheckSquare,
  Code2,
  FileCode2,
  FileText,
  Heading1,
  Italic,
  Link2,
  List,
  ListOrdered,
  Quote,
} from "lucide-react";
import { type ReactNode, useRef } from "react";
import { Input } from "@/components";
import { Markdown } from "@/components/ui/markdown";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { fetchRepositoryReadme } from "@/lib/repository-content";

export type ProjectFormValues = {
  kind: "project" | "idea";
  title: string;
  description?: string;
  repository?: string;
  content?: string;
  visibility: "private" | "unlisted" | "public";
  status?: "active" | "paused" | "archived";
  ownerId?: string;
  domain?: string;
};

export const validateTitle = (value: string) => {
  const trimmed = value.trim();
  if (!trimmed) return "Title is required";
  if (trimmed.length > 200) return "Max 200 characters";
  return undefined;
};

export const validateDescription = (value?: string) => {
  if ((value ?? "").length > 1000) return "Max 1000 characters";
  return undefined;
};

export const validateRepository = (value: string | undefined, kind: ProjectFormValues["kind"]) => {
  const trimmed = value?.trim() ?? "";
  if (kind === "project" && !trimmed) return "Repository URL is required for projects";
  if (!trimmed) return undefined;
  if (trimmed.length > 500) return "Max 500 characters";
  try {
    new URL(trimmed);
    return undefined;
  } catch {
    return "Must be a valid URL";
  }
};

export const validateContent = (value: string | undefined, kind: ProjectFormValues["kind"]) => {
  const trimmed = value?.trim() ?? "";
  if (kind === "idea" && !trimmed) return "Markdown content is required for ideas";
  if ((value ?? "").length > 50000) return "Max 50,000 characters";
  return undefined;
};

export const validateOptionalMaxLength = (
  value: string | undefined,
  max: number,
  message: string,
) => {
  if ((value ?? "").length > max) return message;
  return undefined;
};

export function fieldError(error: unknown): string | undefined {
  if (!error) return undefined;
  if (Array.isArray(error)) {
    for (const issue of error) {
      const message = fieldError(issue);
      if (message) return message;
    }
    return undefined;
  }
  if (typeof error === "string") return error;
  if (typeof error === "object" && error !== null && "message" in error) {
    return String((error as { message: unknown }).message);
  }
  return String(error);
}

interface ProjectFormLayoutProps {
  form: any;
  mode: "create" | "edit";
  isAdmin: boolean;
  defaultOwnerId: string;
  tab: "write" | "preview";
  setTab: (tab: "write" | "preview") => void;
  slugPreview?: string;
}

export function ProjectFormLayout({
  form,
  mode,
  isAdmin,
  defaultOwnerId,
  tab,
  setTab,
  slugPreview,
}: ProjectFormLayoutProps) {
  const kind = useStore(form.store, (s: any) => s.values.kind);
  const repositoryUrl = useStore(form.store, (s: any) => s.values.repository ?? "");

  const readmeQuery = useQuery({
    queryKey: ["projectFormReadme", repositoryUrl],
    queryFn: async () => {
      if (!repositoryUrl.trim()) return null;
      return await fetchRepositoryReadme(repositoryUrl.trim());
    },
    enabled: kind === "project" && Boolean(repositoryUrl?.trim()),
  });

  return (
    <div className="flex flex-1 flex-col overflow-visible lg:min-h-0 lg:overflow-hidden lg:flex-row">
      <div
        style={{ width: mode === "edit" ? undefined : undefined, flexShrink: 0 }}
        className="overflow-visible border-b border-border bg-card px-4 py-5 sm:px-6 sm:py-6 lg:overflow-y-auto lg:w-[340px] lg:border-b-0 lg:border-r xl:w-[360px]"
      >
        <div className="space-y-6 pb-[env(safe-area-inset-bottom,0px)] lg:pb-0">
          <form.Field name="kind">
            {(field: any) => (
              <div className="space-y-2">
                <FormLabel>Type</FormLabel>
                <div className="grid grid-cols-2 gap-2">
                  {(
                    [
                      {
                        value: "project" as const,
                        label: "Project",
                        icon: <FileCode2 size={15} />,
                      },
                      { value: "idea" as const, label: "Idea", icon: <FileText size={15} /> },
                    ] as const
                  ).map((opt) => {
                    const active = field.state.value === opt.value;
                    return (
                      <button
                        key={opt.value}
                        type="button"
                        onClick={() => {
                          field.handleChange(opt.value);
                          if (mode === "create" && tab === "preview") setTab("write");
                        }}
                        className={`${active ? "border-2 border-brand-accent bg-brand-accent-light" : "border border-border bg-card"}`}
                        style={{
                          padding: mode === "edit" ? "10px 12px" : 12,
                          borderRadius: 10,
                          display: "flex",
                          alignItems: "center",
                          gap: 6,
                          textAlign: "left",
                          cursor: "pointer",
                          transition: "all 0.12s",
                        }}
                      >
                        {opt.icon}
                        <span
                          className="text-foreground"
                          style={{
                            fontWeight: 700,
                            fontSize: mode === "edit" ? 13 : 14,
                          }}
                        >
                          {opt.label}
                        </span>
                      </button>
                    );
                  })}
                </div>
              </div>
            )}
          </form.Field>

          <form.Field
            name="title"
            validators={{
              onChange: ({ value }: any) => validateTitle(value),
              onSubmit: ({ value }: any) => validateTitle(value),
            }}
          >
            {(field: any) => {
              const err = fieldError(field.state.meta.errors[0]);
              return (
                <div className="space-y-1.5">
                  <FormLabel htmlFor="title">Title</FormLabel>
                  <Input
                    id="title"
                    value={field.state.value}
                    onChange={(e) => field.handleChange(e.target.value)}
                    placeholder={kind === "project" ? "near analytics" : "On-chain social graphs"}
                    className={err ? "!border-destructive" : ""}
                  />
                  {err && <ErrorText>{err}</ErrorText>}
                  {slugPreview !== undefined && (
                    <p className="text-muted-foreground" style={{ fontSize: 12, marginTop: 2 }}>
                      /{slugPreview || "my-entry"}
                    </p>
                  )}
                </div>
              );
            }}
          </form.Field>

          <form.Field
            name="description"
            validators={{
              onChange: ({ value }: any) => validateDescription(value),
              onSubmit: ({ value }: any) => validateDescription(value),
            }}
          >
            {(field: any) => {
              const err = fieldError(field.state.meta.errors[0]);
              return (
                <div className="space-y-1.5">
                  <FormLabel htmlFor="description">Description</FormLabel>
                  <NearTextarea
                    id="description"
                    value={field.state.value ?? ""}
                    onChange={(v) => field.handleChange(v)}
                    placeholder="A short summary shown in the list"
                    rows={3}
                    error={!!err}
                  />
                  {err && <ErrorText>{err}</ErrorText>}
                </div>
              );
            }}
          </form.Field>

          {kind === "project" && (
            <form.Field
              name="repository"
              validators={{
                onChangeListenTo: ["kind"],
                onChange: ({ value, fieldApi }: any) =>
                  validateRepository(value, fieldApi.form.getFieldValue("kind")),
                onSubmit: ({ value, fieldApi }: any) =>
                  validateRepository(value, fieldApi.form.getFieldValue("kind")),
              }}
            >
              {(field: any) => {
                const err = fieldError(field.state.meta.errors[0]);
                return (
                  <div className="space-y-1.5">
                    <FormLabel htmlFor="repository">Repository URL</FormLabel>
                    <Input
                      id="repository"
                      type="url"
                      value={field.state.value ?? ""}
                      onChange={(e) => field.handleChange(e.target.value)}
                      placeholder="https://github.com/user/repo"
                      className={`font-mono text-sm ${err ? "!border-destructive" : ""}`}
                    />
                    {err && <ErrorText>{err}</ErrorText>}
                    <HelperText>README fetched from the default branch.</HelperText>
                  </div>
                );
              }}
            </form.Field>
          )}

          {mode === "edit" && (
            <form.Field name="status">
              {(field: any) => (
                <div className="space-y-2">
                  <FormLabel>Status</FormLabel>
                  <NearSelect
                    value={field.state.value ?? "active"}
                    options={[
                      { value: "active", label: "Active" },
                      { value: "paused", label: "Paused" },
                      { value: "archived", label: "Archived" },
                    ]}
                    onChange={(v) => field.handleChange(v)}
                  />
                </div>
              )}
            </form.Field>
          )}

          <form.Field name="visibility">
            {(field: any) => (
              <div className="space-y-2">
                <FormLabel>Visibility</FormLabel>
                <div className="flex flex-col gap-2">
                  {(
                    [
                      { value: "public" as const, label: "Public", desc: "Visible in the feed" },
                      {
                        value: "unlisted" as const,
                        label: "Unlisted",
                        desc: "Only via direct link",
                      },
                      { value: "private" as const, label: "Private", desc: "Only you" },
                    ] as const
                  ).map((opt) => {
                    const active = field.state.value === opt.value;
                    return (
                      <button
                        key={opt.value}
                        type="button"
                        onClick={() => field.handleChange(opt.value)}
                        className={`${active ? "border-2 border-brand-accent bg-brand-accent-light" : "border border-border bg-card"}`}
                        style={{
                          padding: "10px 14px",
                          borderRadius: 10,
                          display: "flex",
                          alignItems: "center",
                          justifyContent: "space-between",
                          cursor: "pointer",
                          transition: "all 0.12s",
                        }}
                      >
                        <span className="text-foreground" style={{ fontSize: 14, fontWeight: 700 }}>
                          {opt.label}
                        </span>
                        <span className="text-muted-foreground" style={{ fontSize: 12 }}>
                          {opt.desc}
                        </span>
                      </button>
                    );
                  })}
                </div>
              </div>
            )}
          </form.Field>

          {kind === "idea" && (
            <form.Field
              name="domain"
              validators={{
                onChange: ({ value }: any) =>
                  validateOptionalMaxLength(value, 255, "Max 255 characters"),
                onSubmit: ({ value }: any) =>
                  validateOptionalMaxLength(value, 255, "Max 255 characters"),
              }}
            >
              {(field: any) => {
                const err = fieldError(field.state.meta.errors[0]);
                return (
                  <div className="space-y-1.5">
                    <FormLabel htmlFor="domain">Domain</FormLabel>
                    <Input
                      id="domain"
                      value={field.state.value ?? ""}
                      onChange={(e) => field.handleChange(e.target.value)}
                      placeholder="example.com"
                      className={`font-mono text-sm ${err ? "!border-destructive" : ""}`}
                    />
                    {err && <ErrorText>{err}</ErrorText>}
                    <HelperText>Already have a domain to use?</HelperText>
                  </div>
                );
              }}
            </form.Field>
          )}

          {isAdmin && (
            <form.Field
              name="ownerId"
              validators={{
                onChange: ({ value }: any) =>
                  validateOptionalMaxLength(value, 255, "Max 255 characters"),
                onSubmit: ({ value }: any) =>
                  validateOptionalMaxLength(value, 255, "Max 255 characters"),
              }}
            >
              {(field: any) => {
                const err = fieldError(field.state.meta.errors[0]);
                return (
                  <div className="space-y-1.5">
                    <FormLabel htmlFor="ownerId">Owner</FormLabel>
                    <Input
                      id="ownerId"
                      value={field.state.value ?? ""}
                      onChange={(e) => field.handleChange(e.target.value)}
                      placeholder={defaultOwnerId}
                      className={`font-mono text-sm ${err ? "!border-destructive" : ""}`}
                    />
                    {err && <ErrorText>{err}</ErrorText>}
                    <HelperText>NEAR account that owns this entry. Defaults to you.</HelperText>
                  </div>
                );
              }}
            </form.Field>
          )}
        </div>
      </div>

      <div className="flex flex-col overflow-visible bg-muted lg:min-w-0 lg:flex-1 lg:overflow-hidden">
        {kind === "idea" ? (
          <form.Field
            name="content"
            validators={{
              onChangeListenTo: ["kind"],
              onChange: ({ value, fieldApi }: any) =>
                validateContent(value, fieldApi.form.getFieldValue("kind")),
              onSubmit: ({ value, fieldApi }: any) =>
                validateContent(value, fieldApi.form.getFieldValue("kind")),
            }}
          >
            {(field: any) => {
              const err = fieldError(field.state.meta.errors[0]);
              return (
                <>
                  <div className="hidden min-h-0 flex-1 lg:grid lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
                    <div className="min-h-0 border-r border-border bg-card">
                      <ContentWriteTab
                        value={field.state.value ?? ""}
                        onChange={field.handleChange}
                        error={err}
                      />
                    </div>
                    <MarkdownPreviewPanel content={field.state.value ?? ""} />
                  </div>

                  <Tabs
                    value={tab}
                    onValueChange={(v) => setTab(v as "write" | "preview")}
                    className="flex flex-col overflow-visible gap-0 lg:hidden"
                  >
                    <div className="shrink-0 border-b border-border bg-card">
                      <TabsList
                        style={{
                          border: "none",
                          height: "auto",
                          padding: "0 12px",
                          display: "flex",
                          justifyContent: "flex-start",
                          gap: 0,
                          width: "auto",
                        }}
                        className="bg-transparent border-b-0"
                      >
                        {(["write", "preview"] as const).map((t) => (
                          <TabsTrigger
                            key={t}
                            value={t}
                            className={`${tab === t ? "border-b-2 border-brand-accent text-foreground" : "border-b-2 border-transparent text-muted-foreground"}`}
                            style={{
                              padding: "14px 20px",
                              fontSize: 13,
                              fontWeight: 600,
                              borderRadius: 0,
                              marginBottom: -1,
                              transition: "all 0.12s",
                              minHeight: 44,
                              WebkitTapHighlightColor: "transparent",
                            }}
                          >
                            {t === "write" ? "Write" : "Preview"}
                          </TabsTrigger>
                        ))}
                      </TabsList>
                    </div>

                    <TabsContent value="write" className="m-0 overflow-visible">
                      <ContentWriteTab
                        value={field.state.value ?? ""}
                        onChange={field.handleChange}
                        error={err}
                      />
                    </TabsContent>

                    <TabsContent value="preview" className="m-0 overflow-visible">
                      <MarkdownPreviewPanel content={field.state.value ?? ""} compact />
                    </TabsContent>
                  </Tabs>
                </>
              );
            }}
          </form.Field>
        ) : (
          <div className="flex flex-col overflow-visible lg:min-h-0 lg:flex-1 lg:overflow-hidden">
            {repositoryUrl.trim() ? (
              <>
                <div className="flex shrink-0 items-center gap-2 border-b border-border bg-card px-4 py-3 sm:px-6">
                  <FileCode2 size={14} className="text-muted-foreground" />
                  <span className="text-foreground" style={{ fontSize: 13, fontWeight: 600 }}>
                    README Preview
                  </span>
                  {readmeQuery.isLoading && (
                    <span className="text-muted-foreground" style={{ fontSize: 12 }}>
                      Loading…
                    </span>
                  )}
                </div>
                <div className="overflow-visible px-4 py-5 sm:px-8 sm:py-6 lg:min-h-0 lg:flex-1 lg:overflow-y-auto">
                  {readmeQuery.isLoading ? (
                    <div className="text-muted-foreground" style={{ fontSize: 14 }}>
                      Loading README…
                    </div>
                  ) : readmeQuery.data ? (
                    <Markdown content={readmeQuery.data} />
                  ) : (
                    <div
                      className="border border-dashed border-border text-muted-foreground"
                      style={{
                        padding: "32px 24px",
                        borderRadius: 12,
                        textAlign: "center",
                        fontSize: 14,
                      }}
                    >
                      No README available for this repository.
                    </div>
                  )}
                </div>
              </>
            ) : (
              <div className="flex flex-1 flex-col items-center justify-center gap-3 p-6 text-center sm:p-8">
                <FileCode2 size={40} className="text-border" />
                <p className="text-foreground" style={{ fontSize: 15, fontWeight: 600 }}>
                  Set a repository URL
                </p>
                <p className="text-muted-foreground" style={{ fontSize: 13, maxWidth: 280 }}>
                  The project detail page will fetch and render the README from the default branch.
                </p>
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function ContentWriteTab({
  value,
  onChange,
  error,
}: {
  value: string;
  onChange: (v: string) => void;
  error?: string;
}) {
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const applyMarkdown = (tool: MarkdownTool) => {
    const textarea = textareaRef.current;
    if (!textarea) return;

    const start = textarea.selectionStart ?? value.length;
    const end = textarea.selectionEnd ?? value.length;
    const selectedText = value.slice(start, end);
    const nextValue = tool.apply(value, selectedText, start, end);
    onChange(nextValue.text);

    requestAnimationFrame(() => {
      textarea.focus();
      textarea.setSelectionRange(nextValue.selectionStart, nextValue.selectionEnd);
    });
  };

  return (
    <div className="flex min-h-[55vh] flex-col lg:h-full lg:min-h-0">
      <div className="flex flex-wrap items-center gap-1.5 border-b border-border bg-card px-3 py-2.5 sm:px-4">
        {MARKDOWN_TOOLS.map((tool) => {
          const Icon = tool.icon;
          return (
            <button
              key={tool.label}
              type="button"
              onClick={() => applyMarkdown(tool)}
              title={tool.label}
              className="inline-flex h-10 w-10 items-center justify-center rounded-lg border border-border bg-background text-muted-foreground transition-colors hover:border-brand-accent hover:bg-brand-accent-light hover:text-foreground active:scale-95"
              style={{ WebkitTapHighlightColor: "transparent" }}
            >
              <Icon size={14} />
            </button>
          );
        })}
      </div>
      <textarea
        ref={textareaRef}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={"# My Idea\n\nDescribe the concept, motivation, and next steps…"}
        className={`text-foreground bg-muted border-none outline-none resize-none ${error ? "border-t-2 border-destructive" : ""}`}
        style={{
          flex: 1,
          width: "100%",
          minHeight: 320,
          padding: "20px 20px 24px",
          fontFamily: "ui-monospace, SFMono-Regular, monospace",
          fontSize: 13,
          lineHeight: "1.65",
        }}
      />
      {error && (
        <div
          className="bg-status-danger-surface text-destructive border-t border-status-danger-border"
          style={{
            padding: "8px 32px",
            fontSize: 12,
            flexShrink: 0,
          }}
        >
          {error}
        </div>
      )}
    </div>
  );
}

function MarkdownPreviewPanel({ content, compact }: { content: string; compact?: boolean }) {
  return (
    <div className="flex min-h-[55vh] flex-col overflow-visible lg:min-h-0 lg:flex-1 lg:overflow-hidden">
      <div className="flex shrink-0 items-center gap-2 border-b border-border bg-card px-4 py-3 sm:px-6">
        <FileText size={14} className="text-muted-foreground" />
        <span className="text-foreground" style={{ fontSize: 13, fontWeight: 600 }}>
          Live Preview
        </span>
      </div>
      <div
        className={`${compact ? "px-4 py-5 sm:px-6" : "px-4 py-5 sm:px-8 sm:py-6"} overflow-visible lg:min-h-0 lg:flex-1 lg:overflow-y-auto`}
      >
        {content.trim() ? (
          <Markdown content={content} />
        ) : (
          <div
            className="border border-dashed border-border text-muted-foreground"
            style={{
              padding: "32px 24px",
              borderRadius: 12,
              textAlign: "center",
              fontSize: 14,
            }}
          >
            Start writing to see the preview.
          </div>
        )}
      </div>
    </div>
  );
}

type MarkdownToolResult = {
  text: string;
  selectionStart: number;
  selectionEnd: number;
};

type MarkdownTool = {
  label: string;
  icon: typeof Bold;
  apply: (value: string, selection: string, start: number, end: number) => MarkdownToolResult;
};

function replaceSelection(
  value: string,
  start: number,
  end: number,
  replacement: string,
  selectionStart = start,
  selectionEnd = start + replacement.length,
): MarkdownToolResult {
  return {
    text: `${value.slice(0, start)}${replacement}${value.slice(end)}`,
    selectionStart,
    selectionEnd,
  };
}

function wrapSelection(
  value: string,
  selection: string,
  start: number,
  end: number,
  before: string,
  after: string,
  fallback: string,
): MarkdownToolResult {
  const inner = selection || fallback;
  const replacement = `${before}${inner}${after}`;
  const cursorStart = start + before.length;
  return replaceSelection(value, start, end, replacement, cursorStart, cursorStart + inner.length);
}

const MARKDOWN_TOOLS: MarkdownTool[] = [
  {
    label: "Heading",
    icon: Heading1,
    apply: (value, selection, start, end) => {
      const inner = selection || "Heading";
      const replacement = `# ${inner}`;
      return replaceSelection(
        value,
        start,
        end,
        replacement,
        start + 2,
        start + replacement.length,
      );
    },
  },
  {
    label: "Bold",
    icon: Bold,
    apply: (value, selection, start, end) =>
      wrapSelection(value, selection, start, end, "**", "**", "bold text"),
  },
  {
    label: "Italic",
    icon: Italic,
    apply: (value, selection, start, end) =>
      wrapSelection(value, selection, start, end, "*", "*", "italic text"),
  },
  {
    label: "Link",
    icon: Link2,
    apply: (value, selection, start, end) =>
      wrapSelection(value, selection, start, end, "[", "](https://)", "link text"),
  },
  {
    label: "Code",
    icon: Code2,
    apply: (value, selection, start, end) =>
      wrapSelection(value, selection, start, end, "`", "`", "code"),
  },
  {
    label: "Quote",
    icon: Quote,
    apply: (value, selection, start, end) => {
      const inner = selection || "Quoted thought";
      const replacement = inner
        .split("\n")
        .map((line) => `> ${line}`)
        .join("\n");
      return replaceSelection(
        value,
        start,
        end,
        replacement,
        start + 2,
        start + replacement.length,
      );
    },
  },
  {
    label: "Bullet List",
    icon: List,
    apply: (value, selection, start, end) => {
      const inner = selection || "List item";
      const replacement = inner
        .split("\n")
        .map((line) => `- ${line}`)
        .join("\n");
      return replaceSelection(
        value,
        start,
        end,
        replacement,
        start + 2,
        start + replacement.length,
      );
    },
  },
  {
    label: "Numbered List",
    icon: ListOrdered,
    apply: (value, selection, start, end) => {
      const inner = selection || "List item";
      const replacement = inner
        .split("\n")
        .map((line, index) => `${index + 1}. ${line}`)
        .join("\n");
      return replaceSelection(
        value,
        start,
        end,
        replacement,
        start + 3,
        start + replacement.length,
      );
    },
  },
  {
    label: "Checklist",
    icon: CheckSquare,
    apply: (value, selection, start, end) => {
      const inner = selection || "Checklist item";
      const replacement = inner
        .split("\n")
        .map((line) => `- [ ] ${line}`)
        .join("\n");
      return replaceSelection(
        value,
        start,
        end,
        replacement,
        start + 6,
        start + replacement.length,
      );
    },
  },
];

export function FormLabel({ children, htmlFor }: { children: ReactNode; htmlFor?: string }) {
  return (
    <label
      htmlFor={htmlFor}
      className="text-foreground block"
      style={{ fontSize: 14, fontWeight: 700, marginBottom: 6 }}
    >
      {children}
    </label>
  );
}

export function HelperText({ children }: { children: ReactNode }) {
  return (
    <p className="text-muted-foreground" style={{ fontSize: 12, marginTop: 4 }}>
      {children}
    </p>
  );
}

export function ErrorText({ children }: { children: ReactNode }) {
  return (
    <p className="text-destructive" style={{ fontSize: 12, marginTop: 4 }}>
      {children}
    </p>
  );
}

export function NearTextarea({
  id,
  value,
  onChange,
  placeholder,
  rows = 4,
  error,
}: {
  id?: string;
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  rows?: number;
  error?: boolean;
}) {
  return (
    <textarea
      id={id}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={placeholder}
      rows={rows}
      className={`text-foreground bg-card border-border focus:border-brand-accent ${error ? "border-destructive focus:border-destructive" : ""}`}
      style={{
        width: "100%",
        padding: "10px 14px",
        borderRadius: 8,
        fontSize: 14,
        outline: "none",
        resize: "vertical",
        fontFamily: "inherit",
        lineHeight: "1.5",
        transition: "border-color 0.12s, box-shadow 0.12s",
      }}
      onFocus={(e) => {
        const el = e.currentTarget as HTMLTextAreaElement;
        el.style.boxShadow = error
          ? "0 0 0 3px rgba(229,47,40,0.1)"
          : "0 0 0 3px rgba(0,217,163,0.1)";
      }}
      onBlur={(e) => {
        const el = e.currentTarget as HTMLTextAreaElement;
        el.style.boxShadow = "none";
      }}
    />
  );
}

export function NearSelect({
  value,
  options,
  onChange,
}: {
  value: string;
  options: { value: string; label: string }[];
  onChange: (v: string) => void;
}) {
  return (
    <select
      className="border border-border focus:border-brand-accent text-foreground bg-card"
      value={value}
      onChange={(e) => onChange(e.target.value)}
      style={{
        width: "100%",
        height: 40,
        padding: "0 14px",
        borderRadius: 8,
        fontSize: 14,
        outline: "none",
        cursor: "pointer",
        appearance: "auto",
      }}
    >
      {options.map((opt) => (
        <option key={opt.value} value={opt.value}>
          {opt.label}
        </option>
      ))}
    </select>
  );
}
