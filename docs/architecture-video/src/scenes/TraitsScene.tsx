import {
  AbsoluteFill,
  interpolate,
  useCurrentFrame,
  useVideoConfig,
  Easing,
} from "remotion";
import { COLORS, FONTS } from "../theme";

const TRAITS = [
  {
    name: "Database",
    file: "src/db/mod.rs",
    impls: ["PgBackend", "LibSqlBackend"],
  },
  {
    name: "Tool",
    file: "src/tools/tool.rs",
    impls: ["60+ builtins", "McpToolWrapper", "WasmToolWrapper"],
  },
  {
    name: "LlmProvider",
    file: "src/llm/provider.rs",
    impls: ["Anthropic", "OpenAI", "Bedrock", "NearAi", "+ 11 decorators"],
  },
  {
    name: "EmbeddingProvider",
    file: "src/workspace/embeddings.rs",
    impls: ["OpenAi", "NearAi", "Bedrock", "Ollama", "Cached"],
  },
  {
    name: "NetworkPolicyDecider",
    file: "src/sandbox/proxy/policy.rs",
    impls: ["Default", "AllowAll", "DenyAll"],
  },
  {
    name: "Hook",
    file: "src/hooks/hook.rs",
    impls: ["AuditLog", "Rule", "OutboundWebhook", "SessionStart"],
  },
  {
    name: "Observer",
    file: "src/observability/traits.rs",
    impls: ["Noop", "Log", "Multi"],
  },
  {
    name: "Tunnel",
    file: "src/tunnel/mod.rs",
    impls: ["None", "Cloudflare", "Ngrok", "Tailscale", "Custom"],
  },
];

export const TraitsScene: React.FC = () => {
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
        <span style={{ color: COLORS.purple }}>Extensibility</span> &mdash;
        traits + implementers
      </div>
      <div
        style={{
          opacity: headingOpacity,
          fontSize: 14,
          color: COLORS.textMuted,
          marginBottom: 24,
          fontFamily: FONTS.mono,
        }}
      >
        impl YourTrait for YourType &mdash; plug in without touching core
      </div>

      <div
        style={{
          display: "grid",
          gridTemplateColumns: "1fr 1fr",
          gap: 12,
          flex: 1,
        }}
      >
        {TRAITS.map((t, i) => {
          const delay = 0.4 + i * 0.22;
          const opacity = interpolate(
            frame,
            [delay * fps, (delay + 0.35) * fps],
            [0, 1],
            { extrapolateLeft: "clamp", extrapolateRight: "clamp" },
          );
          const x = interpolate(
            frame,
            [delay * fps, (delay + 0.35) * fps],
            [i % 2 === 0 ? -30 : 30, 0],
            {
              extrapolateLeft: "clamp",
              extrapolateRight: "clamp",
              easing: Easing.bezier(0.16, 1, 0.3, 1),
            },
          );

          return (
            <div
              key={t.name}
              style={{
                opacity,
                transform: `translateX(${x}px)`,
                backgroundColor: COLORS.bgLight,
                border: `1px solid ${COLORS.border}`,
                borderLeft: `4px solid ${COLORS.purple}`,
                borderRadius: 10,
                padding: "14px 20px",
                display: "flex",
                flexDirection: "column",
                gap: 6,
              }}
            >
              <div
                style={{
                  display: "flex",
                  alignItems: "baseline",
                  justifyContent: "space-between",
                  gap: 12,
                }}
              >
                <div
                  style={{
                    fontSize: 20,
                    fontWeight: 800,
                    color: COLORS.purple,
                    fontFamily: FONTS.mono,
                  }}
                >
                  {t.name}
                </div>
                <div
                  style={{
                    fontSize: 11,
                    color: COLORS.textMuted,
                    fontFamily: FONTS.mono,
                  }}
                >
                  {t.file}
                </div>
              </div>
              <div
                style={{
                  fontSize: 12,
                  fontFamily: FONTS.mono,
                  display: "flex",
                  flexWrap: "wrap",
                  gap: 6,
                }}
              >
                {t.impls.map((impl) => (
                  <span
                    key={impl}
                    style={{
                      backgroundColor: "#0b1120",
                      border: `1px solid ${COLORS.border}`,
                      borderRadius: 4,
                      padding: "2px 8px",
                      color: COLORS.cyan,
                    }}
                  >
                    {impl}
                  </span>
                ))}
              </div>
            </div>
          );
        })}
      </div>
    </AbsoluteFill>
  );
};
