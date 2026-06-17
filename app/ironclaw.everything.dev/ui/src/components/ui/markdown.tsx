import { Copy } from "lucide-react";
import { useState } from "react";
import type { Components } from "react-markdown";
import ReactMarkdown from "react-markdown";
import rehypeHighlight from "rehype-highlight";
import rehypeSanitize, { defaultSchema } from "rehype-sanitize";
import rehypeSlug from "rehype-slug";
import remarkGfm from "remark-gfm";
import { cn } from "@/lib/utils";

interface MarkdownProps {
  content: string;
  className?: string;
}

const sanitizeSchema = {
  ...defaultSchema,
  tagNames: [
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "p",
    "a",
    "strong",
    "em",
    "del",
    "br",
    "hr",
    "ul",
    "ol",
    "li",
    "blockquote",
    "pre",
    "code",
    "table",
    "thead",
    "tbody",
    "tr",
    "th",
    "td",
    "img",
    "input",
    "span",
  ],
  attributes: {
    ...defaultSchema.attributes,
    a: ["href"],
    img: ["src", "alt"],
    code: ["className"],
    input: ["type", "checked", "disabled"],
    span: ["className", "id"],
    pre: ["className"],
    td: ["align"],
    th: ["align"],
    "*": ["id"],
  },
};

export function Markdown({ content, className }: MarkdownProps) {
  return (
    <article
      className={cn(
        "max-w-none",
        /* headings */
        "[&_h1]:mt-10 [&_h1]:mb-4 [&_h1]:text-[1.875rem] [&_h1]:font-semibold [&_h1]:tracking-tight [&_h1]:text-foreground first:[&>h1]:mt-0",
        "[&_h2]:mt-8 [&_h2]:mb-3 [&_h2]:text-2xl [&_h2]:font-semibold [&_h2]:tracking-tight [&_h2]:text-foreground [&_h2]:pb-2 [&_h2]:border-b [&_h2]:border-border",
        "[&_h3]:mt-6 [&_h3]:mb-2 [&_h3]:text-lg [&_h3]:font-semibold [&_h3]:text-foreground",
        "[&_h4]:mt-5 [&_h4]:mb-2 [&_h4]:text-base [&_h4]:font-semibold [&_h4]:text-foreground",
        "[&_h5]:mt-4 [&_h5]:mb-1 [&_h5]:text-sm [&_h5]:font-semibold [&_h5]:text-foreground",
        "[&_h6]:mt-4 [&_h6]:mb-1 [&_h6]:text-sm [&_h6]:font-semibold [&_h6]:text-muted-foreground",
        /* body */
        "[&_p]:mb-4 [&_p]:text-base [&_p]:leading-7 [&_p]:text-foreground",
        /* links */
        "[&_a]:text-brand-accent [&_a]:font-medium [&_a]:underline-offset-4 hover:[&_a]:underline",
        /* lists */
        "[&_ul]:mb-4 [&_ul]:list-disc [&_ul]:pl-6 [&_ul]:text-foreground",
        "[&_ol]:mb-4 [&_ol]:list-decimal [&_ol]:pl-6 [&_ol]:text-foreground",
        "[&_li]:mb-1.5 [&_li]:leading-7",
        "[&_li>ul]:mt-1.5 [&_li>ol]:mt-1.5 [&_li>ul]:mb-0 [&_li>ol]:mb-0",
        /* task lists */
        "[&_input[type=checkbox]]:mr-2 [&_input[type=checkbox]]:accent-brand-accent",
        /* blockquote */
        "[&_blockquote]:mb-4 [&_blockquote]:border-l-4 [&_blockquote]:border-brand-accent [&_blockquote]:pl-4 [&_blockquote]:py-1 [&_blockquote]:bg-muted [&_blockquote]:rounded-r-md [&_blockquote]:text-muted-foreground [&_blockquote]:not-italic",
        /* hr */
        "[&_hr]:my-8 [&_hr]:border-border",
        /* images */
        "[&_img]:max-w-full [&_img]:rounded-xl [&_img]:my-4",
        /* strong / em */
        "[&_strong]:font-semibold [&_strong]:text-foreground",
        "[&_em]:italic",
        /* strikethrough */
        "[&_del]:line-through [&_del]:text-muted-foreground",
        /* footnotes */
        "[&_.footnotes]:mt-8 [&_.footnotes]:pt-4 [&_.footnotes]:border-t [&_.footnotes]:border-border [&_.footnotes]:text-sm [&_.footnotes]:text-muted-foreground",
        className,
      )}
    >
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[rehypeSanitize(sanitizeSchema) as any, rehypeHighlight, rehypeSlug]}
        components={markdownComponents}
      >
        {content}
      </ReactMarkdown>
    </article>
  );
}

