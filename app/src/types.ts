export type Deployment = {
  id: string;
  name: string;
  origin: string;
  hosted: boolean;
};

export type Session = {
  tenant_id: string;
  user_id: string;
  capabilities?: { operator_webui_config?: boolean };
  features?: Record<string, boolean>;
};

export type ThreadRecord = {
  thread_id?: string;
  id?: string;
  title?: string | null;
  name?: string | null;
  created_at?: string;
  updated_at?: string;
  status?: string;
  [key: string]: unknown;
};

export type TimelineMessage = {
  message_id?: string;
  id?: string;
  role?: string;
  kind?: string;
  content?: string;
  text?: string;
  created_at?: string;
  [key: string]: unknown;
};

export type DraftAttachment = {
  id: string;
  name: string;
  mimeType: string;
  uri: string;
  size?: number;
};

export type TimelineResponse = {
  thread: ThreadRecord;
  messages: TimelineMessage[];
  summary_artifacts?: unknown[];
  next_cursor?: string;
};

export type Automation = {
  automation_id: string;
  name: string;
  source: string | { kind?: string; cron?: string; timezone?: string };
  state: string;
  is_active: boolean;
  next_run_at?: string;
  last_run_at?: string;
  last_status?: "ok" | "error";
};

export type ToolSetting = {
  key?: string;
  name?: string;
  value?: unknown;
  state?: string;
  [key: string]: unknown;
};
