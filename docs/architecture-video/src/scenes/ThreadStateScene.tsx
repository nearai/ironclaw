import {
  AbsoluteFill,
  interpolate,
  useCurrentFrame,
  useVideoConfig,
  Easing,
} from "remotion";
import { COLORS, FONTS } from "../theme";

type State = {
  label: string;
  x: number;
  y: number;
  color: string;
};

const STATES: State[] = [
  { label: "Created", x: 50, y: 230, color: COLORS.textMuted },
  { label: "Running", x: 270, y: 230, color: COLORS.primary },
  { label: "Waiting", x: 510, y: 110, color: COLORS.accent },
  { label: "Suspended", x: 510, y: 350, color: COLORS.pink },
  { label: "Completed", x: 780, y: 150, color: COLORS.success },
  { label: "Failed", x: 780, y: 330, color: COLORS.danger },
  { label: "Done", x: 1000, y: 230, color: COLORS.cyan },
];

type Edge = { from: number; to: number; label?: string };
const EDGES: Edge[] = [
  { from: 0, to: 1, label: "spawn" },
  { from: 1, to: 2, label: "await tool" },
  { from: 1, to: 3, label: "checkpoint" },
  { from: 2, to: 1, label: "result" },
  { from: 3, to: 1, label: "resume" },
  { from: 1, to: 4, label: "FINAL()" },
  { from: 1, to: 5, label: "error" },
  { from: 4, to: 6 },
  { from: 5, to: 6 },
];

const NODE_W = 150;
const NODE_H = 52;

export const ThreadStateScene: React.FC = () => {
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
        <span style={{ color: COLORS.primary }}>ThreadState</span> &mdash;
        state machine
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
        crates/ironclaw_engine/src/types/thread.rs &bull; events persisted to
        Store (never deleted)
      </div>

      <svg
        width="1160"
        height="500"
        viewBox="0 0 1160 500"
        style={{ position: "absolute", top: 130, left: 60 }}
      >
        <defs>
          <marker
            id="ts-arrow"
            markerWidth="8"
            markerHeight="6"
            refX="8"
            refY="3"
            orient="auto"
          >
            <polygon points="0 0, 8 3, 0 6" fill={COLORS.border} />
          </marker>
        </defs>

        {EDGES.map((edge, i) => {
          const from = STATES[edge.from];
          const to = STATES[edge.to];
          const fromCx = from.x + NODE_W / 2;
          const fromCy = from.y + NODE_H / 2;
          const toCx = to.x + NODE_W / 2;
          const toCy = to.y + NODE_H / 2;

          const delay = 0.8 + i * 0.25;
          const progress = interpolate(
            frame,
            [delay * fps, (delay + 0.4) * fps],
            [0, 1],
            { extrapolateLeft: "clamp", extrapolateRight: "clamp" },
          );

          const len = Math.sqrt(
            (toCx - fromCx) ** 2 + (toCy - fromCy) ** 2,
          );

          return (
            <g key={i} opacity={progress}>
              <line
                x1={fromCx}
                y1={fromCy}
                x2={toCx}
                y2={toCy}
                stroke={COLORS.border}
                strokeWidth={2}
                strokeDasharray={len}
                strokeDashoffset={len * (1 - progress)}
                markerEnd="url(#ts-arrow)"
              />
              {edge.label && (
                <text
                  x={(fromCx + toCx) / 2}
                  y={(fromCy + toCy) / 2 - 6}
                  fill={COLORS.textMuted}
                  fontSize={11}
                  fontFamily={FONTS.mono}
                  textAnchor="middle"
                  opacity={progress > 0.6 ? (progress - 0.6) * 2.5 : 0}
                >
                  {edge.label}
                </text>
              )}
            </g>
          );
        })}
      </svg>

      {STATES.map((s, i) => {
        const delay = 0.3 + i * 0.2;
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
            key={s.label}
            style={{
              position: "absolute",
              left: 60 + s.x,
              top: 130 + s.y,
              width: NODE_W,
              height: NODE_H,
              opacity,
              transform: `scale(${scale})`,
              backgroundColor: COLORS.bgLight,
              border: `2px solid ${s.color}`,
              borderRadius: 10,
              display: "flex",
              justifyContent: "center",
              alignItems: "center",
              fontSize: 16,
              fontWeight: 700,
              color: s.color,
              fontFamily: FONTS.mono,
            }}
          >
            {s.label}
          </div>
        );
      })}
    </AbsoluteFill>
  );
};
