import { Platform } from "react-native";
import * as SecureStore from "expo-secure-store";
import * as SQLite from "expo-sqlite";
import * as Crypto from "expo-crypto";
import type { Automation, ThreadRecord, TimelineMessage } from "@/types";

const DATABASE_KEY = "ironclaw.mobile.database-key.v1";
const DATABASE_NAME = "ironclaw-mobile.db";

let databasePromise: Promise<SQLite.SQLiteDatabase> | undefined;

async function encryptionKey(): Promise<string> {
  const existing = await SecureStore.getItemAsync(DATABASE_KEY);
  if (existing) return existing;
  const next = `${Crypto.randomUUID()}${Crypto.randomUUID()}`.replaceAll("-", "");
  await SecureStore.setItemAsync(DATABASE_KEY, next, {
    keychainAccessible: SecureStore.WHEN_UNLOCKED_THIS_DEVICE_ONLY
  });
  return next;
}

async function openNativeDatabase(): Promise<SQLite.SQLiteDatabase> {
  const db = await SQLite.openDatabaseAsync(DATABASE_NAME);
  const key = await encryptionKey();
  await db.execAsync(`PRAGMA key = '${key}';`);
  await db.execAsync(`
    PRAGMA journal_mode = WAL;
    PRAGMA foreign_keys = ON;
    CREATE TABLE IF NOT EXISTS schema_meta (
      version INTEGER NOT NULL
    );
    INSERT INTO schema_meta(version)
      SELECT 1 WHERE NOT EXISTS (SELECT 1 FROM schema_meta);
    CREATE TABLE IF NOT EXISTS threads (
      scope TEXT NOT NULL,
      thread_id TEXT NOT NULL,
      payload TEXT NOT NULL,
      updated_at INTEGER NOT NULL,
      PRIMARY KEY(scope, thread_id)
    );
    CREATE TABLE IF NOT EXISTS timeline (
      scope TEXT NOT NULL,
      thread_id TEXT NOT NULL,
      message_id TEXT NOT NULL,
      position INTEGER NOT NULL,
      payload TEXT NOT NULL,
      updated_at INTEGER NOT NULL,
      PRIMARY KEY(scope, thread_id, message_id)
    );
    CREATE INDEX IF NOT EXISTS timeline_order
      ON timeline(scope, thread_id, position);
    CREATE TABLE IF NOT EXISTS automations (
      scope TEXT NOT NULL,
      automation_id TEXT NOT NULL,
      payload TEXT NOT NULL,
      updated_at INTEGER NOT NULL,
      PRIMARY KEY(scope, automation_id)
    );
    CREATE TABLE IF NOT EXISTS drafts (
      scope TEXT NOT NULL,
      thread_id TEXT NOT NULL,
      content TEXT NOT NULL,
      updated_at INTEGER NOT NULL,
      PRIMARY KEY(scope, thread_id)
    );
    CREATE TABLE IF NOT EXISTS sync_meta (
      scope TEXT NOT NULL,
      resource TEXT NOT NULL,
      synced_at INTEGER NOT NULL,
      PRIMARY KEY(scope, resource)
    );
  `);
  return db;
}

export async function database(): Promise<SQLite.SQLiteDatabase | null> {
  if (Platform.OS === "web") return null;
  databasePromise ??= openNativeDatabase();
  return databasePromise;
}

function recordId(record: ThreadRecord): string {
  return record.thread_id ?? record.id ?? "";
}

function timelineId(message: TimelineMessage, index: number): string {
  return message.message_id ?? message.id ?? `position-${index}`;
}

export async function cacheThreads(scope: string, threads: ThreadRecord[]): Promise<void> {
  const db = await database();
  if (!db) return;
  await db.withTransactionAsync(async () => {
    await db.runAsync("DELETE FROM threads WHERE scope = ?", scope);
    for (const thread of threads) {
      const id = recordId(thread);
      if (!id) continue;
      await db.runAsync(
        `INSERT INTO threads(scope, thread_id, payload, updated_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(scope, thread_id)
         DO UPDATE SET payload = excluded.payload, updated_at = excluded.updated_at`,
        scope,
        id,
        JSON.stringify(thread),
        Date.now()
      );
    }
    await db.runAsync(
      `INSERT INTO sync_meta(scope, resource, synced_at) VALUES (?, 'threads', ?)
       ON CONFLICT(scope, resource) DO UPDATE SET synced_at = excluded.synced_at`,
      scope,
      Date.now()
    );
  });
}

