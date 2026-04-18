---
name: architecture-video
description: Generate or update the IronClaw architecture overview video using Remotion. Use when asked to update, regenerate, or modify the architecture video, add/remove scenes, or reflect codebase changes in the video.
---

# Architecture Video Generator

Generates and maintains the animated architecture overview video in `docs/architecture-video/` using Remotion (React-based video framework).

## When to use

- User asks to update, regenerate, or modify the architecture video
- User asks to add or remove scenes from the video
- Codebase architecture has changed and the video needs to reflect it
- User wants to preview or render the video

## Before making changes

### 1. Read current architecture

Read these files to understand the current system architecture:

- `CLAUDE.md` — top-level project structure, module specs, key traits, principles
- `crates/ironclaw_engine/CLAUDE.md` — engine v2 primitives, execution loop, CodeAct
- `src/agent/CLAUDE.md` — agent loop architecture
- `src/llm/CLAUDE.md` — LLM provider architecture
- `src/db/CLAUDE.md` — database dual-backend architecture
- `src/tools/README.md` — tool system architecture
- `src/workspace/README.md` — workspace/memory architecture

### 2. Read current video scenes

Read `docs/architecture-video/src/IronClawArchitecture.tsx` to understand current scene order, durations, and transitions. Then read individual scenes in `docs/architecture-video/src/scenes/` to see what's already covered.

### 3. Identify gaps

Compare the architecture documentation with what the video covers. Look for:
- New modules or traits added since the video was last updated
- Renamed or restructured components
- New data flows or state machines
- Removed or deprecated features

## Video project structure

```
docs/architecture-video/
├── package.json              # Remotion deps
├── remotion.config.ts        # Build config
├── src/
│   ├── Root.tsx              # Remotion entry — registers the composition
│   ├── IronClawArchitecture.tsx  # Main composition — scene order + transitions
│   ├── theme.ts              # Color palette + font constants
│   ├── components/
│   │   └── Code.tsx          # Syntax-highlighted code block component
│   └── scenes/               # One file per scene
│       ├── TitleScene.tsx
│       ├── PrimitivesScene.tsx
│       ├── ExecutionLoopScene.tsx
│       ├── CodeActScene.tsx
│       ├── ThreadStateScene.tsx
│       ├── SkillsPipelineScene.tsx
│       ├── ToolDispatchScene.tsx
│       ├── ChannelsRoutingScene.tsx
│       ├── ChannelImplsScene.tsx
│       ├── TraitsScene.tsx
│       ├── LlmDecoratorScene.tsx
│       └── OutroScene.tsx
```

Render script: `scripts/render-architecture-video.sh`

## Current scene inventory (12 scenes, ~82s at 30fps)

| # | Scene | File | Duration | Content |
|---|-------|------|----------|---------|
| 1 | Title | TitleScene.tsx | 4s | Animated IronClaw logo + tagline |
| 2 | Five Primitives | PrimitivesScene.tsx | 8s | Thread / Step / Capability / MemoryDoc / Project |
| 3 | Execution Loop | ExecutionLoopScene.tsx | 8s | 7-step ExecutionLoop::run() pipeline |
| 4 | CodeAct | CodeActScene.tsx | 10s | Python code → host fns → suspend/resume flow |
| 5 | Thread State | ThreadStateScene.tsx | 7s | Created→Running⇄Waiting/Suspended→Completed/Failed→Done |
| 6 | Skills Pipeline | SkillsPipelineScene.tsx | 8s | Gating → Scoring → Budget → Attenuation |
| 7 | Tool Dispatch | ToolDispatchScene.tsx | 9s | 9-step ToolDispatcher::dispatch() pipeline |
| 8 | Channels Routing | ChannelsRoutingScene.tsx | 7s | Channel trait + stream::select_all merging |
| 9 | Channel Impls | ChannelImplsScene.tsx | 7s | REPL / HTTP / Web / Signal / TUI / WASM |
| 10 | Traits | TraitsScene.tsx | 8s | 8 traits with concrete implementers |
| 11 | LLM Decorators | LlmDecoratorScene.tsx | 7s | SmartRouting→CircuitBreaker→...→Base decorator chain |
| 12 | Outro | OutroScene.tsx | 5s | Start Contributing + getting-started steps |

