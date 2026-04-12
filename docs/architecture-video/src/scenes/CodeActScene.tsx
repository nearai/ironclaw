import {
  AbsoluteFill,
  interpolate,
  useCurrentFrame,
  useVideoConfig,
  Easing,
  Sequence,
} from "remotion";
import { COLORS, FONTS } from "../theme";
import { CodeBlock } from "../components/Code";

const PYTHON_CODE = `# Monty VM (NOT CPython) - injected context:
#   context, goal, step_number, user_timezone

search = await web_search(query=goal)
summary = llm_query(
  prompt="Summarize findings",
  context=search
)

if needs_approval(summary):
  mission_create(
    name="follow_up",
    cadence="0 9 * * *",
  )

FINAL(summary)`;

const HOST_FNS = [
  "__execute_action__(name, params)",
  "__list_skills__()",
  "__llm_query__(prompt, context)",
  "__memory_search__(query)",
  "__policy_check__(action)",
];

export const CodeActScene: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const headingOpacity = interpolate(frame, [0, 0.4 * fps], [0, 1], {
    extrapolateRight: "clamp",
  });

  const codeOpacity = interpolate(
    frame,
    [0.5 * fps, 1.2 * fps],
    [0, 1],
    { extrapolateLeft: "clamp", extrapolateRight: "clamp" },
  );

  const codeX = interpolate(frame, [0.5 * fps, 1.2 * fps], [-30, 0], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
    easing: Easing.bezier(0.16, 1, 0.3, 1),
  });

  const arrowProgress = interpolate(
    frame,
    [2.5 * fps, 3.2 * fps],
    [0, 1],
    { extrapolateLeft: "clamp", extrapolateRight: "clamp" },
  );

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
        <span style={{ color: COLORS.accent }}>CodeAct</span> &mdash; LLM
        writes Python, Monty executes
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
        prompts/codeact_preamble.md &bull;
        executor/orchestrator.rs
      </div>

      {/* Left: Python code */}
      <div
        style={{
          position: "absolute",
          left: 60,
          top: 160,
          width: 560,
          opacity: codeOpacity,
          transform: `translateX(${codeX}px)`,
        }}
      >
        <div
          style={{
            fontSize: 14,
            color: COLORS.textMuted,
            fontFamily: FONTS.mono,
            marginBottom: 8,
            textTransform: "uppercase",
            letterSpacing: 2,
          }}
        >
          ① LLM output
        </div>
        <CodeBlock code={PYTHON_CODE} fontSize={14} />
      </div>

      {/* Arrow */}
      <svg
        width="120"
        height="60"
        style={{
          position: "absolute",
          left: 620,
          top: 380,
          opacity: arrowProgress,
        }}
      >
        <line
          x1={0}
          y1={30}
          x2={100}
          y2={30}
          stroke={COLORS.accent}
          strokeWidth={3}
          strokeDasharray={100}
          strokeDashoffset={100 * (1 - arrowProgress)}
        />
        <polygon
          points="100,22 115,30 100,38"
          fill={COLORS.accent}
          opacity={arrowProgress > 0.8 ? 1 : 0}
        />
      </svg>

      {/* Right: Host function dispatch */}
      <Sequence from={Math.round(2.8 * fps)} layout="none">
        <HostFnPanel />
      </Sequence>

      {/* Bottom: suspension flow */}
      <Sequence from={Math.round(5.5 * fps)} layout="none">
        <SuspendFlow />
      </Sequence>
    </AbsoluteFill>
  );
};

const HostFnPanel: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const opacity = interpolate(frame, [0, 0.5 * fps], [0, 1], {
    extrapolateRight: "clamp",
  });
  const x = interpolate(frame, [0, 0.5 * fps], [30, 0], {
    extrapolateRight: "clamp",
    easing: Easing.bezier(0.16, 1, 0.3, 1),
  });

  return (
    <div
      style={{
        position: "absolute",
        left: 760,
        top: 160,
        width: 440,
        opacity,
        transform: `translateX(${x}px)`,
      }}
    >
      <div
        style={{
          fontSize: 14,
          color: COLORS.textMuted,
          fontFamily: FONTS.mono,
          marginBottom: 8,
          textTransform: "uppercase",
          letterSpacing: 2,
        }}
      >
        ② Rust host functions
      </div>
      <div
        style={{
          backgroundColor: "#0b1120",
          border: `1px solid ${COLORS.border}`,
          borderRadius: 10,
          padding: 20,
        }}
      >
        {HOST_FNS.map((fn, i) => {
          const delay = 0.2 + i * 0.2;
          const itemOpacity = interpolate(
            frame,
            [delay * fps, (delay + 0.3) * fps],
            [0, 1],
            { extrapolateLeft: "clamp", extrapolateRight: "clamp" },
          );
          return (
            <div
              key={fn}
              style={{
                opacity: itemOpacity,
                fontSize: 13,
                color: COLORS.cyan,
                fontFamily: FONTS.mono,
                padding: "6px 0",
                borderBottom:
                  i < HOST_FNS.length - 1
                    ? `1px solid ${COLORS.border}`
                    : "none",
              }}
            >
              <span style={{ color: COLORS.purple }}>fn</span> {fn}
            </div>
          );
        })}
      </div>
    </div>
  );
};

const SuspendFlow: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const boxes = [
    { label: "Python suspends", color: COLORS.accent },
    { label: "Lease check", color: COLORS.danger },
    { label: "Policy check", color: COLORS.danger },
    { label: "ToolDispatcher", color: COLORS.primary },
    { label: "Result → Python", color: COLORS.success },
  ];

  return (
    <div
      style={{
        position: "absolute",
        left: 60,
        right: 60,
        bottom: 60,
      }}
    >
      <div
        style={{
          fontSize: 14,
          color: COLORS.textMuted,
          fontFamily: FONTS.mono,
          marginBottom: 10,
          textTransform: "uppercase",
          letterSpacing: 2,
        }}
      >
        ③ Suspend → execute → resume
      </div>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 6,
        }}
      >
        {boxes.map((b, i) => {
          const delay = 0.3 + i * 0.35;
          const opacity = interpolate(
            frame,
            [delay * fps, (delay + 0.3) * fps],
            [0, 1],
            { extrapolateLeft: "clamp", extrapolateRight: "clamp" },
          );
          const scale = interpolate(
            frame,
            [delay * fps, (delay + 0.3) * fps],
            [0.85, 1],
            {
              extrapolateLeft: "clamp",
              extrapolateRight: "clamp",
              easing: Easing.bezier(0.16, 1, 0.3, 1),
            },
          );
          return (
            <div
              key={b.label}
              style={{
                display: "flex",
                alignItems: "center",
                gap: 6,
              }}
            >
              <div
                style={{
                  opacity,
                  transform: `scale(${scale})`,
                  backgroundColor: COLORS.bgLight,
                  border: `2px solid ${b.color}`,
                  borderRadius: 8,
                  padding: "10px 14px",
                  fontSize: 13,
                  fontWeight: 700,
                  color: b.color,
                  fontFamily: FONTS.mono,
                  whiteSpace: "nowrap",
                }}
              >
                {b.label}
              </div>
              {i < boxes.length - 1 && (
                <div
                  style={{
                    opacity: opacity * 0.7,
                    fontSize: 20,
                    color: COLORS.textMuted,
                  }}
                >
                  →
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
};
