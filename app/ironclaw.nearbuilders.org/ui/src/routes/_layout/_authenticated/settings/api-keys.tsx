import { createFileRoute } from "@tanstack/react-router";
import { UserApiKeysPanel } from "@/components/settings-sections";

export const Route = createFileRoute("/_layout/_authenticated/settings/api-keys")({
  component: ApiKeysSettings,
});

function ApiKeysSettings() {
  return <UserApiKeysPanel />;
}
