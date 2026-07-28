import { Drawer } from "expo-router/drawer";
import { usePathname } from "expo-router";
import { Platform } from "react-native";
import { StatusBar } from "expo-status-bar";
import { SessionProvider } from "@/auth/session-context";
import { colors } from "@/theme";
import { AppDrawer } from "@/components/app-drawer";

export default function RootLayout() {
  const pathname = usePathname();
  const focusedWorkspace = pathname.startsWith("/thread/") || pathname.startsWith("/automations") || pathname.startsWith("/settings");
  return (
    <SessionProvider>
      <StatusBar style="light" />
      <Drawer
        drawerContent={(props) => <AppDrawer {...props} />}
        screenOptions={{
          headerShown: false,
          drawerType: Platform.OS === "web" && !focusedWorkspace ? "permanent" : "front",
          swipeEnabled: true,
          drawerStyle: { backgroundColor: colors.surface, width: 300 },
          sceneStyle: { backgroundColor: colors.background }
        }}
      >
        <Drawer.Screen name="index" options={{ drawerItemStyle: { display: "none" } }} />
        <Drawer.Screen name="login" options={{ drawerItemStyle: { display: "none" } }} />
        <Drawer.Screen name="(tabs)" options={{ drawerItemStyle: { display: "none" } }} />
        <Drawer.Screen name="thread/[id]" options={{ drawerItemStyle: { display: "none" } }} />
      </Drawer>
    </SessionProvider>
  );
}