## Remotion patterns used in this project

All animations MUST be driven by `useCurrentFrame()` — never CSS transitions or Tailwind animation classes.

### Animation pattern

```tsx
const frame = useCurrentFrame();
const { fps } = useVideoConfig();

const opacity = interpolate(frame, [0, 0.5 * fps], [0, 1], {
  extrapolateRight: "clamp",
});
const y = interpolate(frame, [0, 0.5 * fps], [30, 0], {
  extrapolateRight: "clamp",
  easing: Easing.bezier(0.16, 1, 0.3, 1),
});
```

### Staggered list pattern

For items that appear one by one:

```tsx
{items.map((item, i) => {
  const delay = 0.4 + i * 0.3; // seconds
  const opacity = interpolate(
    frame,
    [delay * fps, (delay + 0.35) * fps],
    [0, 1],
    { extrapolateLeft: "clamp", extrapolateRight: "clamp" }
  );
  return <div style={{ opacity }} key={item.id}>...</div>;
})}
```

### Scene transitions

Scenes are composed using `TransitionSeries` with alternating `fade()` and `slide({ direction: "from-right" })` transitions, each 15 frames (0.5s):

```tsx
<TransitionSeries>
  <TransitionSeries.Sequence durationInFrames={s(8)}>
    <MyScene />
  </TransitionSeries.Sequence>
  <TransitionSeries.Transition
    presentation={fade()}
    timing={linearTiming({ durationInFrames: 15 })}
  />
  <TransitionSeries.Sequence durationInFrames={s(7)}>
    <NextScene />
  </TransitionSeries.Sequence>
</TransitionSeries>
```

### Code blocks

Use the `CodeBlock` component from `../components/Code` for syntax-highlighted code:

```tsx
import { CodeBlock } from "../components/Code";

<CodeBlock code={`pub trait Channel: Send + Sync {
  async fn start(&self) -> Result<MessageStream>;
}`} fontSize={13} />
```

### Theme

Import colors and fonts from `../theme`:

```tsx
import { COLORS, FONTS } from "../theme";

// Available colors:
// bg, bgLight, primary, primaryLight, accent, accentLight,
// success, danger, text, textMuted, border, purple, cyan, pink

// Available fonts:
// mono (monospace), sans (system-ui)
```

## Adding a new scene

1. Create `src/scenes/MyNewScene.tsx` following existing patterns
2. Export the component
3. Import in `IronClawArchitecture.tsx`
4. Add to the `SCENES` array with duration and transition type
5. `TOTAL_DURATION` auto-computes from the array
6. Verify with: `npx remotion still IronClawArchitecture --scale=0.25 --frame=<N>`

### Scene template

```tsx
import {
  AbsoluteFill,
  interpolate,
  useCurrentFrame,
  useVideoConfig,
  Easing,
} from "remotion";
import { COLORS, FONTS } from "../theme";

export const MyNewScene: React.FC = () => {
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
        <span style={{ color: COLORS.primary }}>Title</span> — subtitle
      </div>
      {/* Scene content */}
    </AbsoluteFill>
  );
};
```

## Verification

After making changes:

1. **Type check:** `cd docs/architecture-video && npx tsc --noEmit`
2. **Spot check frames:** `npx remotion still IronClawArchitecture --scale=0.25 --frame=<N>`
   - At 30fps, frame N corresponds to time N/30 seconds
   - Check at least one frame per modified scene
3. **Full render:** `./scripts/render-architecture-video.sh [output-path]`
4. **Preview in browser:** `cd docs/architecture-video && npm run dev`

## Design guidelines

- Dark theme (slate-900 background) — matches typical developer tooling
- Each scene has a colored heading keyword using a trait-appropriate color
- File:line references in muted monospace below headings
- Data flows use staggered animation (0.3-0.5s delays between items)
- State machines use SVG with animated dash-offset for arrows
- Code blocks use the `CodeBlock` component with syntax highlighting
- Keep scene duration proportional to content density (7-10s typical)
- Total video should stay under 120s for attention retention
