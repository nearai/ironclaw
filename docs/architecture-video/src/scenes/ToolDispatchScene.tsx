import {
  AbsoluteFill,
  interpolate,
  useCurrentFrame,
  useVideoConfig,
  Easing,
} from "remotion";
import { COLORS, FONTS } from "../theme";

const STEPS = [
  {
    n: 1,
    label: "Tool resolution",
    code: "ToolRegistry::get_resolved(name)",
    color: COLORS.cyan,
  },
  {
    n: 2,
    label: "Param normalization",
    code: "prepare_tool_params()",
    color: COLORS.cyan,
  },
  {
    n: 3,
    label: "Injection validation",
    code: "SafetyLayer::validate_tool_params()",
    color: COLORS.danger,
  },
  {
    n: 4,
    label: "Schema validation",
    code: "jsonschema::validate(schema, params)",
    color: COLORS.primary,
  },
  {
    n: 5,
    label: "Sensitive param redaction",
    code: "redact_params() → [REDACTED]",
    color: COLORS.danger,
  },
  {
    n: 6,
    label: "System job creation",
    code: "store.create_system_job(user, src)",
    color: COLORS.success,
  },
  {
    n: 7,
    label: "Execute with timeout",
    code: "timeout(tool.execution_timeout(), ...)",
    color: COLORS.accent,
  },
  {
    n: 8,
    label: "Output sanitization",
    code: "SafetyLayer::sanitize_tool_output()",
    color: COLORS.danger,
  },
  {
    n: 9,
    label: "Audit persistence",
    code: "store.save_action(job_id, &action)",
    color: COLORS.success,
  },
];

export const ToolDispatchScene: React.FC = () => {
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
        <span style={{ color: COLORS.accent }}>ToolDispatcher::dispatch()</span>
      </div>
      <div
        style={{
          opacity: headingOpacity,
          fontSize: 14,
          color: COLORS.textMuted,
          marginBottom: 22,
          fontFamily: FONTS.mono,
        }}
      >
        src/tools/dispatch.rs:116 &bull; every action (channel / routine /
        system) flows through this
      </div>

      <div
        style={{
          display: "grid",
          gridTemplateColumns: "1fr 1fr 1fr",
          gap: 12,
          flex: 1,
        }}
      >
        {STEPS.map((s, i) => {
          const delay = 0.3 + i * 0.3;
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

          return (
            <div
              key={s.n}
              style={{
                opacity,
                transform: `scale(${scale})`,
                backgroundColor: COLORS.bgLight,
                border: `1px solid ${COLORS.border}`,
                borderLeft: `4px solid ${s.color}`,
                borderRadius: 10,
                padding: "14px 18px",
                display: "flex",
                flexDirection: "column",
                gap: 6,
              }}
            >
              <div
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 10,
                }}
              >
                <div
                  style={{
                    width: 26,
                    height: 26,
                    borderRadius: "50%",
                    backgroundColor: s.color,
                    color: COLORS.bg,
                    display: "flex",
                    justifyContent: "center",
                    alignItems: "center",
                    fontSize: 13,
                    fontWeight: 800,
                    fontFamily: FONTS.mono,
                    flexShrink: 0,
                  }}
                >
                  {s.n}
                </div>
                <div
                  style={{
                    fontSize: 16,
                    fontWeight: 700,
                    color: COLORS.text,
                  }}
                >
                  {s.label}
                </div>
              </div>
              <div
                style={{
                  fontSize: 11,
                  color: COLORS.textMuted,
                  fontFamily: FONTS.mono,
                  marginLeft: 36,
                }}
              >
                {s.code}
              </div>
            </div>
          );
        })}
      </div>

      {/* Key insight callout */}
      {(() => {
        const calloutOpacity = interpolate(
          frame,
          [3.5 * fps, 4.2 * fps],
          [0, 1],
          { extrapolateLeft: "clamp", extrapolateRight: "clamp" },
        );
        return (
          <div
            style={{
              opacity: calloutOpacity,
              marginTop: 16,
              fontSize: 13,
              color: COLORS.textMuted,
              fontFamily: FONTS.mono,
              backgroundColor: `${COLORS.accent}10`,
              border: `1px solid ${COLORS.accent}40`,
              borderRadius: 8,
              padding: "10px 16px",
            }}
          >
            <span style={{ color: COLORS.accent, fontWeight: 700 }}>
              Key:{" "}
            </span>
            Tools receive raw params, audit row gets redacted params +
            sanitized output
          </div>
        );
      })()}
    </AbsoluteFill>
  );
};
