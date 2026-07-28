import React from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import * as Clipboard from "expo-clipboard";
import { marked } from "marked";
import { colors } from "@/theme";

type MarkdownToken = { type: string; [key: string]: unknown };

function tokensFor(value: string): MarkdownToken[] {
  return marked.lexer(value) as unknown as MarkdownToken[];
}

function InlineText({ tokens, fallback }: { tokens?: MarkdownToken[]; fallback?: string }) {
  const first = fallback ? marked.lexer(fallback)[0] as unknown as MarkdownToken | undefined : undefined;
  const inline = tokens ?? (first?.tokens as MarkdownToken[] | undefined) ?? [];
  return (
    <Text style={styles.paragraph}>
      {inline.map((token, index) => {
        const text = String(token.text ?? token.raw ?? "");
        if (token.type === "strong") return <Text key={index} style={styles.bold}><InlineText tokens={token.tokens as MarkdownToken[]} fallback={text} /></Text>;
        if (token.type === "em") return <Text key={index} style={styles.italic}><InlineText tokens={token.tokens as MarkdownToken[]} fallback={text} /></Text>;
        if (token.type === "codespan") return <Text key={index} style={styles.inlineCode}>{text}</Text>;
        if (token.type === "del") return <Text key={index} style={styles.strikethrough}><InlineText tokens={token.tokens as MarkdownToken[]} fallback={text} /></Text>;
        if (token.type === "link") return <Text key={index} style={styles.link}>{String(token.text ?? text)}</Text>;
        if (token.type === "br") return <Text key={index}>{"\n"}</Text>;
        return <Text key={index}>{text}</Text>;
      })}
    </Text>
  );
}

function CodeBlock({ code, language }: { code: string; language?: string }) {
  const [expanded, setExpanded] = React.useState(false);
  const long = code.split("\n").length > 16 || code.length > 1200;
  const shown = long && !expanded ? `${code.split("\n").slice(0, 16).join("\n")}\n…` : code;
  async function copy() {
    await Clipboard.setStringAsync(code);
  }
  return (
    <View style={styles.codeFrame}>
      <View style={styles.codeToolbar}>
        <Text style={styles.language}>{language || "code"}</Text>
        <Pressable accessibilityRole="button" onPress={() => void copy()}><Text style={styles.codeAction}>Copy</Text></Pressable>
      </View>
      <ScrollView horizontal showsHorizontalScrollIndicator={false} style={styles.codeScroll}>
        <Text selectable style={styles.code}>{shown}</Text>
      </ScrollView>
      {long ? <Pressable accessibilityRole="button" onPress={() => setExpanded((value) => !value)} style={styles.expandCode}><Text style={styles.codeAction}>{expanded ? "Show less" : "Show more"}</Text></Pressable> : null}
    </View>
  );
}

function Block({ token, index }: { token: MarkdownToken; index: number }): React.ReactNode {
  switch (token.type) {
    case "heading": {
      const depth = Number(token.depth ?? 2);
      return <Text key={index} style={depth === 1 ? styles.h1 : depth === 2 ? styles.h2 : styles.h3}><InlineText tokens={token.tokens as MarkdownToken[]} fallback={String(token.text ?? "")} /></Text>;
    }
    case "paragraph":
    case "text":
      return <InlineText key={index} tokens={token.tokens as MarkdownToken[]} fallback={String(token.text ?? "")} />;
    case "code":
      return <CodeBlock key={index} code={String(token.text ?? "")} language={String(token.lang ?? "")} />;
    case "blockquote":
      return <View key={index} style={styles.quote}><InlineText fallback={String(token.text ?? "")} tokens={token.tokens as MarkdownToken[]} /></View>;
    case "list": {
      const items = (token.items as MarkdownToken[] | undefined) ?? [];
      return <View key={index} style={styles.list}>{items.map((item, itemIndex) => <View key={itemIndex} style={styles.listItem}><Text style={styles.listMark}>{token.ordered ? `${Number(token.start ?? 1) + itemIndex}.` : "•"}</Text><InlineText tokens={item.tokens as MarkdownToken[]} fallback={String(item.text ?? "")} /></View>)}</View>;
    }
    case "table": {
      const header = (token.header as MarkdownToken[] | undefined) ?? [];
      const rows = (token.rows as MarkdownToken[][] | undefined) ?? [];
      return <ScrollView key={index} horizontal><View style={styles.table}>{[header, ...rows].map((row, rowIndex) => <View key={rowIndex} style={styles.tableRow}>{row.map((cell, cellIndex) => <View key={cellIndex} style={[styles.tableCell, rowIndex === 0 && styles.tableHeader]}><InlineText tokens={cell.tokens as MarkdownToken[]} fallback={String(cell.text ?? "")} /></View>)}</View>)}</View></ScrollView>;
    }
    case "hr": return <View key={index} style={styles.rule} />;
    case "space": return null;
    default: return <InlineText key={index} fallback={String(token.text ?? token.raw ?? "")} tokens={token.tokens as MarkdownToken[]} />;
  }
}

export function Markdown({ content }: { content: string }) {
  return <View style={styles.markdown}>{tokensFor(content).map((token, index) => Block({ token, index }))}</View>;
}

