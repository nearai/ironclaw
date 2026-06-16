--> statement-breakpoint
DROP TABLE IF EXISTS "ironclaw_scope_bindings" CASCADE;
--> statement-breakpoint
CREATE TABLE "ironclaw_scope_bindings" (
	"id" text PRIMARY KEY NOT NULL,
	"tenant_id" text NOT NULL,
	"scope_type" text DEFAULT 'personal' NOT NULL,
	"connection_id" text NOT NULL REFERENCES "ironclaw_connections"("id") ON DELETE cascade,
	"created_by" text,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE UNIQUE INDEX "scope_bindings_tenant_type_unique" ON "ironclaw_scope_bindings" USING btree ("tenant_id", "scope_type");
--> statement-breakpoint
CREATE TABLE "user_preferences" (
	"user_id" text PRIMARY KEY NOT NULL,
	"ironclaw_mode" text DEFAULT 'auto' NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
INSERT INTO "ironclaw_connections" ("id", "name", "tunnel_url", "api_token", "updated_by", "updated_at")
SELECT 'conn_' || tc."tenant_id", 'Migrated from tenant_credentials', tc."tunnel_url", tc."api_token", tc."updated_by", tc."updated_at"
FROM "tenant_credentials" tc
WHERE NOT EXISTS (SELECT 1 FROM "ironclaw_connections" c WHERE c."id" = 'conn_' || tc."tenant_id");
--> statement-breakpoint
INSERT INTO "ironclaw_scope_bindings" ("id", "tenant_id", "scope_type", "connection_id", "created_by")
SELECT 'sb_' || tc."tenant_id" || '_personal', tc."tenant_id", 'personal', 'conn_' || tc."tenant_id", tc."updated_by"
FROM "tenant_credentials" tc
WHERE NOT EXISTS (SELECT 1 FROM "ironclaw_scope_bindings" sb WHERE sb."tenant_id" = tc."tenant_id");
