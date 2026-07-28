import React from "react";
import { Ionicons } from "@expo/vector-icons";
import { Pressable, StyleSheet } from "react-native";
import { useNavigation } from "expo-router";
import { colors } from "@/theme";

export function DrawerButton() {
  const navigation = useNavigation();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel="Open navigation"
      onPress={() => (navigation as unknown as { openDrawer: () => void }).openDrawer()}
      style={styles.button}
    >
      <Ionicons name="menu-outline" size={21} color={colors.text} />
    </Pressable>
  );
}

const styles = StyleSheet.create({
  button: { position: "absolute", top: 10, left: 10, zIndex: 5, width: 38, height: 38, borderRadius: 19, alignItems: "center", justifyContent: "center", backgroundColor: colors.surfaceRaised, borderColor: colors.border, borderWidth: 1 }
});
