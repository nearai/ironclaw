/** @type {import("expo/config").ExpoConfig} */
const config = (() => {
  const buildProfile = process.env.IRONCLAW_APP_ENV ?? "development";
  const production = buildProfile === "production";

  return {
    name: production ? "IronClaw" : `IronClaw ${buildProfile}`,
    slug: "ironclaw",
    owner: "near-ai",
    scheme: production ? "ironclaw" : `ironclaw-${buildProfile}`,
    version: "0.1.0",
    orientation: "portrait",
    userInterfaceStyle: "automatic",
    newArchEnabled: true,
    runtimeVersion: {
      policy: "fingerprint"
    },
    updates: {
      url: "https://u.expo.dev/229a2c75-ae46-4492-bf60-00ea5887c199"
    },
    ios: {
      bundleIdentifier: production ? "ai.near.ironclaw" : `ai.near.ironclaw.${buildProfile}`,
      deploymentTarget: "16.4",
      supportsTablet: true
    },
    android: {
      package: production ? "ai.near.ironclaw" : `ai.near.ironclaw.${buildProfile}`,
      minSdkVersion: 33,
      compileSdkVersion: 36,
      targetSdkVersion: 36,
      adaptiveIcon: {
        backgroundColor: "#0b1020"
      }
    },
    web: {
      bundler: "metro"
    },
    plugins: [
      "expo-router",
      "expo-font",
      [
        "expo-secure-store",
        {
          configureAndroidBackup: true,
          faceIDPermission: "Allow IronClaw to unlock your agent session."
        }
      ],
      [
        "expo-sqlite",
        {
          useSQLCipher: true
        }
      ]
    ],
    experiments: {
      typedRoutes: true
    },
    extra: {
      buildProfile,
      hostedOrigin: production ? "https://agent.near.ai" : "https://agent-stg.near.ai",
      eas: {
        projectId: "229a2c75-ae46-4492-bf60-00ea5887c199"
      }
    }
  };
})();

module.exports = config;
