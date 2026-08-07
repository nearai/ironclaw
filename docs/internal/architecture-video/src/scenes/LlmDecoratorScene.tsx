import {
  AbsoluteFill,
  interpolate,
  useCurrentFrame,
  useVideoConfig,
  Easing,
} from "remotion";
import { COLORS, FONTS } from "../theme";

const LAYERS = [
  {
    name: "SmartRouting",
    desc: "pick provider by cost/latency/capability",
    color: COLORS.primary,
  },
  {
    name: "CircuitBreaker",
    desc: "fail fast on downstream outage",
    color: COLORS.danger,
  },
  {
    name: "Retry",
    desc: "exponential backoff + jitter",
    color: COLORS.accent,
  },
  {
    name: "Failover",
    desc: "secondary provider on primary failure",
    color: COLORS.purple,
  },
  {
    name: "Cached",
    desc: "prompt-hash cache for deterministic calls",
    color: COLORS.cyan,
  },
  {
    name: "TokenRefreshing",
    desc: "OAuth token rotation",
    color: COLORS.success,
  },
  {
    name: "Base Provider",
    desc: "Anthropic / OpenAI / Bedrock / NearAi / Ollama",
    color: COLORS.pink,
    base: true,
  },
];

export const LlmDecoratorScene: React.FC = () => {
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
        <span style={{ color: COLORS.primary }}>LlmProvider</span> &mdash;
        decorator chain
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
        src/llm/provider.rs &bull; each decorator wraps the next, same trait
      </div>

      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: 0,
          maxWidth: 900,
          margin: "0 auto",
          width: "100%",
        }}
      >
        {LAYERS.map((layer, i) => {
          const delay = 0.3 + i * 0.35;
          const opacity = interpolate(
            frame,
            [delay * fps, (delay + 0.35) * fps],
            [0, 1],
            { extrapolateLeft: "clamp", extrapolateRight: "clamp" },
          );
          const scale = interpolate(
            frame,
            [delay * fps, (delay + 0.35) * fps],
            [0.9, 1],
            {
              extrapolateLeft: "clamp",
              extrapolateRight: "clamp",
              easing: Easing.bezier(0.16, 1, 0.3, 1),
            },
          );

          const indent = layer.base ? 0 : i * 8;

          return (
            <div
              key={layer.name}
              style={{
                opacity,
                transform: `scale(${scale})`,
                marginLeft: indent,
                marginRight: indent,
                backgroundColor: layer.base
                  ? `${layer.color}20`
                  : COLORS.bgLight,
                border: `2px solid ${layer.color}`,
                borderRadius: 10,
                padding: "12px 22px",
                display: "flex",
                alignItems: "center",
                gap: 16,
                marginBottom: 6,
              }}
            >
              <div
                style={{
                  fontSize: 11,
                  color: COLORS.textMuted,
                  fontFamily: FONTS.mono,
                  minWidth: 24,
                }}
              >
                {layer.base ? "■" : `${i + 1}.`}
              </div>
              <div
                style={{
                  fontSize: 18,
                  fontWeight: 800,
                  color: layer.color,
                  fontFamily: FONTS.mono,
                  minWidth: 220,
                }}
              >
                {layer.name}
              </div>
              <div
                style={{
                  fontSize: 13,
                  color: COLORS.textMuted,
                  fontFamily: FONTS.mono,
                }}
              >
                {layer.desc}
              </div>
            </div>
          );
        })}
      </div>

      {(() => {
        const flowOpacity = interpolate(
          frame,
          [3.5 * fps, 4.0 * fps],
          [0, 1],
          { extrapolateLeft: "clamp", extrapolateRight: "clamp" },
        );
        return (
          <div
            style={{
              opacity: flowOpacity,
              textAlign: "center",
              marginTop: 16,
              fontSize: 13,
              color: COLORS.textMuted,
              fontFamily: FONTS.mono,
            }}
          >
            agent → top of chain → ... → base provider → HTTP call
          </div>
        );
      })()}
    </AbsoluteFill>
  );
};
