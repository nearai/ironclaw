import {
  AbsoluteFill,
  interpolate,
  useCurrentFrame,
  useVideoConfig,
  Easing,
} from "remotion";
import { COLORS, FONTS } from "../theme";

const STEPS = [
  { n: 1, label: "Load checkpoint", detail: "resume from prior run" },
  { n: 2, label: "Transition → Running", detail: "ThreadState state machine" },
  { n: 3, label: "Pre-fetch memory docs", detail: "shared context injection" },
  { n: 4, label: "Inject CodeAct prompt", detail: "preamble + actions from leases" },
  { n: 5, label: "Load Python orchestrator", detail: "versioned, self-modify opt-in" },
  { n: 6, label: "execute_orchestrator()", detail: "Monty VM runs user code" },
  { n: 7, label: "Persist state + events", detail: "never deleted, full audit" },
];

export const ExecutionLoopScene: React.FC = () => {
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
        <span style={{ color: COLORS.cyan }}>ExecutionLoop::run()</span>
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
        crates/ironclaw_engine/src/executor/loop_engine.rs:188
      </div>

      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: 14,
        }}
      >
        {STEPS.map((s, i) => {
          const delay = 0.4 + i * 0.5;
          const opacity = interpolate(
            frame,
            [delay * fps, (delay + 0.35) * fps],
            [0, 1],
            { extrapolateLeft: "clamp", extrapolateRight: "clamp" },
          );
          const scale = interpolate(
            frame,
            [delay * fps, (delay + 0.35) * fps],
            [0.92, 1],
            {
              extrapolateLeft: "clamp",
              extrapolateRight: "clamp",
              easing: Easing.bezier(0.16, 1, 0.3, 1),
            },
          );

          // Connector line between steps
          const lineProgress =
            i < STEPS.length - 1
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
                position: "relative",
                opacity,
                transform: `scale(${scale})`,
                transformOrigin: "left center",
              }}
            >
              <div
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 20,
                }}
              >
                <div
                  style={{
                    width: 44,
                    height: 44,
                    borderRadius: "50%",
                    backgroundColor: COLORS.cyan,
                    color: COLORS.bg,
                    display: "flex",
                    justifyContent: "center",
                    alignItems: "center",
                    fontSize: 18,
                    fontWeight: 800,
                    flexShrink: 0,
                    fontFamily: FONTS.mono,
                  }}
                >
                  {s.n}
                </div>
                <div
                  style={{
                    flex: 1,
                    backgroundColor: COLORS.bgLight,
                    border: `1px solid ${COLORS.border}`,
                    borderRadius: 8,
                    padding: "10px 20px",
                    display: "flex",
                    alignItems: "center",
                    justifyContent: "space-between",
                  }}
                >
                  <div
                    style={{
                      fontSize: 18,
                      fontWeight: 700,
                      color: COLORS.text,
                      fontFamily: FONTS.mono,
                    }}
                  >
                    {s.label}
                  </div>
                  <div
                    style={{
                      fontSize: 13,
                      color: COLORS.textMuted,
                      fontFamily: FONTS.mono,
                    }}
                  >
                    {s.detail}
                  </div>
                </div>
              </div>
              {i < STEPS.length - 1 && (
                <div
                  style={{
                    position: "absolute",
                    left: 22,
                    top: 44,
                    width: 2,
                    height: 14,
                    backgroundColor: COLORS.cyan,
                    opacity: lineProgress,
                    transformOrigin: "top",
                    transform: `scaleY(${lineProgress})`,
                  }}
                />
              )}
            </div>
          );
        })}
      </div>
    </AbsoluteFill>
  );
};
