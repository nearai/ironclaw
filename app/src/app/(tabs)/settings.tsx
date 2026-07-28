import React from "react";
import { ScrollView, StyleSheet, Switch, Text, TextInput, View } from "react-native";
import * as Linking from "expo-linking";
import { useSession } from "@/auth/session-context";
import { Button, Card, Screen, textStyles } from "@/components/ui";
import { DrawerButton } from "@/components/drawer-button";
import type { ToolSetting } from "@/types";
import { colors } from "@/theme";

export default function SettingsScreen() {
  const { api, deployment, session, signOut } = useSession();
  const [tools, setTools] = React.useState<ToolSetting[]>([]);
  const [autoApprove, setAutoApprove] = React.useState(false);
  const [error, setError] = React.useState("");
  const [updatingTool, setUpdatingTool] = React.useState("");
  const [checking, setChecking] = React.useState(false);
  const [checkedAt, setCheckedAt] = React.useState("");
  const [toolQuery, setToolQuery] = React.useState("");

  const refresh = React.useCallback(async () => {
    try {
      const entries = await api.toolSettings();
      setTools(entries);
      const global = entries.find((entry) => entry.key === "agent.auto_approve_tools");
      setAutoApprove(Boolean(global?.value));
      setError("");
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Could not load settings");
    }
  }, [api]);

  React.useEffect(() => void refresh(), [refresh]);

  async function updateAutoApprove(enabled: boolean) {
    setAutoApprove(enabled);
    try {
      await api.setGlobalAutoApprove(enabled);
      await refresh();
    } catch (reason) {
      setAutoApprove(!enabled);
      setError(reason instanceof Error ? reason.message : "Could not update setting");
    }
  }

  async function updateTool(
    tool: ToolSetting,
    state: "ask" | "always_allow" | "always_deny"
  ) {
    const capabilityId = tool.key?.startsWith("tool.") ? tool.key.slice(5) : "";
    if (!capabilityId) return;
    setUpdatingTool(capabilityId);
    try {
      await api.setToolPermission(capabilityId, state);
      await refresh();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Could not update tool permission");
    } finally {
      setUpdatingTool("");
    }
  }

  async function checkConnection() {
    setChecking(true);
    try {
      await api.session();
      setCheckedAt(new Date().toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }));
      setError("");
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Connection check failed");
    } finally {
      setChecking(false);
    }
  }

  const capabilityTools = tools.filter((tool) => tool.key?.startsWith("tool."));
  const visibleTools = capabilityTools.filter((tool) => `${tool.name ?? ""} ${tool.key ?? ""}`.toLowerCase().includes(toolQuery.trim().toLowerCase()));

  return (
    <Screen style={styles.flush}>
      <DrawerButton />
      <ScrollView contentContainerStyle={styles.content}>
        <Card>
          <Text style={textStyles.heading}>Account</Text>
          <Text style={textStyles.body}>{session?.user_id ?? "Cached session"}</Text>
          <Text style={textStyles.muted}>Tenant · {session?.tenant_id ?? "Unknown"}</Text>
          <Text style={textStyles.muted}>{session?.capabilities?.operator_webui_config ? "Administrator access" : "Standard access"}</Text>
          <Button title="Sign out" tone="danger" onPress={() => void signOut()} />
        </Card>
        <Card>
          <Text style={textStyles.heading}>Agent connection</Text>
          <Text style={textStyles.body}>{deployment.name}</Text>
          <Text selectable style={textStyles.muted}>{deployment.origin}</Text>
          <Text style={textStyles.muted}>{deployment.hosted ? "Hosted deployment" : "Dedicated deployment"}</Text>
          <Button title={checking ? "Checking…" : "Test connection"} tone="secondary" onPress={() => void checkConnection()} disabled={checking} />
          {checkedAt ? <Text style={textStyles.muted}>Connected at {checkedAt}</Text> : null}
        </Card>
        <Card>
          <Text style={textStyles.heading}>Safety & approvals</Text>
          <View style={styles.row}>
            <View style={styles.grow}>
              <Text style={textStyles.heading}>Auto-approve tools</Text>
              <Text style={textStyles.muted}>
                Allow tools without asking each time. Use only on trusted deployments.
              </Text>
            </View>
            <Switch
              value={autoApprove}
              onValueChange={(value) => void updateAutoApprove(value)}
              trackColor={{ false: colors.border, true: colors.primaryPressed }}
            />
          </View>
        </Card>
        <Card>
          <Text style={textStyles.heading}>Tool permissions</Text>
          <Text style={textStyles.muted}>Choose whether each capability asks before it runs.</Text>
          <View style={styles.search}><Text style={styles.searchLabel}>Search</Text><View style={styles.grow}><TextInput value={toolQuery} onChangeText={setToolQuery} placeholder="Find a tool" placeholderTextColor={colors.muted} style={styles.searchInput} /></View></View>
          {visibleTools.length ? visibleTools.map((tool, index) => {
            const value = tool.value && typeof tool.value === "object"
              ? (tool.value as { state?: string }).state
              : tool.state;
            const capabilityId = tool.key?.slice(5) ?? "";
            return (
            <View key={tool.key ?? tool.name ?? index} style={styles.tool}>
              <Text style={textStyles.body}>{tool.name ?? tool.key ?? "Tool setting"}</Text>
              <Text style={textStyles.muted}>{value ?? "ask"}</Text>
              <View style={styles.permissions}>
                {([
                  ["Ask", "ask"],
                  ["Allow", "always_allow"],
                  ["Deny", "always_deny"]
                ] as const).map(([label, state]) => (
                  <View key={state} style={styles.grow}>
                    <Button
                      title={updatingTool === capabilityId ? "Saving…" : label}
                      tone={value === state ? "primary" : state === "always_deny" ? "danger" : "secondary"}
                      disabled={Boolean(updatingTool)}
                      onPress={() => void updateTool(tool, state)}
                    />
                  </View>
                ))}
              </View>
            </View>
            );
          }) : <Text style={textStyles.muted}>{capabilityTools.length ? "No matching tools." : "No tool permissions reported."}</Text>}
        </Card>
        <Card>
          <Text style={textStyles.heading}>App & offline data</Text>
          <Text style={textStyles.muted}>Recent threads, automations, and drafts are encrypted and kept on this device so they remain readable offline.</Text>
          <Text style={textStyles.muted}>The app never stores your bearer token in the conversation database.</Text>
        </Card>
        <Card>
          <Text style={textStyles.heading}>Advanced configuration</Text>
          <Text style={textStyles.muted}>Model providers, channels, skills, networking, and deployment-wide agent limits are managed in the IronClaw web console.</Text>
          <Button title="Open web console" tone="secondary" onPress={() => void Linking.openURL(deployment.origin)} />
        </Card>
        {error ? <Text style={textStyles.error}>{error}</Text> : null}
        <Button title="Refresh settings" tone="secondary" onPress={() => void refresh()} />
      </ScrollView>
    </Screen>
  );
}

const styles = StyleSheet.create({
  flush: { padding: 0 },
  content: { padding: 16, gap: 12 },
  row: { flexDirection: "row", alignItems: "center", gap: 12 },
  grow: { flex: 1 },
  search: { flexDirection: "row", alignItems: "center", gap: 8, borderBottomColor: colors.border, borderBottomWidth: 1, paddingVertical: 8 },
  searchLabel: { color: colors.muted, fontSize: 12, textTransform: "uppercase" },
  searchInput: { color: colors.text, fontSize: 15, paddingVertical: 4 },
  tool: { borderTopColor: colors.border, borderTopWidth: 1, paddingTop: 10, gap: 8 },
  permissions: { flexDirection: "row", gap: 6 }
});
