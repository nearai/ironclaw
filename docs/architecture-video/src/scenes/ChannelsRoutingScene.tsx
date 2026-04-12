import {
  AbsoluteFill,
  interpolate,
  useCurrentFrame,
  useVideoConfig,
  Easing,
} from "remotion";
import { COLORS, FONTS } from "../theme";
import { CodeBlock } from "../components/Code";

const TRAIT_CODE = `pub trait Channel: Send + Sync {
  fn name(&self) -> &str;
  async fn start(&self) -> Result<MessageStream>;
  async fn respond(&self, msg: &IncomingMessage,
                   response: OutgoingResponse);
  async fn send_status(&self, s: StatusUpdate, meta: &Value);
  async fn broadcast(&self, user_id: &str,
                     response: OutgoingResponse);
  async fn health_check(&self);
  fn conversation_context(&self, meta: &Value)
       -> HashMap<String, String>;
  async fn shutdown(&self);
}`;

const MERGE_CODE = `// ChannelManager::start_all()
let streams: Vec<MessageStream> = channels
  .iter()
  .map(|c| c.start())
  .collect().await?;

// Merge N channels into 1 stream
stream::select_all(streams)`;

export const ChannelsRoutingScene: React.FC = () => {
  const frame = useCurrentFrame();
  const { fps } = useVideoConfig();

  const headingOpacity = interpolate(frame, [0, 0.4 * fps], [0, 1], {
    extrapolateRight: "clamp",
  });

  const leftOpacity = interpolate(frame, [0.5 * fps, 1.2 * fps], [0, 1], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });
  const leftX = interpolate(frame, [0.5 * fps, 1.2 * fps], [-30, 0], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
    easing: Easing.bezier(0.16, 1, 0.3, 1),
  });

  const rightOpacity = interpolate(frame, [1.5 * fps, 2.2 * fps], [0, 1], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
  });
  const rightX = interpolate(frame, [1.5 * fps, 2.2 * fps], [30, 0], {
    extrapolateLeft: "clamp",
    extrapolateRight: "clamp",
    easing: Easing.bezier(0.16, 1, 0.3, 1),
  });

  const routingOpacity = interpolate(
    frame,
    [2.8 * fps, 3.5 * fps],
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
        <span style={{ color: COLORS.cyan }}>Channels</span> &mdash; trait +
        stream merging
      </div>
      <div
        style={{
          opacity: headingOpacity,
          fontSize: 14,
          color: COLORS.textMuted,
          marginBottom: 20,
          fontFamily: FONTS.mono,
        }}
      >
        src/channels/channel.rs &bull; src/channels/manager.rs
      </div>

      <div
        style={{
          position: "absolute",
          left: 60,
          top: 150,
          width: 640,
          opacity: leftOpacity,
          transform: `translateX(${leftX}px)`,
        }}
      >
        <div
          style={{
            fontSize: 12,
            color: COLORS.textMuted,
            fontFamily: FONTS.mono,
            marginBottom: 6,
            textTransform: "uppercase",
            letterSpacing: 2,
          }}
        >
          Channel trait (8 methods)
        </div>
        <CodeBlock code={TRAIT_CODE} fontSize={13} />
      </div>

      <div
        style={{
          position: "absolute",
          right: 60,
          top: 150,
          width: 470,
          opacity: rightOpacity,
          transform: `translateX(${rightX}px)`,
        }}
      >
        <div
          style={{
            fontSize: 12,
            color: COLORS.textMuted,
            fontFamily: FONTS.mono,
            marginBottom: 6,
            textTransform: "uppercase",
            letterSpacing: 2,
          }}
        >
          stream::select_all merges N→1
        </div>
        <CodeBlock code={MERGE_CODE} fontSize={13} />

        <div
          style={{
            marginTop: 20,
            opacity: routingOpacity,
            backgroundColor: COLORS.bgLight,
            border: `1px solid ${COLORS.border}`,
            borderLeft: `4px solid ${COLORS.accent}`,
            borderRadius: 8,
            padding: "14px 18px",
          }}
        >
          <div
            style={{
              fontSize: 14,
              fontWeight: 700,
              color: COLORS.accent,
              marginBottom: 8,
            }}
          >
            routing_target_from_metadata()
          </div>
          <div
            style={{
              fontSize: 11,
              color: COLORS.textMuted,
              fontFamily: FONTS.mono,
              lineHeight: 1.7,
            }}
          >
            signal_target → chat_id → channel_id → target
          </div>
        </div>
      </div>
    </AbsoluteFill>
  );
};
