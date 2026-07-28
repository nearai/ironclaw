import React from "react";
import { DrawerContentScrollView, type DrawerContentComponentProps } from "expo-router/drawer";
import { Alert, Pressable, StyleSheet, Text, View } from "react-native";
import { router, usePathname } from "expo-router";
import { Ionicons } from "@expo/vector-icons";
import { useSession } from "@/auth/session-context";
import { Button, Field, textStyles } from "@/components/ui";
import { cacheThreads, cachedThreads } from "@/storage/database";
import { clientActionId, threadId } from "@/lib/ids";
import type { ThreadRecord } from "@/types";
import { colors } from "@/theme";

function title(thread: ThreadRecord): string {
  return thread.title?.trim() || thread.name?.trim() || "New chat";
}

export function AppDrawer(props: DrawerContentComponentProps) {
  const { api, deployment, session } = useSession();
  const scope = `${deployment.origin}|${session?.user_id ?? "cached"}`;
  const pathname = usePathname();
  const [threads, setThreads] = React.useState<ThreadRecord[]>([]);
  const [query, setQuery] = React.useState("");
  const [loading, setLoading] = React.useState(false);

  const refresh = React.useCallback(async () => {
    const local = await cachedThreads(scope);
    if (local.length) setThreads(local);
    try {
      const response = await api.listThreads();
      setThreads(response.threads);
      await cacheThreads(scope, response.threads);
    } catch {
      // Cached threads remain useful while offline.
    }
  }, [api, scope]);

  React.useEffect(() => { void refresh(); }, [pathname, refresh]);

  function close() { props.navigation.closeDrawer(); }
  function go(path: "/(tabs)/automations" | "/(tabs)/settings") {
    close();
    router.push(path);
  }
  async function createThread() {
    setLoading(true);
    try {
      const response = await api.createThread(clientActionId());
      const id = threadId(response.thread);
      await refresh();
      close();
      if (id) router.push({ pathname: "/thread/[id]", params: { id } });
    } catch (reason) {
      Alert.alert("Could not create thread", reason instanceof Error ? reason.message : "Try again.");
    } finally {
      setLoading(false);
    }
  }

  const visible = threads.filter((item) => `${title(item)} ${threadId(item)}`.toLowerCase().includes(query.trim().toLowerCase()));
  return (
    <DrawerContentScrollView {...props} contentContainerStyle={styles.content}>
      <View style={styles.brand}><View style={styles.logo}><Ionicons name="sparkles" size={18} color={colors.primaryText} /></View><View><Text style={styles.brandName}>IronClaw</Text><Text style={styles.deployment}>{deployment.name}</Text></View></View>
      <Button title="＋  New thread" disabled={loading} onPress={() => void createThread()} />
      <Field value={query} onChangeText={setQuery} placeholder="Search threads" returnKeyType="search" />
      <View style={styles.sectionHeader}><Text style={styles.sectionTitle}>Threads</Text><Pressable onPress={() => void refresh()}><Ionicons name="refresh" size={16} color={colors.muted} /></Pressable></View>
      <View style={styles.threadList}>
        {visible.slice(0, 30).map((item) => {
          const id = threadId(item);
          return <Pressable key={id} onPress={() => { close(); router.push({ pathname: "/thread/[id]", params: { id } }); }} style={styles.thread}><Ionicons name="chatbubble-outline" size={15} color={colors.muted} /><Text numberOfLines={2} style={styles.threadText}>{title(item)}</Text></Pressable>;
        })}
        {!visible.length ? <Text style={textStyles.muted}>No saved threads</Text> : null}
      </View>
      <View style={styles.divider} />
      <Pressable onPress={() => go("/(tabs)/automations")} style={styles.menu}><Ionicons name="timer-outline" size={19} color={colors.muted} /><Text style={styles.menuText}>Automations</Text></Pressable>
      <Pressable onPress={() => go("/(tabs)/settings")} style={styles.menu}><Ionicons name="settings-outline" size={19} color={colors.muted} /><Text style={styles.menuText}>Settings</Text></Pressable>
    </DrawerContentScrollView>
  );
}

const styles = StyleSheet.create({
  content: { flexGrow: 1, backgroundColor: colors.surface, padding: 14, gap: 12 },
  brand: { flexDirection: "row", alignItems: "center", gap: 10, paddingVertical: 8 },
  logo: { width: 32, height: 32, borderRadius: 10, backgroundColor: colors.primarySoft, alignItems: "center", justifyContent: "center" },
  brandName: { color: colors.text, fontSize: 18, fontWeight: "700" },
  deployment: { color: colors.muted, fontSize: 12 },
  sectionHeader: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", paddingTop: 4 },
  sectionTitle: { color: colors.muted, fontSize: 12, fontWeight: "700", textTransform: "uppercase", letterSpacing: 0.6 },
  threadList: { gap: 3 },
  thread: { flexDirection: "row", alignItems: "center", gap: 9, paddingVertical: 9, paddingHorizontal: 7, borderRadius: 8 },
  threadText: { color: colors.body, flex: 1, fontSize: 14 },
  divider: { height: 1, backgroundColor: colors.border, marginVertical: 4 },
  menu: { flexDirection: "row", alignItems: "center", gap: 11, paddingVertical: 10, paddingHorizontal: 7 },
  menuText: { color: colors.body, fontSize: 15 }
});
