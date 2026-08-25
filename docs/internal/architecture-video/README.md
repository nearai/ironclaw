# IronClaw Architecture Overview Video

> **⚠️ STALE — describes the pre-Reborn architecture (April 2026).** The
> scenes cite `ironclaw_engine` and the v1 channel model, both deleted with
> the v1 monolith; nothing here reflects the current Reborn stack
> (`webui → ProductSurface → assistant → composition → runtime`, the turn
> runtime, or the family directory layout). For current architecture docs
> read `openwiki/`. ✎ 2026-08-21: the `architecture-video` Claude skill was
> removed (its scene walkthrough taught the deleted v1 architecture and the
> video was never regenerated); to bring the video up to date, rewrite the
> scenes in `src/scenes/` against the current Reborn docs and render with
> `scripts/render-architecture-video.sh` — do not cite this video's content
> until that happens. Content last updated in #2365 (2026-04-18); only
> dependencies and paths have changed since.

A Remotion-based animated video that walks new contributors through IronClaw's
internals — the five primitives, execution loop, CodeAct, thread state machine,
skills pipeline, tool dispatcher, channels, extensibility traits, and the LLM
provider decorator chain.

See the project-level render script for end-to-end use:

- `scripts/render-architecture-video.sh` — one-command MP4 render

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
