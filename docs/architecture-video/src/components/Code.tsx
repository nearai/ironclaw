import { COLORS, FONTS } from "../theme";

type Token = { text: string; color?: string };

// Very simple tokenizer - colors keywords, strings, comments, types
export const highlight = (code: string): Token[][] => {
  const keywords = new Set([
    "pub", "fn", "async", "await", "let", "mut", "struct", "enum", "impl",
    "trait", "for", "match", "if", "else", "return", "use", "mod", "self",
    "Self", "dyn", "Box", "Vec", "Option", "Result", "as", "in", "while",
    "loop", "break", "continue", "const", "static", "type", "where",
    "def", "class", "import", "from", "None", "True", "False", "await",
    "try", "except", "with", "yield", "lambda", "global", "nonlocal",
  ]);
  const types = new Set([
    "Thread", "Step", "Capability", "MemoryDoc", "Project", "ThreadId",
    "StepId", "String", "u32", "u64", "usize", "bool", "i32", "f64",
    "LlmResponse", "ActionCall", "ActionResult", "ThreadEvent", "Uuid",
    "ExecutionLoop", "ThreadManager", "CapabilityRegistry", "LeaseManager",
    "PolicyEngine", "Store", "IncomingMessage", "OutgoingResponse",
    "MessageStream", "Channel", "Tool", "LlmProvider", "Database",
    "DispatchSource", "ToolDispatcher", "SafetyLayer", "ActionRecord",
    "ExecutionTier", "ThreadState", "ThreadConfig",
  ]);

  return code.split("\n").map((line) => {
    const tokens: Token[] = [];
    let i = 0;
    while (i < line.length) {
      // Comments
      if (line.slice(i).startsWith("//") || line.slice(i).startsWith("#")) {
        tokens.push({ text: line.slice(i), color: COLORS.textMuted });
        break;
      }
      // Strings
      if (line[i] === '"' || line[i] === "'") {
        const quote = line[i];
        let end = i + 1;
        while (end < line.length && line[end] !== quote) end++;
        tokens.push({
          text: line.slice(i, end + 1),
          color: COLORS.success,
        });
        i = end + 1;
        continue;
      }
      // Words
      if (/[a-zA-Z_]/.test(line[i])) {
        let end = i;
        while (end < line.length && /[a-zA-Z0-9_]/.test(line[end])) end++;
        const word = line.slice(i, end);
        let color: string | undefined;
        if (keywords.has(word)) color = COLORS.purple;
        else if (types.has(word)) color = COLORS.cyan;
        else if (line[end] === "(") color = COLORS.accentLight;
        tokens.push({ text: word, color });
        i = end;
        continue;
      }
      // Numbers
      if (/[0-9]/.test(line[i])) {
        let end = i;
        while (end < line.length && /[0-9.]/.test(line[end])) end++;
        tokens.push({ text: line.slice(i, end), color: COLORS.accent });
        i = end;
        continue;
      }
      // Other
      tokens.push({ text: line[i] });
      i++;
    }
    return tokens;
  });
};

export const CodeBlock: React.FC<{
  code: string;
  fontSize?: number;
  opacity?: number;
  style?: React.CSSProperties;
}> = ({ code, fontSize = 15, opacity = 1, style = {} }) => {
  const lines = highlight(code);
  return (
    <div
      style={{
        fontFamily: FONTS.mono,
        fontSize,
        lineHeight: 1.55,
        backgroundColor: "#0b1120",
        border: `1px solid ${COLORS.border}`,
        borderRadius: 10,
        padding: "16px 20px",
        color: COLORS.text,
        whiteSpace: "pre",
        opacity,
        ...style,
      }}
    >
      {lines.map((tokens, li) => (
        <div key={li}>
          {tokens.length === 0 ? (
            <span>&nbsp;</span>
          ) : (
            tokens.map((t, ti) => (
              <span key={ti} style={{ color: t.color || COLORS.text }}>
                {t.text}
              </span>
            ))
          )}
        </div>
      ))}
    </div>
  );
};
