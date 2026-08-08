import {
  AbsoluteFill,
  interpolate,
  useCurrentFrame,
  useVideoConfig,
  Easing,
} from "remotion";
import { COLORS, FONTS } from "../theme";

const PRIMITIVES = [
  {
    name: "Thread",
    replaces: "Session + Job + Routine + Sub-agent",
    desc: "Unit of work. Has state, config, messages, leases, events.",
    color: COLORS.primary,
    file: "crates/ironclaw_engine/src/types/thread.rs",
  },
  {
    name: "Step",
    replaces: "Turn + LLM Call + Tool Call",
    desc: "Unit of execution. LLM call + subsequent action executions.",
    color: COLORS.cyan,
    file: "crates/ironclaw_engine/src/types/step.rs",
  },
  {
    name: "Capability",
    replaces: "Tool + Skill + Hook",
    desc: "Unit of effect with leases and policies.",
    color: COLORS.accent,
    file: "crates/ironclaw_engine/src/capability/",
  },
  {
    name: "MemoryDoc",
    replaces: "Summary + Lesson + Skill + Note",
    desc: "Durable knowledge. Injected into context on thread start.",
    color: COLORS.success,
    file: "crates/ironclaw_engine/src/types/memory.rs",
  },
  {
    name: "Project",
    replaces: "Workspace + Namespace",
    desc: "Context scope. Owns memory, threads, missions.",
    color: COLORS.purple,
    file: "crates/ironclaw_engine/src/types/project.rs",
  },
];

export const PrimitivesScene: React.FC = () => {
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
          marginBottom: 8,
        }}
      >
        <span style={{ color: COLORS.primary }}>Engine v2</span> &mdash; Five
        Primitives
      </div>
      <div
        style={{
          opacity: headingOpacity,
          fontSize: 16,
          color: COLORS.textMuted,
          marginBottom: 36,
          fontFamily: FONTS.mono,
        }}
      >
        crates/ironclaw_engine/ &mdash; replaces ~10 v1 abstractions
      </div>

      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: 14,
        }}
      >
        {PRIMITIVES.map((p, i) => {
          const delay = 0.4 + i * 0.35;
          const opacity = interpolate(
            frame,
            [delay * fps, (delay + 0.4) * fps],
            [0, 1],
            { extrapolateLeft: "clamp", extrapolateRight: "clamp" },
          );
          const x = interpolate(
            frame,
            [delay * fps, (delay + 0.4) * fps],
            [-40, 0],
            {
              extrapolateLeft: "clamp",
              extrapolateRight: "clamp",
              easing: Easing.bezier(0.16, 1, 0.3, 1),
            },
          );

          return (
            <div
              key={p.name}
              style={{
                opacity,
                transform: `translateX(${x}px)`,
                backgroundColor: COLORS.bgLight,
                border: `1px solid ${COLORS.border}`,
                borderLeft: `5px solid ${p.color}`,
                borderRadius: 10,
                padding: "16px 24px",
                display: "flex",
                alignItems: "center",
                gap: 24,
              }}
            >
              <div
                style={{
                  fontSize: 26,
                  fontWeight: 800,
                  color: p.color,
                  fontFamily: FONTS.mono,
                  minWidth: 180,
                }}
              >
                {p.name}
              </div>
              <div style={{ flex: 1 }}>
                <div
                  style={{
                    fontSize: 15,
                    color: COLORS.text,
                    marginBottom: 4,
                  }}
                >
                  {p.desc}
                </div>
                <div
                  style={{
                    fontSize: 12,
                    color: COLORS.textMuted,
                    fontFamily: FONTS.mono,
                  }}
                >
                  replaces: {p.replaces}
                </div>
              </div>
              <div
                style={{
                  fontSize: 11,
                  color: COLORS.textMuted,
                  fontFamily: FONTS.mono,
                  maxWidth: 260,
                  textAlign: "right",
                }}
              >
                {p.file}
              </div>
            </div>
          );
        })}
      </div>
    </AbsoluteFill>
  );
};
