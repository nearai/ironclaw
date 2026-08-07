import {
  AbsoluteFill,
  interpolate,
  useCurrentFrame,
  useVideoConfig,
  Easing,
} from "remotion";
import { COLORS, FONTS } from "../theme";

const STEPS = [
  { cmd: "git clone", desc: "Clone the repo" },
  { cmd: "cargo test", desc: "Run the test suite" },
  { cmd: "cargo run", desc: "Start the assistant" },
  { cmd: "Read CLAUDE.md", desc: "Module specs are your guide" },
];

export const OutroScene: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const titleOpacity = interpolate(frame, [0, 0.5 * fps], [0, 1], {
    extrapolateRight: "clamp",
  });

  const titleY = interpolate(frame, [0, 0.5 * fps], [30, 0], {
    extrapolateRight: "clamp",
    easing: Easing.bezier(0.16, 1, 0.3, 1),
  });

  const lineWidth = interpolate(frame, [0.6 * fps, 1.2 * fps], [0, 300], {
    extrapolateRight: "clamp",
    easing: Easing.bezier(0.16, 1, 0.3, 1),
  });

  const footerOpacity = interpolate(
    frame,
    [3.0 * fps, 3.5 * fps],
    [0, 1],
    { extrapolateRight: "clamp" },
  );

  return (
    <AbsoluteFill
      style={{
        backgroundColor: COLORS.bg,
        fontFamily: FONTS.sans,
        justifyContent: "center",
        alignItems: "center",
      }}
    >
      {/* Grid background */}
      <AbsoluteFill
        style={{
          opacity: 0.05,
          backgroundImage: `linear-gradient(${COLORS.primary} 1px, transparent 1px), linear-gradient(90deg, ${COLORS.primary} 1px, transparent 1px)`,
          backgroundSize: "60px 60px",
        }}
      />

      {/* Glow */}
      <div
        style={{
          position: "absolute",
          width: 500,
          height: 500,
          borderRadius: "50%",
          background: `radial-gradient(circle, ${COLORS.accent}15 0%, transparent 70%)`,
          top: "50%",
          left: "50%",
          transform: "translate(-50%, -50%)",
        }}
      />

      <div
        style={{
          opacity: titleOpacity,
          transform: `translateY(${titleY}px)`,
          fontSize: 52,
          fontWeight: 800,
          color: COLORS.text,
          marginBottom: 12,
          textAlign: "center",
        }}
      >
        Start <span style={{ color: COLORS.accent }}>Contributing</span>
      </div>

      <div
        style={{
          width: lineWidth,
          height: 3,
          backgroundColor: COLORS.accent,
          borderRadius: 2,
          marginBottom: 48,
        }}
      />

      {/* Getting started steps */}
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: 16,
          width: 600,
        }}
      >
        {STEPS.map((step, i) => {
          const delay = 1.0 + i * 0.35;
          const opacity = interpolate(
            frame,
            [delay * fps, (delay + 0.3) * fps],
            [0, 1],
            { extrapolateLeft: "clamp", extrapolateRight: "clamp" },
          );
          const x = interpolate(
            frame,
            [delay * fps, (delay + 0.3) * fps],
            [20, 0],
            {
              extrapolateLeft: "clamp",
              extrapolateRight: "clamp",
              easing: Easing.bezier(0.16, 1, 0.3, 1),
            },
          );

          return (
            <div
              key={step.cmd}
              style={{
                opacity,
                transform: `translateX(${x}px)`,
                display: "flex",
                alignItems: "center",
                gap: 20,
              }}
            >
              <div
                style={{
                  width: 36,
                  height: 36,
                  borderRadius: "50%",
                  backgroundColor: COLORS.accent,
                  color: COLORS.bg,
                  display: "flex",
                  justifyContent: "center",
                  alignItems: "center",
                  fontSize: 18,
                  fontWeight: 800,
                  flexShrink: 0,
                }}
              >
                {i + 1}
              </div>
              <div
                style={{
                  fontSize: 20,
                  fontWeight: 700,
                  color: COLORS.primary,
                  fontFamily: FONTS.mono,
                  minWidth: 200,
                }}
              >
                {step.cmd}
              </div>
              <div
                style={{
                  fontSize: 16,
                  color: COLORS.textMuted,
                }}
              >
                {step.desc}
              </div>
            </div>
          );
        })}
      </div>

      {/* Footer */}
      <div
        style={{
          position: "absolute",
          bottom: 60,
          opacity: footerOpacity,
          fontSize: 18,
          color: COLORS.textMuted,
          fontFamily: FONTS.mono,
          textAlign: "center",
        }}
      >
        Built with Rust + Tokio &bull; All I/O is async &bull; cargo fmt
        &amp;&amp; cargo clippy &amp;&amp; cargo test
      </div>
    </AbsoluteFill>
  );
};
