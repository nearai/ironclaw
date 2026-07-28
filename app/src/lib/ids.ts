import * as Crypto from "expo-crypto";

export function clientActionId(): string {
  return Crypto.randomUUID();
}

export function threadId(record: { thread_id?: string; id?: string }): string {
  return record.thread_id ?? record.id ?? "";
}

export function messageText(record: {
  content?: string;
  text?: string;
  [key: string]: unknown;
}): string {
  if (typeof record.content === "string") return record.content;
  if (typeof record.text === "string") return record.text;
  return "";
}
