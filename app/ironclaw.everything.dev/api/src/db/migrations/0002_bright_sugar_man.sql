CREATE TABLE "ironclaw_connections" (
	"id" text PRIMARY KEY NOT NULL,
	"name" text DEFAULT '' NOT NULL,
	"tunnel_url" text NOT NULL,
	"api_token" text NOT NULL,
	"created_by" text,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "ironclaw_scope_bindings" (
	"tenant_id" text NOT NULL,
	"agent_id" text,
	"project_id" text,
	"connection_id" text NOT NULL REFERENCES "ironclaw_connections"("id") ON DELETE cascade,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"created_by" text,
	CONSTRAINT "ironclaw_scope_bindings_tenant_id_agent_id_project_id_pk" PRIMARY KEY("tenant_id", "agent_id", "project_id")
);
