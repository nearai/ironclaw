CREATE TABLE "hackathon_registrations" (
	"id" text PRIMARY KEY NOT NULL,
	"agent_id" text NOT NULL,
	"participant_name" text NOT NULL,
	"nova_account_id" text NOT NULL,
	"user_id" text NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE UNIQUE INDEX "registration_agent_unique" ON "hackathon_registrations" USING btree ("agent_id");
--> statement-breakpoint
CREATE TABLE "hackathon_submissions" (
	"id" text PRIMARY KEY NOT NULL,
	"agent_id" text NOT NULL,
	"user_id" text NOT NULL,
	"project_title" text NOT NULL,
	"description" text NOT NULL,
	"demo_url" text NOT NULL,
	"github_url" text,
	"skills_list" text,
	"demo_notes" text,
	"cid" text NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE UNIQUE INDEX "submission_agent_unique" ON "hackathon_submissions" USING btree ("agent_id");
