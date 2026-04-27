import {
  AbsoluteFill,
  interpolate,
  useCurrentFrame,
  useVideoConfig,
  Easing,
} from "remotion";
import { COLORS, FONTS } from "../theme";

export const TitleScene: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const titleY = interpolate(frame, [0, 1 * fps], [60, 0], {
    extrapolateRight: "clamp",
    easing: Easing.bezier(0.16, 1, 0.3, 1),
  });

  const titleOpacity = interpolate(frame, [0, 0.6 * fps], [0, 1], {
    extrapolateRight: "clamp",
  });

  const subtitleOpacity = interpolate(
    frame,
    [0.5 * fps, 1.2 * fps],
    [0, 1],
    { extrapolateRight: "clamp" },
  );

  const subtitleY = interpolate(frame, [0.5 * fps, 1.2 * fps], [30, 0], {
    extrapolateRight: "clamp",
    easing: Easing.bezier(0.16, 1, 0.3, 1),
  });

  const lineWidth = interpolate(frame, [0.8 * fps, 1.6 * fps], [0, 400], {
    extrapolateRight: "clamp",
    easing: Easing.bezier(0.16, 1, 0.3, 1),
  });

  const taglineOpacity = interpolate(
    frame,
    [1.4 * fps, 2.0 * fps],
    [0, 1],
    { extrapolateRight: "clamp" },
  );

  // Animated grid background
  const gridOpacity = interpolate(frame, [0, 1 * fps], [0, 0.08], {
    extrapolateRight: "clamp",
  });

  return (
    <AbsoluteFill
      style={{
        backgroundColor: COLORS.bg,
        justifyContent: "center",
        alignItems: "center",
        fontFamily: FONTS.sans,
      }}
    >
      {/* Grid background */}
      <AbsoluteFill
        style={{
          opacity: gridOpacity,
          backgroundImage: `linear-gradient(${COLORS.primary} 1px, transparent 1px), linear-gradient(90deg, ${COLORS.primary} 1px, transparent 1px)`,
          backgroundSize: "60px 60px",
        }}
      />

      {/* Glow effect */}
      <div
        style={{
          position: "absolute",
          width: 600,
          height: 600,
          borderRadius: "50%",
          background: `radial-gradient(circle, ${COLORS.primary}20 0%, transparent 70%)`,
          top: "50%",
          left: "50%",
          transform: "translate(-50%, -50%)",
        }}
      />

      {/* Claw icon */}
      <div
        style={{
          opacity: titleOpacity,
          transform: `translateY(${titleY}px)`,
          fontSize: 64,
          marginBottom: 20,
        }}
      >
        🦀
      </div>

      {/* Title */}
      <div
        style={{
          opacity: titleOpacity,
          transform: `translateY(${titleY}px)`,
          fontSize: 80,
          fontWeight: 800,
          color: COLORS.text,
          letterSpacing: -2,
        }}
      >
        <span style={{ color: COLORS.primary }}>Iron</span>
        <span style={{ color: COLORS.accent }}>Claw</span>
      </div>

      {/* Divider line */}
      <div
        style={{
          width: lineWidth,
          height: 3,
          backgroundColor: COLORS.primary,
          marginTop: 16,
          marginBottom: 16,
          borderRadius: 2,
        }}
      />

      {/* Subtitle */}
      <div
        style={{
          opacity: subtitleOpacity,
          transform: `translateY(${subtitleY}px)`,
          fontSize: 32,
          fontWeight: 600,
          color: COLORS.textMuted,
          letterSpacing: 6,
          textTransform: "uppercase",
        }}
      >
        Architecture Overview
      </div>

      {/* Tagline */}
      <div
        style={{
          opacity: taglineOpacity,
          fontSize: 20,
          color: COLORS.textMuted,
          marginTop: 40,
          fontFamily: FONTS.mono,
        }}
      >
        Secure Personal AI Assistant &mdash; A Contributor&apos;s Guide
      </div>
    </AbsoluteFill>
  );
};