function extractCodeText(children: React.ReactNode): string {
  let result = "";
  const walk = (node: React.ReactNode) => {
    if (typeof node === "string") {
      result += node;
    } else if (Array.isArray(node)) {
      node.forEach(walk);
    } else if (node && typeof node === "object" && "props" in node) {
      const el = node as React.ReactElement<{ children?: React.ReactNode }>;
      if (el.props?.children) walk(el.props.children);
    }
  };
  walk(children);
  return result;
}

const markdownComponents: Components = {
  code: ({ className, children, ...props }) => {
    const isBlock = className?.startsWith("language-");

    if (!isBlock) {
      return (
        <code
          className="bg-secondary border border-border rounded px-1.5 py-0.5 text-foreground font-mono text-[0.85em]"
          {...props}
        >
          {children}
        </code>
      );
    }

    return (
      <code
        className={cn(className, "block min-w-full")}
        style={{ fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace", fontSize: 13 }}
        {...props}
      >
        {children}
      </code>
    );
  },

  pre: ({ children }) => {
    const [copied, setCopied] = useState(false);
    return (
      <div className="group relative mb-5">
        <button
          type="button"
          onClick={async () => {
            const text = extractCodeText(children);
            await navigator.clipboard.writeText(text);
            setCopied(true);
            setTimeout(() => setCopied(false), 1500);
          }}
          className="absolute right-2 top-2 z-10 rounded-md border border-border bg-background p-1.5 opacity-0 transition-opacity group-hover:opacity-100 hover:bg-muted"
          title={copied ? "Copied!" : "Copy code"}
        >
          <Copy size={12} className={cn(copied ? "text-[color:var(--near-green)]" : "text-muted-foreground")} />
        </button>
        <pre className="bg-muted border border-border rounded-[10px] p-5 overflow-x-auto leading-relaxed">
          {children}
        </pre>
      </div>
    );
  },

  a: ({ href, children }) => (
    <a href={href ?? "#"} target="_blank" rel="noopener noreferrer">
      {children}
    </a>
  ),

  img: ({ src, alt }) => (
    <img
      src={src}
      alt={alt ?? ""}
      style={{ maxWidth: "100%", borderRadius: 10, margin: "16px 0", display: "block" }}
    />
  ),

  table: ({ children }) => (
    <div style={{ overflowX: "auto", marginBottom: 20 }}>
      <table className="min-w-full border-collapse text-sm text-left">{children}</table>
    </div>
  ),

  thead: ({ children }) => <thead className="bg-muted border-b-2 border-border">{children}</thead>,

  th: ({ children }) => (
    <th className="px-4 py-2.5 font-bold text-[13px] text-foreground border-b border-border whitespace-nowrap">
      {children}
    </th>
  ),

  td: ({ children }) => (
    <td className="px-4 py-2.5 border-b border-border text-foreground text-sm align-top">
      {children}
    </td>
  ),

  tr: ({ children }) => (
    <tr className="transition-colors duration-100 hover:bg-muted">{children}</tr>
  ),

  blockquote: ({ children }) => (
    <blockquote className="border-l-4 border-brand-accent pl-4 pt-1 pb-1 mb-4 bg-muted rounded-r-lg text-muted-foreground">
      {children}
    </blockquote>
  ),

  hr: () => <hr className="border-none border-t border-border my-8" />,
};
