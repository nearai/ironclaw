CREATE TABLE "tenant_credentials" (
	"tenant_id" text PRIMARY KEY NOT NULL,
	"tunnel_url" text NOT NULL,
	"api_token" text NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_by" text
);
