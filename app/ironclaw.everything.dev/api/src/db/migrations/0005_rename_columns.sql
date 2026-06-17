ALTER TABLE "ironclaw_connections" RENAME COLUMN "tunnel_url" TO "base_url";
ALTER TABLE "ironclaw_connections" RENAME COLUMN "api_token" TO "api_token_encrypted";
ALTER TABLE "tenant_credentials" RENAME COLUMN "tunnel_url" TO "base_url";
ALTER TABLE "tenant_credentials" RENAME COLUMN "api_token" TO "api_token_encrypted";
