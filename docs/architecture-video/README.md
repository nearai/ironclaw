# IronClaw Architecture Overview Video

A Remotion-based animated video that walks new contributors through IronClaw's
internals — the five primitives, execution loop, CodeAct, thread state machine,
skills pipeline, tool dispatcher, channels, extensibility traits, and the LLM
provider decorator chain.

See the project-level render script and Claude skill for end-to-end use:

- `scripts/render-architecture-video.sh` — one-command MP4 render
- `.claude/skills/architecture-video/SKILL.md` — how to update scenes when
  architecture changes

## Commands

Install dependencies (first time only):

```console
npm ci
```

Preview in browser (Remotion Studio with hot reload):

```console
npm run dev
```

Render to MP4 from this directory:

```console
npx remotion render IronClawArchitecture out.mp4
```

Or from the repository root:

```console
./scripts/render-architecture-video.sh output.mp4
```

Type-check and lint:

```console
npm run lint
```

## Structure

- `src/IronClawArchitecture.tsx` — scene sequencing, durations, transitions
- `src/scenes/*.tsx` — one file per scene (12 total)
- `src/components/Code.tsx` — shared syntax-highlighted code block
- `src/theme.ts` — shared colors and fonts
- `src/Root.tsx` — Remotion composition registration

## License

This video project is part of IronClaw and dual-licensed MIT OR Apache-2.0.
Remotion itself has a [custom license](https://github.com/remotion-dev/remotion/blob/main/LICENSE.md);
use is covered under the open-source free tier for this project.
