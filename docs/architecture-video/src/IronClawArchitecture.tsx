import React from "react";
import { TransitionSeries, linearTiming } from "@remotion/transitions";
import { fade } from "@remotion/transitions/fade";
import { slide } from "@remotion/transitions/slide";

import { TitleScene } from "./scenes/TitleScene";
import { PrimitivesScene } from "./scenes/PrimitivesScene";
import { ExecutionLoopScene } from "./scenes/ExecutionLoopScene";
import { CodeActScene } from "./scenes/CodeActScene";
import { ThreadStateScene } from "./scenes/ThreadStateScene";
import { SkillsPipelineScene } from "./scenes/SkillsPipelineScene";
import { ToolDispatchScene } from "./scenes/ToolDispatchScene";
import { ChannelsRoutingScene } from "./scenes/ChannelsRoutingScene";
import { ChannelImplsScene } from "./scenes/ChannelImplsScene";
import { TraitsScene } from "./scenes/TraitsScene";
import { LlmDecoratorScene } from "./scenes/LlmDecoratorScene";
import { OutroScene } from "./scenes/OutroScene";

const s = (seconds: number) => Math.round(seconds * 30);

const SCENES = [
  { comp: TitleScene, dur: s(4), transition: fade },
  { comp: PrimitivesScene, dur: s(8), transition: slide },
  { comp: ExecutionLoopScene, dur: s(8), transition: fade },
  { comp: CodeActScene, dur: s(10), transition: slide },
  { comp: ThreadStateScene, dur: s(7), transition: fade },
  { comp: SkillsPipelineScene, dur: s(8), transition: slide },
  { comp: ToolDispatchScene, dur: s(9), transition: fade },
  { comp: ChannelsRoutingScene, dur: s(7), transition: slide },
  { comp: ChannelImplsScene, dur: s(7), transition: fade },
  { comp: TraitsScene, dur: s(8), transition: slide },
  { comp: LlmDecoratorScene, dur: s(7), transition: fade },
  { comp: OutroScene, dur: s(5) },
];

const TRANSITION_DUR = 15;

export const TOTAL_DURATION =
  SCENES.reduce((acc, sc) => acc + sc.dur, 0) -
  (SCENES.length - 1) * TRANSITION_DUR;

export const IronClawArchitecture: React.FC = () => {
  return (
    <TransitionSeries>
      {SCENES.map((sc, i) => {
        const Comp = sc.comp;
        const isLast = i === SCENES.length - 1;
        const presentation = sc.transition
          ? sc.transition === slide
            ? slide({ direction: "from-right" })
            : fade()
          : null;
        return (
          <React.Fragment key={i}>
            <TransitionSeries.Sequence durationInFrames={sc.dur}>
              <Comp />
            </TransitionSeries.Sequence>
            {!isLast && presentation && (
              <TransitionSeries.Transition
                presentation={presentation}
                timing={linearTiming({ durationInFrames: TRANSITION_DUR })}
              />
            )}
          </React.Fragment>
        );
      })}
    </TransitionSeries>
  );
};