export async function cachedThreads(scope: string): Promise<ThreadRecord[]> {
  const db = await database();
  if (!db) return [];
  const rows = await db.getAllAsync<{ payload: string }>(
    "SELECT payload FROM threads WHERE scope = ? ORDER BY updated_at DESC",
    scope
  );
  return rows.flatMap((row) => {
    try {
      return [JSON.parse(row.payload) as ThreadRecord];
    } catch {
      return [];
    }
  });
}

export async function cacheTimeline(
  scope: string,
  threadId: string,
  messages: TimelineMessage[]
): Promise<void> {
  const db = await database();
  if (!db) return;
  await db.withTransactionAsync(async () => {
    await db.runAsync(
      "DELETE FROM timeline WHERE scope = ? AND thread_id = ?",
      scope,
      threadId
    );
    for (const [index, message] of messages.entries()) {
      await db.runAsync(
        `INSERT INTO timeline(scope, thread_id, message_id, position, payload, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(scope, thread_id, message_id)
         DO UPDATE SET position = excluded.position, payload = excluded.payload,
           updated_at = excluded.updated_at`,
        scope,
        threadId,
        timelineId(message, index),
        index,
        JSON.stringify(message),
        Date.now()
      );
    }
    await db.runAsync(
      `INSERT INTO sync_meta(scope, resource, synced_at) VALUES (?, ?, ?)
       ON CONFLICT(scope, resource) DO UPDATE SET synced_at = excluded.synced_at`,
      scope,
      `timeline:${threadId}`,
      Date.now()
    );
  });
}

export async function cachedTimeline(
  scope: string,
  threadId: string
): Promise<TimelineMessage[]> {
  const db = await database();
  if (!db) return [];
  const rows = await db.getAllAsync<{ payload: string }>(
    `SELECT payload FROM timeline WHERE scope = ? AND thread_id = ?
     ORDER BY position ASC`,
    scope,
    threadId
  );
  return rows.flatMap((row) => {
    try {
      return [JSON.parse(row.payload) as TimelineMessage];
    } catch {
      return [];
    }
  });
}

export async function cacheAutomations(scope: string, rows: Automation[]): Promise<void> {
  const db = await database();
  if (!db) return;
  await db.withTransactionAsync(async () => {
    await db.runAsync("DELETE FROM automations WHERE scope = ?", scope);
    for (const automation of rows) {
      await db.runAsync(
        `INSERT INTO automations(scope, automation_id, payload, updated_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(scope, automation_id)
         DO UPDATE SET payload = excluded.payload, updated_at = excluded.updated_at`,
        scope,
        automation.automation_id,
        JSON.stringify(automation),
        Date.now()
      );
    }
    await db.runAsync(
      `INSERT INTO sync_meta(scope, resource, synced_at) VALUES (?, 'automations', ?)
       ON CONFLICT(scope, resource) DO UPDATE SET synced_at = excluded.synced_at`,
      scope,
      Date.now()
    );
  });
}

export async function cachedAutomations(scope: string): Promise<Automation[]> {
  const db = await database();
  if (!db) return [];
  const rows = await db.getAllAsync<{ payload: string }>(
    "SELECT payload FROM automations WHERE scope = ? ORDER BY updated_at DESC",
    scope
  );
  return rows.flatMap((row) => {
    try {
      return [JSON.parse(row.payload) as Automation];
    } catch {
      return [];
    }
  });
}

export async function saveDraft(scope: string, threadId: string, content: string): Promise<void> {
  const db = await database();
  if (!db) return;
  await db.runAsync(
    `INSERT INTO drafts(scope, thread_id, content, updated_at) VALUES (?, ?, ?, ?)
     ON CONFLICT(scope, thread_id)
     DO UPDATE SET content = excluded.content, updated_at = excluded.updated_at`,
    scope,
    threadId,
    content,
    Date.now()
  );
}

export async function loadDraft(scope: string, threadId: string): Promise<string> {
  const db = await database();
  if (!db) return "";
  const row = await db.getFirstAsync<{ content: string }>(
    "SELECT content FROM drafts WHERE scope = ? AND thread_id = ?",
    scope,
    threadId
  );
  return row?.content ?? "";
}
