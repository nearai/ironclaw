import React from "react";

// The canonical NEAR "N" mark on a 0 0 24 24 viewBox, copied verbatim from
// onboarding/provider-logos.tsx so the process indicator uses the exact brand
// glyph. Filled brand-blue as `.near-base`, and reused as the clip that keeps
// the chasing comet inside the letterform.
const NEAR_GLYPH =
  "M21.443 0c-.89 0-1.714.46-2.18 1.218l-5.017 7.448a.533.533 0 0 0 .792.7l4.938-4.282a.2.2 0 0 1 .334.151v13.41a.2.2 0 0 1-.354.128L5.03.905A2.555 2.555 0 0 0 3.078 0h-.521A2.557 2.557 0 0 0 0 2.557v18.886a2.557 2.557 0 0 0 4.736 1.338l5.017-7.448a.533.533 0 0 0-.792-.7l-4.938 4.283a.2.2 0 0 1-.333-.152V5.352a.2.2 0 0 1 .354-.128l14.924 17.87c.486.574 1.2.905 1.952.906h.521A2.558 2.558 0 0 0 24 21.445V2.557A2.558 2.558 0 0 0 21.443 0Z";

// The comet rides the N's spine: down the left stroke, up the diagonal, up the
// right stroke. A thick round stroke clipped to the glyph fill reads as a light
// travelling the vector path. The geometry matches the approved design mockup.
const NEAR_SPINE = "M2.6 22.2V2.4L21.4 21.6V1.8";

type NearProcessIndicatorProps = {
  state: "working" | "done";
  label: string;
  elapsed?: string;
};

// Presentational live/working indicator: the NEAR mark with a light chasing its
// spine while the agent works, resolving to a solid brand-blue glyph when done.
// Left-aligned icon + label, no container box (see the "live status line" in the
// agent-activity mockup). Animation + `--near-blue` live in styles/app.css.
export function NearProcessIndicator({
  state,
  label,
  elapsed,
}: NearProcessIndicatorProps) {
  const clipId = React.useId();
  const working = state === "working";

  return (
    <div className={`near-process ${working ? "is-busy" : "is-done"}`}>
      <svg
        className="near-process-icon"
        viewBox="0 0 24 24"
        aria-hidden="true"
      >
        {working && (
          <defs>
            <clipPath id={clipId}>
              <path d={NEAR_GLYPH} />
            </clipPath>
          </defs>
        )}
        <path className="near-base" d={NEAR_GLYPH} />
        {working && (
          <g clipPath={`url(#${clipId})`}>
            <path className="near-comet" d={NEAR_SPINE} />
          </g>
        )}
      </svg>
      <span className="near-process-label">{label}</span>
      {working && elapsed ? (
        <span className="near-process-elapsed">{elapsed}</span>
      ) : null}
    </div>
  );
}
