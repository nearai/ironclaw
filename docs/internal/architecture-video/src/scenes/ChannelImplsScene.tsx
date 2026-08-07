import {
  AbsoluteFill,
  interpolate,
  useCurrentFrame,
  useVideoConfig,
  Easing,
} from "remotion";
import { COLORS, FONTS } from "../theme";

const CHANNELS = [
  {
    icon: "⌨",
    name: "REPL",
    file: "src/channels/repl.rs",
    input: "stdin via rustyline",
    output: "stdout + termimad markdown",
    color: COLORS.primary,
  },
  {
    icon: "🌐",
    name: "HTTP",
    file: "src/channels/http.rs",
    input: "POST + HMAC-SHA256 validation",
    output: "oneshot response channel",
    color: COLORS.cyan,
  },
  {
    icon: "💻",
    name: "Web",
    file: "src/channels/web/mod.rs",
    input: "SSE/WebSocket + bearer auth",
    output: "SseManager::broadcast_for_user()",
    color: COLORS.accent,
  },
  {
    icon: "📱",
    name: "Signal",
    file: "src/channels/signal.rs",
    input: "signal-cli SSE /api/v1/events",
    output: "JSON-RPC to /api/v1/rpc",
    color: COLORS.success,
  },
  {
    icon: "📺",
    name: "TUI",
    file: "src/channels/cli/ (ratatui)",
    input: "crossterm key + mouse events",
    output: "direct buffer render",
    color: COLORS.purple,
  },
  {
    icon: "🧩",
    name: "WASM",
    file: "src/channels/wasm/wrapper.rs",
    input: "dynamic module + host_bridge",
    output: "host bridge callbacks",
    color: COLORS.pink,
  },
];

export const ChannelImplsScene: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const headingOpacity = interpolate(frame, [0, 0.4 * fps], [0, 1], {
    extrapolateRight: "clamp",
  });

  return (
    <AbsoluteFill
      style={{
        backgroundColor: COLORS.bg,
        fontFamily: FONTS.sans,
        padding: 60,
      }}
    >
      <div
        style={{
          opacity: headingOpacity,
          fontSize: 42,
          fontWeight: 700,
          color: COLORS.text,
          marginBottom: 4,
        }}
      >
        <span style={{ color: COLORS.cyan }}>Channel</span> implementations
      </div>
      <div
        style={{
          opacity: headingOpacity,
          fontSize: 14,
          color: COLORS.textMuted,
          marginBottom: 30,
          fontFamily: FONTS.mono,
        }}
      >
        each implements the same 8-method trait &bull; plugged via
        ChannelManager::register()
      </div>

      <div
        style={{
          display: "grid",
          gridTemplateColumns: "1fr 1fr 1fr",
          gap: 16,
          flex: 1,
        }}
      >
        {CHANNELS.map((c, i) => {
          const delay = 0.4 + i * 0.3;
          const opacity = interpolate(
            frame,
            [delay * fps, (delay + 0.35) * fps],
            [0, 1],
            { extrapolateLeft: "clamp", extrapolateRight: "clamp" },
          );
          const y = interpolate(
            frame,
            [delay * fps, (delay + 0.35) * fps],
            [30, 0],
            {
              extrapolateLeft: "clamp",
              extrapolateRight: "clamp",
              easing: Easing.bezier(0.16, 1, 0.3, 1),
            },
          );

          return (
            <div
              key={c.name}
              style={{
                opacity,
                transform: `translateY(${y}px)`,
                backgroundColor: COLORS.bgLight,
                border: `1px solid ${COLORS.border}`,
                borderTop: `4px solid ${c.color}`,
                borderRadius: 12,
                padding: "18px 22px",
                display: "flex",
                flexDirection: "column",
                gap: 10,
              }}
            >
              <div
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 12,
                }}
              >
                <div style={{ fontSize: 28 }}>{c.icon}</div>
                <div
                  style={{
                    fontSize: 22,
                    fontWeight: 800,
                    color: c.color,
                  }}
                >
                  {c.name}
                </div>
              </div>
              <div
                style={{
                  fontSize: 11,
                  color: COLORS.textMuted,
                  fontFamily: FONTS.mono,
                }}
              >
                {c.file}
              </div>
              <div style={{ fontSize: 12, marginTop: 4 }}>
                <div style={{ color: COLORS.success, marginBottom: 4 }}>
                  <span style={{ color: COLORS.textMuted }}>in:</span>{" "}
                  {c.input}
                </div>
                <div style={{ color: COLORS.accent }}>
                  <span style={{ color: COLORS.textMuted }}>out:</span>{" "}
                  {c.output}
                </div>
              </div>
            </div>
          );
        })}
      </div>
    </AbsoluteFill>
  );
};