export function CollapsibleAction({ name, status, detail, parameters, result, error, onRetry }: { name: string; status: string; detail?: string; parameters?: string; result?: string; error?: string; onRetry?: () => void }) {
  const [expanded, setExpanded] = React.useState(status === "error" || status === "declined");
  const failed = status === "error" || status === "declined" || status === "failed";
  const label = status === "running" ? "Working" : status === "success" ? "Done" : status === "declined" ? "Declined" : status === "error" ? "Failed" : status;
  return <View style={styles.action}>
    <Pressable accessibilityRole="button" onPress={() => setExpanded((value) => !value)} style={styles.actionHeader}>
      <View style={[styles.dot, { backgroundColor: failed ? colors.danger : status === "success" ? colors.success : colors.primary }]} />
      <Text style={styles.actionStatus}>{label}</Text><Text numberOfLines={1} style={styles.actionName}>{name}</Text><Text style={styles.chevron}>{expanded ? "⌃" : "⌄"}</Text>
    </Pressable>
    {expanded ? <View style={styles.actionBody}>{error ? <Text style={styles.error}>{error}</Text> : null}{detail ? <Text style={styles.detail}>{detail}</Text> : null}{parameters ? <Text selectable style={styles.payload}>{parameters}</Text> : null}{result ? <Text selectable style={styles.payload}>{result}</Text> : null}{!error && !detail && !parameters && !result ? <Text style={styles.detail}>No additional details</Text> : null}{onRetry ? <Pressable accessibilityRole="button" onPress={onRetry} style={styles.retry}><Text style={styles.retryText}>Retry</Text></Pressable> : null}</View> : null}
  </View>;
}

const styles = StyleSheet.create({
  markdown: { gap: 8, width: "100%" }, paragraph: { color: colors.body, fontSize: 16, lineHeight: 25 }, bold: { color: colors.text, fontWeight: "700" }, italic: { fontStyle: "italic" }, strikethrough: { textDecorationLine: "line-through" }, inlineCode: { color: colors.primaryText, backgroundColor: colors.surfaceRaised, fontFamily: "Menlo" }, link: { color: colors.primaryText, textDecorationLine: "underline" }, h1: { color: colors.text, fontSize: 24, lineHeight: 31, fontWeight: "700" }, h2: { color: colors.text, fontSize: 19, lineHeight: 26, fontWeight: "700" }, h3: { color: colors.text, fontSize: 17, lineHeight: 24, fontWeight: "700" }, list: { gap: 6 }, listItem: { flexDirection: "row", gap: 8, alignItems: "flex-start" }, listMark: { color: colors.primaryText, minWidth: 20, fontSize: 16, lineHeight: 25 }, quote: { borderLeftColor: colors.primary, borderLeftWidth: 3, paddingLeft: 12 }, rule: { borderBottomColor: colors.border, borderBottomWidth: 1 }, codeFrame: { backgroundColor: colors.backgroundStrong, borderColor: colors.border, borderWidth: 1, borderRadius: 10, overflow: "hidden" }, codeToolbar: { flexDirection: "row", justifyContent: "space-between", alignItems: "center", paddingHorizontal: 12, paddingVertical: 7, borderBottomColor: colors.border, borderBottomWidth: 1 }, language: { color: colors.faint, fontSize: 11, textTransform: "uppercase", fontFamily: "Menlo" }, codeAction: { color: colors.primaryText, fontSize: 12, fontWeight: "700" }, codeScroll: { padding: 12 }, code: { color: colors.body, fontSize: 13, lineHeight: 19, fontFamily: "Menlo" }, expandCode: { alignItems: "center", borderTopColor: colors.border, borderTopWidth: 1, paddingVertical: 7 }, table: { borderColor: colors.border, borderWidth: 1, borderRadius: 8, overflow: "hidden" }, tableRow: { flexDirection: "row" }, tableCell: { width: 160, flexShrink: 1, borderRightColor: colors.border, borderRightWidth: 1, borderBottomColor: colors.border, borderBottomWidth: 1, padding: 8 }, tableHeader: { backgroundColor: colors.surfaceRaised }, action: { borderBottomColor: colors.border, borderBottomWidth: 1, width: "100%" }, actionHeader: { minHeight: 42, flexDirection: "row", alignItems: "center", gap: 9, paddingHorizontal: 4 }, dot: { width: 8, height: 8, borderRadius: 4 }, actionStatus: { color: colors.muted, fontFamily: "Menlo", fontSize: 11, textTransform: "uppercase" }, actionName: { color: colors.body, flex: 1, fontFamily: "Menlo", fontSize: 13 }, chevron: { color: colors.muted, fontSize: 17 }, actionBody: { backgroundColor: colors.backgroundStrong, borderColor: colors.border, borderLeftWidth: 1, borderRightWidth: 1, borderTopWidth: 1, borderTopLeftRadius: 8, borderTopRightRadius: 8, padding: 12, gap: 8 }, detail: { color: colors.body, fontSize: 13, lineHeight: 19 }, payload: { color: colors.body, fontFamily: "Menlo", fontSize: 12, lineHeight: 18 }, error: { color: colors.danger, fontSize: 13, lineHeight: 19 }, retry: { alignSelf: "flex-start", marginTop: 2, paddingHorizontal: 10, paddingVertical: 6, borderRadius: 8, backgroundColor: colors.surfaceRaised }, retryText: { color: colors.primaryText, fontSize: 13, fontWeight: "700" }
});
