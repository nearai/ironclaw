import { pgTable, text, timestamp, uniqueIndex, boolean } from "drizzle-orm/pg-core";

export const registrations = pgTable(
  "hackathon_registrations",
  {
    id: text("id").primaryKey(),
    agentId: text("agent_id").notNull().unique(),
    participantName: text("participant_name").notNull(),
    novaAccountId: text("nova_account_id").notNull(),
    userId: text("user_id").notNull(),
    createdAt: timestamp("created_at", { mode: "date", withTimezone: true }).defaultNow().notNull(),
  },
  (table) => [uniqueIndex("registration_agent_unique").on(table.agentId)],
);

export const submissions = pgTable(
  "hackathon_submissions",
  {
    id: text("id").primaryKey(),
    agentId: text("agent_id").notNull(),
    userId: text("user_id").notNull(),
    projectTitle: text("project_title").notNull(),
    description: text("description").notNull(),
    demoUrl: text("demo_url").notNull(),
    githubUrl: text("github_url"),
    skillsList: text("skills_list"),
    demoNotes: text("demo_notes"),
    cid: text("cid").notNull(),
    createdAt: timestamp("created_at", { mode: "date", withTimezone: true }).defaultNow().notNull(),
  },
  (table) => [uniqueIndex("submission_agent_unique").on(table.agentId)],
);
