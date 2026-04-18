import {
  AbsoluteFill,
  interpolate,
  useCurrentFrame,
  useVideoConfig,
  Easing,
} from "remotion";
import { COLORS, FONTS } from "../theme";

const STAGES = [
  {
    n: 1,
    name: "Gating",
    color: COLORS.cyan,
    desc: "Check requires: bins, env, config",
    detail: "Skip skills whose prerequisites are missing",
    example: "requires: { bins: [ffmpeg], env: [API_KEY] }",
  },
  {
    n: 2,
    name: "Scoring",
    color: COLORS.primary,
    desc: "Deterministic relevance score",
    detail: "keywords (10/5, cap 30) + patterns (20, cap 40) + tags (3, cap 15)",
    example: "exclude_keywords veto → score = 0",
  },
  {
    n: 3,
    name: "Budget",
    color: COLORS.accent,
    desc: "Fit within SKILLS_MAX_TOKENS",
    detail: "Select top-scoring skills within prompt budget",
    example: "num_tokens_from_string(frontmatter + body)",
  },
  {
    n: 4,
    name: "Attenuation",
    color: COLORS.danger,
    desc: "Minimum trust determines tool ceiling",
    detail: "Trusted: all tools  |  Installed: read-only only",
    example: "memory_search, memory_read, time, echo, json",
  },
];

export const SkillsPipelineScene: React.FC = () => {
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
        <span style={{ color: COLORS.accentLight }}>Skills</span> &mdash;
        selection pipeline
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
        src/skills/ &bull; SKILL.md + YAML frontmatter &bull; called from Python
        via __list_skills__()
      </div>

      <div
        style={{
          display: "flex",
          gap: 12,
          flex: 1,
          alignItems: "stretch",
        }}
      >
        {STAGES.map((s, i) => {
          const delay = 0.4 + i * 0.45;
          const opacity = interpolate(
            frame,
            [delay * fps, (delay + 0.4) * fps],
            [0, 1],
            { extrapolateLeft: "clamp", extrapolateRight: "clamp" },
          );
          const y = interpolate(
            frame,
            [delay * fps, (delay + 0.4) * fps],
            [30, 0],
            {
              extrapolateLeft: "clamp",
              extrapolateRight: "clamp",
              easing: Easing.bezier(0.16, 1, 0.3, 1),
            },
          );
          const arrowProgress =
            i < STAGES.length - 1
              ? interpolate(
                  frame,
                  [(delay + 0.3) * fps, (delay + 0.55) * fps],
                  [0, 1],
                  { extrapolateLeft: "clamp", extrapolateRight: "clamp" },
                )
              : 0;

          return (
            <div
              key={s.n}
              style={{
                flex: 1,
                position: "relative",
                display: "flex",
              }}
            >
              <div
                style={{
                  flex: 1,
                  opacity,
                  transform: `translateY(${y}px)`,
                  backgroundColor: COLORS.bgLight,
                  border: `1px solid ${COLORS.border}`,
                  borderTop: `4px solid ${s.color}`,
                  borderRadius: 12,
                  padding: "20px 22px",
                  display: "flex",
                  flexDirection: "column",
                }}
              >
                <div
                  style={{
                    fontSize: 12,
                    color: COLORS.textMuted,
                    fontFamily: FONTS.mono,
                    marginBottom: 4,
                  }}
                >
                  STAGE {s.n}
                </div>
                <div
                  style={{
                    fontSize: 26,
                    fontWeight: 800,
                    color: s.color,
                    marginBottom: 12,
                  }}
                >
                  {s.name}
                </div>
                <div
                  style={{
                    fontSize: 14,
                    color: COLORS.text,
                    marginBottom: 14,
                    fontWeight: 600,
                  }}
                >
                  {s.desc}
                </div>
                <div
                  style={{
                    fontSize: 12,
                    color: COLORS.textMuted,
                    marginBottom: 18,
                    lineHeight: 1.5,
                  }}
                >
                  {s.detail}
                </div>
                <div
                  style={{
                    marginTop: "auto",
                    fontSize: 11,
                    color: s.color,
                    fontFamily: FONTS.mono,
                    backgroundColor: "#0b1120",
                    padding: "8px 10px",
                    borderRadius: 6,
                    border: `1px solid ${COLORS.border}`,
                  }}
                >
                  {s.example}
                </div>
              </div>
              {i < STAGES.length - 1 && (
                <div
                  style={{
                    position: "absolute",
                    right: -14,
                    top: "50%",
                    transform: `translateY(-50%) scaleX(${arrowProgress})`,
                    transformOrigin: "left",
                    fontSize: 28,
                    color: s.color,
                    fontWeight: 800,
                    opacity: arrowProgress,
                    zIndex: 10,
                  }}
                >
                  →
                </div>
              )}
            </div>
          );
        })}
      </div>
    </AbsoluteFill>
  );
};
