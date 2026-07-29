// Bootstrap snapshot written by the Rust-served index.html before React
// mounts; read by theme/theme.ts to avoid first-paint theme flicker.
// Mirrors the declaration in the app's vite-env.d.ts (identical optional
// members merge cleanly when both are in the same TypeScript program).
interface Window {
  __IRONCLAW_INITIAL_THEME__?: "light" | "dark";
}
