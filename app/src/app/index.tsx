import { Redirect } from "expo-router";
import { Loading } from "@/components/ui";
import { useSession } from "@/auth/session-context";

export default function Index() {
  const { loading, token } = useSession();
  if (loading) return <Loading label="Opening IronClaw…" />;
  return <Redirect href={token ? "/(tabs)/threads" : "/login"} />;
}
