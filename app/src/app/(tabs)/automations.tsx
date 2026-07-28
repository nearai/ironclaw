import React from "react";
import { Alert, FlatList, RefreshControl, StyleSheet, Text, View } from "react-native";
import { useSession } from "@/auth/session-context";
import { Button, Card, Field, Screen, textStyles } from "@/components/ui";
import { DrawerButton } from "@/components/drawer-button";
import { cacheAutomations, cachedAutomations } from "@/storage/database";
import type { Automation } from "@/types";
import { colors } from "@/theme";

export default function AutomationsScreen() {
  const { api, deployment, session } = useSession();
  const scope = `${deployment.origin}|${session?.user_id ?? "cached"}`;
  const [rows, setRows] = React.useState<Automation[]>([]);
  const [refreshing, setRefreshing] = React.useState(false);
  const [scheduler, setScheduler] = React.useState(true);
  const [offline, setOffline] = React.useState(false);
  const [error, setError] = React.useState("");
  const [editingId, setEditingId] = React.useState("");
  const [name, setName] = React.useState("");

  const refresh = React.useCallback(async () => {
    setRefreshing(true);
    const local = await cachedAutomations(scope);
    if (local.length) setRows(local);
    try {
      const response = await api.listAutomations();
      setRows(response.automations);
      setScheduler(response.scheduler_enabled);
      await cacheAutomations(scope, response.automations);
      setOffline(false);
      setError("");
    } catch (reason) {
      setOffline(true);
      if (!local.length) setError(reason instanceof Error ? reason.message : "Could not load");
    } finally {
      setRefreshing(false);
    }
  }, [api, scope]);

  React.useEffect(() => void refresh(), [refresh]);

  async function toggle(row: Automation) {
    try {
      await api.mutateAutomation(row.automation_id, row.is_active ? "pause" : "resume");
      await refresh();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Could not update automation");
    }
  }

  async function rename(row: Automation) {
    const nextName = name.trim();
    if (!nextName) return;
    try {
      await api.renameAutomation(row.automation_id, nextName);
      setEditingId("");
      setName("");
      await refresh();
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Could not rename automation");
    }
  }

  function confirmDelete(row: Automation) {
    Alert.alert(
      "Delete automation?",
      `"${row.name}" and its schedule will be removed.`,
      [
        { text: "Cancel", style: "cancel" },
        {
          text: "Delete",
          style: "destructive",
          onPress: () => {
            void api
              .deleteAutomation(row.automation_id)
              .then(refresh)
              .catch((reason: unknown) =>
                setError(reason instanceof Error ? reason.message : "Could not delete automation")
              );
          }
        }
      ]
    );
  }

  return (
    <Screen style={styles.flush}>
      <DrawerButton />
      {offline ? <Text style={styles.banner}>Offline · saved automations</Text> : null}
      {!scheduler ? <Text style={styles.banner}>Scheduling is disabled on this deployment</Text> : null}
      {error ? <Text style={[textStyles.error, styles.pad]}>{error}</Text> : null}
      <FlatList
        data={rows}
        keyExtractor={(item) => item.automation_id}
        contentContainerStyle={styles.list}
        refreshControl={<RefreshControl refreshing={refreshing} onRefresh={() => void refresh()} />}
        ListEmptyComponent={
          !refreshing ? (
            <Card>
              <Text style={textStyles.heading}>No automations</Text>
              <Text style={textStyles.muted}>
                Ask your agent to schedule recurring or one-time work.
              </Text>
            </Card>
          ) : null
        }
        renderItem={({ item }) => (
          <Card>
            <View style={styles.row}>
              <View style={styles.grow}>
                <Text style={textStyles.heading}>{item.name}</Text>
                <Text style={textStyles.muted}>{item.state} · {String(item.source)}</Text>
              </View>
              <Text style={item.is_active ? styles.active : styles.paused}>
                {item.is_active ? "Active" : "Paused"}
              </Text>
            </View>
            {item.next_run_at ? (
              <Text style={textStyles.muted}>Next: {new Date(item.next_run_at).toLocaleString()}</Text>
            ) : null}
            <Button
              title={item.is_active ? "Pause" : "Resume"}
              tone="secondary"
              disabled={offline}
              onPress={() => void toggle(item)}
            />
            {editingId === item.automation_id ? (
              <View style={styles.edit}>
                <Field
                  autoFocus
                  value={name}
                  onChangeText={setName}
                  placeholder="Automation name"
                  onSubmitEditing={() => void rename(item)}
                />
                <View style={styles.buttonRow}>
                  <View style={styles.grow}>
                    <Button
                      title="Cancel"
                      tone="secondary"
                      onPress={() => setEditingId("")}
                    />
                  </View>
                  <View style={styles.grow}>
                    <Button
                      title="Save"
                      disabled={!name.trim()}
                      onPress={() => void rename(item)}
                    />
                  </View>
                </View>
              </View>
            ) : (
              <View style={styles.buttonRow}>
                <View style={styles.grow}>
                  <Button
                    title="Rename"
                    tone="secondary"
                    disabled={offline}
                    onPress={() => {
                      setEditingId(item.automation_id);
                      setName(item.name);
                    }}
                  />
                </View>
                <View style={styles.grow}>
                  <Button
                    title="Delete"
                    tone="danger"
                    disabled={offline}
                    onPress={() => confirmDelete(item)}
                  />
                </View>
              </View>
            )}
          </Card>
        )}
      />
    </Screen>
  );
}

const styles = StyleSheet.create({
  flush: { padding: 0 },
  list: { padding: 16, gap: 10 },
  pad: { paddingHorizontal: 16 },
  banner: { color: colors.background, backgroundColor: colors.warning, padding: 8, textAlign: "center", fontWeight: "700" },
  row: { flexDirection: "row", alignItems: "center", gap: 12 },
  grow: { flex: 1 },
  edit: { gap: 8 },
  buttonRow: { flexDirection: "row", gap: 8 },
  active: { color: colors.success, fontWeight: "700" },
  paused: { color: colors.muted, fontWeight: "700" }
});
