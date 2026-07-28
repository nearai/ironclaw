import Constants from "expo-constants";
import React from "react";
import { KeyboardAvoidingView, Platform, ScrollView, StyleSheet, Text, View } from "react-native";
import { Redirect } from "expo-router";
import { useSession } from "@/auth/session-context";
import { Button, Card, Field, Screen, textStyles } from "@/components/ui";
import { colors } from "@/theme";

const hostedOrigin =
  (Constants.expoConfig?.extra?.hostedOrigin as string | undefined) ??
  "https://agent-stg.near.ai";
const production = Constants.expoConfig?.extra?.buildProfile === "production";

function allowedOrigin(value: string): boolean {
  if (value.startsWith("https://")) return true;
  if (production) return false;
  return /^http:\/\/(localhost|127\.0\.0\.1|\[::1\])(?::\d+)?\/?$/i.test(value.trim());
}

export default function LoginScreen() {
  const { token: activeToken, connectWithToken, loginWithProvider, error } = useSession();
  const [advanced, setAdvanced] = React.useState(false);
  const [origin, setOrigin] = React.useState(hostedOrigin);
  const [token, setToken] = React.useState("");
  const [busy, setBusy] = React.useState("");
  const [localError, setLocalError] = React.useState("");

  if (activeToken) return <Redirect href="/(tabs)/threads" />;

  async function providerLogin(provider: string) {
    setBusy(provider);
    setLocalError("");
    try {
      await loginWithProvider(provider);
    } catch (reason) {
      setLocalError(reason instanceof Error ? reason.message : "Sign-in failed");
    } finally {
      setBusy("");
    }
  }

  async function connect() {
    setBusy("token");
    setLocalError("");
    try {
      await connectWithToken(origin, token.trim());
    } catch (reason) {
      setLocalError(reason instanceof Error ? reason.message : "Could not connect");
    } finally {
      setBusy("");
    }
  }

  return (
    <KeyboardAvoidingView
      style={styles.root}
      behavior={Platform.OS === "ios" ? "padding" : undefined}
    >
      <ScrollView contentContainerStyle={styles.content} keyboardShouldPersistTaps="handled">
        <View style={styles.hero}>
          <Text style={styles.mark}>IC</Text>
          <Text style={textStyles.title}>Your agent, wherever you are.</Text>
          <Text style={textStyles.muted}>
            Ask questions, do knowledge work, and keep coding tasks moving.
          </Text>
        </View>
        <Card>
          {["google", "apple", "github", "near"].map((provider) => (
            <Button
              key={provider}
              title={`${busy === provider ? "Opening…" : "Continue"} with ${
                provider === "near" ? "NEAR" : provider[0]?.toUpperCase() + provider.slice(1)
              }`}
              disabled={Boolean(busy)}
              tone="secondary"
              onPress={() => void providerLogin(provider)}
            />
          ))}
          <Text style={textStyles.muted}>Connects to {hostedOrigin}</Text>
        </Card>
        <Button
          title={advanced ? "Hide dedicated deployment" : "Pair a dedicated deployment"}
          tone="secondary"
          onPress={() => setAdvanced((value) => !value)}
        />
        {advanced ? (
          <Card>
            <Text style={textStyles.heading}>Dedicated deployment</Text>
            <Text style={textStyles.muted}>
              Until one-time mobile pairing is enabled on the deployment, enter its scoped bearer.
            </Text>
            <Field
              autoCapitalize="none"
              autoCorrect={false}
              keyboardType="url"
              onChangeText={setOrigin}
              placeholder="https://agent.example.com"
              value={origin}
            />
            <Field
              autoCapitalize="none"
              autoCorrect={false}
              onChangeText={setToken}
              placeholder="Pairing or scoped bearer token"
              secureTextEntry
              value={token}
            />
            <Button
              title={busy === "token" ? "Connecting…" : "Connect securely"}
              disabled={!allowedOrigin(origin) || !token.trim() || Boolean(busy)}
              onPress={() => void connect()}
            />
          </Card>
        ) : null}
        {localError || error ? <Text style={textStyles.error}>{localError || error}</Text> : null}
      </ScrollView>
    </KeyboardAvoidingView>
  );
}

const styles = StyleSheet.create({
  root: { flex: 1, backgroundColor: colors.background },
  content: { flexGrow: 1, justifyContent: "center", padding: 24, gap: 16 },
  hero: { gap: 10, marginBottom: 12 },
  mark: {
    color: colors.background,
    backgroundColor: colors.primary,
    width: 52,
    height: 52,
    borderRadius: 16,
    textAlign: "center",
    textAlignVertical: "center",
    lineHeight: 52,
    fontWeight: "900",
    fontSize: 20,
    overflow: "hidden"
  }
});
